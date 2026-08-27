//! DPLL SAT-based dependency resolver for Debian packages.
//!
//! # Architecture
//!
//! 1. Build a [`Universe`] from an `apt-cache` `Packages` index.
//! 2. Create a [`Resolver`] over that universe.
//! 3. Call [`Resolver::resolve`] with the names of packages to install.
//!
//! The solver encodes the dependency problem as propositional clauses and uses
//! the CDCL algorithm (unit propagation with watched literals, 1UIP conflict
//! analysis, non-chronological backtracking, and a VSIDS-style variable
//! ordering) to find a satisfying assignment.
//!
//! # Example
//!
//! ```
//! use tpt_l_apt_solver::{Package, Universe, Resolver};
//! use tpt_l_deb_version::Version;
//!
//! let mut u = Universe::new();
//! u.add_package(Package {
//!     name: "hello".to_string(),
//!     version: Version::parse("1.0").unwrap(),
//!     depends: vec![],
//!     pre_depends: vec![],
//!     conflicts: vec![],
//!     breaks: vec![],
//!     provides: vec![],
//!     recommends: vec![],
//!     suggests: vec![],
//! });
//!
//! let resolver = Resolver::new(u);
//! let plan = resolver.resolve(&["hello"]).unwrap();
//! assert_eq!(plan.install.len(), 1);
//! ```

use std::collections::{HashMap, VecDeque};

use rayon::prelude::*;
use thiserror::Error;
use tpt_l_deb_version::{Version, VersionConstraint};

// ─── Public data model ────────────────────────────────────────────────────────

/// A single (name, version) package with its dependency metadata.
#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub version: Version,
    /// AND of these dependency groups (each group is OR of alternatives).
    pub depends: Vec<DependencyGroup>,
    /// Same semantics as `depends` but must be satisfied before unpacking.
    pub pre_depends: Vec<DependencyGroup>,
    /// Packages that conflict with this one.
    pub conflicts: Vec<DependencyGroup>,
    /// Packages that this package breaks (softer than conflicts).
    pub breaks: Vec<DependencyGroup>,
    /// Virtual package names this package provides.
    pub provides: Vec<String>,
    /// Packages recommended by this one (installed automatically when possible,
    /// but their absence does not make the overall selection unsatisfiable).
    pub recommends: Vec<DependencyGroup>,
    /// Packages merely suggested by this one (informational only).
    pub suggests: Vec<DependencyGroup>,
}

/// An OR group of dependency alternatives (`a | b | c`).
#[derive(Debug, Clone)]
pub struct DependencyGroup {
    pub alternatives: Vec<DependencySpec>,
}

impl DependencyGroup {
    /// Parse a comma-separated list of dependency groups.
    ///
    /// Each group is pipe-separated alternatives, e.g.
    /// `"libc6 (>= 2.17) | libc6-amd64, libssl3"`.
    pub fn parse(s: &str) -> Result<Vec<DependencyGroup>, SolverError> {
        s.split(',')
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(|group_str| {
                let alternatives = group_str
                    .split('|')
                    .map(str::trim)
                    .filter(|a| !a.is_empty())
                    .map(DependencySpec::parse)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(DependencyGroup { alternatives })
            })
            .collect()
    }
}

/// A single dependency specification: `name [(op version)]`.
#[derive(Debug, Clone)]
pub struct DependencySpec {
    pub name: String,
    pub constraint: Option<VersionConstraint>,
}

impl DependencySpec {
    /// Parse `"name"` or `"name (op version)"`.
    pub fn parse(s: &str) -> Result<Self, SolverError> {
        let s = s.trim();
        if let Some(paren) = s.find('(') {
            let name = s[..paren].trim().to_string();
            let rest = s[paren + 1..].trim_end_matches(')').trim();
            let constraint = VersionConstraint::parse(rest)
                .map_err(|e| SolverError::ParseError(e.to_string()))?;
            Ok(Self {
                name,
                constraint: Some(constraint),
            })
        } else {
            // Strip arch qualifier like ":amd64"
            let name = s.find(':').map(|i| &s[..i]).unwrap_or(s).trim().to_string();
            Ok(Self {
                name,
                constraint: None,
            })
        }
    }
}

// ─── Universe ─────────────────────────────────────────────────────────────────

/// All available packages for a given architecture.
#[derive(Clone)]
pub struct Universe {
    packages: Vec<Package>,
    /// Real name → indices into `packages`.
    by_name: HashMap<String, Vec<usize>>,
    /// Virtual name → indices of providing packages.
    providers: HashMap<String, Vec<usize>>,
}

impl Universe {
    /// Create an empty universe.
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
            by_name: HashMap::new(),
            providers: HashMap::new(),
        }
    }

    /// Add a package to the universe.
    pub fn add_package(&mut self, pkg: Package) {
        let idx = self.packages.len();
        self.by_name.entry(pkg.name.clone()).or_default().push(idx);
        for virt in &pkg.provides {
            self.providers.entry(virt.clone()).or_default().push(idx);
        }
        self.packages.push(pkg);
    }

    /// Iterate over all packages with the given real name.
    pub fn packages_named(&self, name: &str) -> impl Iterator<Item = &Package> {
        self.by_name
            .get(name)
            .into_iter()
            .flat_map(|idxs| idxs.iter().map(|&i| &self.packages[i]))
    }

    /// Iterate over packages that provide `virtual_name`.
    pub fn providers_of(&self, virtual_name: &str) -> impl Iterator<Item = &Package> {
        self.providers
            .get(virtual_name)
            .into_iter()
            .flat_map(|idxs| idxs.iter().map(|&i| &self.packages[i]))
    }

    /// Build a `Universe` from a slice of binary package stanzas.
    ///
    /// Parsing is parallelised with Rayon.
    pub fn from_binary_packages(
        pkgs: &[tpt_l_control_file::BinaryPackage],
    ) -> Result<Universe, SolverError> {
        let parsed: Vec<Result<Package, SolverError>> =
            pkgs.par_iter().map(binary_package_to_pkg).collect();

        let mut universe = Universe::new();
        for result in parsed {
            universe.add_package(result?);
        }
        Ok(universe)
    }
}

impl Default for Universe {
    fn default() -> Self {
        Self::new()
    }
}

fn binary_package_to_pkg(bp: &tpt_l_control_file::BinaryPackage) -> Result<Package, SolverError> {
    let version = Version::parse(&bp.version_str)
        .map_err(|e| SolverError::ParseError(format!("{}: {}", bp.name, e)))?;

    let parse_deps = |s: &Option<String>| -> Result<Vec<DependencyGroup>, SolverError> {
        match s {
            None => Ok(vec![]),
            Some(raw) => DependencyGroup::parse(raw),
        }
    };

    let provides: Vec<String> = match &bp.provides {
        None => vec![],
        Some(raw) => raw
            .split(',')
            .map(|s| {
                let s = s.trim();
                s.find('(')
                    .map(|i| s[..i].trim().to_string())
                    .unwrap_or_else(|| s.to_string())
            })
            .filter(|s| !s.is_empty())
            .collect(),
    };

    Ok(Package {
        name: bp.name.clone(),
        version,
        depends: parse_deps(&bp.depends)?,
        pre_depends: parse_deps(&bp.pre_depends)?,
        conflicts: parse_deps(&bp.conflicts)?,
        breaks: parse_deps(&bp.breaks)?,
        provides,
        recommends: parse_deps(&bp.recommends)?,
        suggests: parse_deps(&bp.suggests)?,
    })
}

// ─── SAT encoding ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Literal {
    var: usize,
    negated: bool,
}

impl Literal {
    fn pos(var: usize) -> Self {
        Self {
            var,
            negated: false,
        }
    }
    fn neg(var: usize) -> Self {
        Self { var, negated: true }
    }
    fn negate(self) -> Self {
        Self {
            var: self.var,
            negated: !self.negated,
        }
    }
}

#[derive(Debug, Clone)]
struct Clause {
    literals: Vec<Literal>,
}

// ─── CDCL solver ──────────────────────────────────────────────────────────────

/// A SAT solver using conflict-driven clause learning (CDCL) with watched
/// literals, 1UIP conflict analysis, non-chronological backtracking, and a
/// VSIDS-style variable ordering with activity decay.
#[derive(Clone)]
struct Solver {
    num_vars: usize,
    clauses: Vec<Clause>,
    assignment: Vec<Option<bool>>,
    /// Clause that forced a variable's assignment, or `None` for decisions and
    /// root-level (unit) assignments.
    reason: Vec<Option<usize>>,
    /// Decision level at which each variable was assigned.
    level: Vec<usize>,
    /// Assignment stack (literals currently true), in assignment order.
    trail: Vec<Literal>,
    /// Indices into `trail` marking the start of each decision level.
    trail_lim: Vec<usize>,
    /// `watch[var]` → clause indices that mention `var` (any polarity).
    watch: Vec<Vec<usize>>,
    /// Queue of literals whose assignment may enable further implications.
    prop_queue: VecDeque<Literal>,
    /// VSIDS activity scores.
    activity: Vec<f64>,
    var_inc: f64,
    var_decay: f64,
    /// Diversifies the decision order across a parallel portfolio; seed 0 keeps
    /// the deterministic var-index tie-break.
    seed: u64,
}

impl Solver {
    fn new(num_vars: usize, clauses: Vec<Clause>) -> Self {
        let mut s = Self {
            num_vars,
            clauses: Vec::new(),
            assignment: vec![None; num_vars],
            reason: vec![None; num_vars],
            level: vec![0; num_vars],
            trail: Vec::new(),
            trail_lim: Vec::new(),
            watch: vec![Vec::new(); num_vars],
            prop_queue: VecDeque::new(),
            activity: vec![0.0; num_vars],
            var_inc: 1.0,
            var_decay: 0.95,
            seed: 0,
        };
        for clause in clauses {
            s.add_clause(clause.literals);
        }
        s
    }

    fn add_clause(&mut self, lits: Vec<Literal>) -> usize {
        let ci = self.clauses.len();
        self.clauses.push(Clause { literals: lits });
        for &l in &self.clauses[ci].literals {
            self.watch[l.var].push(ci);
        }
        ci
    }

    fn lit_value(&self, lit: Literal) -> Option<bool> {
        self.assignment[lit.var].map(|v| v != lit.negated)
    }

    /// `Some(true)` = satisfied, `Some(false)` = falsified, `None` = pending.
    fn clause_status(&self, ci: usize) -> Option<bool> {
        let mut all_false = true;
        for &lit in &self.clauses[ci].literals {
            match self.lit_value(lit) {
                Some(true) => return Some(true),
                Some(false) => {}
                None => all_false = false,
            }
        }
        if all_false {
            Some(false)
        } else {
            None
        }
    }

    fn assign(&mut self, lit: Literal, reason: Option<usize>) {
        self.assignment[lit.var] = Some(!lit.negated);
        self.reason[lit.var] = reason;
        self.level[lit.var] = self.trail_lim.len();
        self.trail.push(lit);
    }

    fn new_decision_level(&mut self) {
        self.trail_lim.push(self.trail.len());
    }

    fn cancel_until(&mut self, level: usize) {
        while self.trail_lim.len() > level {
            let start = self.trail_lim.pop().unwrap();
            while self.trail.len() > start {
                let lit = self.trail.pop().unwrap();
                self.assignment[lit.var] = None;
                self.reason[lit.var] = None;
                self.level[lit.var] = 0;
            }
        }
    }

    fn all_assigned(&self) -> bool {
        self.assignment.iter().all(|a| a.is_some())
    }

    /// Unit propagation with watched literals. Returns the conflicting clause
    /// index on a conflict, or `None` if propagation completed cleanly.
    fn propagate(&mut self) -> Option<usize> {
        while let Some(lit) = self.prop_queue.pop_front() {
            let neg = lit.negate();
            let var = lit.var;
            let ids: Vec<usize> = self.watch[var].clone();
            for ci in ids {
                if !self.clauses[ci].literals.contains(&neg) {
                    continue;
                }
                match self.clause_status(ci) {
                    Some(true) => continue,
                    Some(false) => return Some(ci),
                    None => {
                        let unassigned: Vec<Literal> = self.clauses[ci]
                            .literals
                            .iter()
                            .copied()
                            .filter(|&l| self.lit_value(l).is_none())
                            .collect();
                        if unassigned.len() == 1 {
                            let u = unassigned[0];
                            self.assign(u, Some(ci));
                            self.prop_queue.push_back(u);
                        }
                    }
                }
            }
        }
        None
    }

    /// Choose the unassigned variable with the highest VSIDS activity.
    ///
    /// Ties are broken deterministically by a seed-derived key so that a
    /// parallel portfolio (see [`Solver::solve_parallel`]) explores different
    /// decision orders per worker while `seed == 0` stays fully reproducible.
    fn pick_var(&self) -> Option<usize> {
        let mut best_var: Option<usize> = None;
        let mut best_act = f64::NEG_INFINITY;
        let mut best_key = 0u64;
        for v in 0..self.num_vars {
            if self.assignment[v].is_some() {
                continue;
            }
            let act = self.activity[v];
            let key = (v as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(self.seed);
            let better = match best_var {
                None => true,
                Some(_) => act > best_act || (act == best_act && key > best_key),
            };
            if better {
                best_var = Some(v);
                best_act = act;
                best_key = key;
            }
        }
        best_var
    }

    fn var_bump_activity(&mut self, v: usize) {
        self.activity[v] += self.var_inc;
        if self.activity[v] > 1e100 {
            for a in &mut self.activity {
                *a *= 1e-100;
            }
            self.var_inc *= 1e-100;
        }
    }

    fn var_decay_activity(&mut self) {
        self.var_inc /= self.var_decay;
    }

    /// 1UIP conflict analysis: produce a learnt asserting clause and the decision
    /// level to backjump to.
    fn analyze(&self, confl: usize) -> (Vec<Literal>, usize) {
        let dl = self.trail_lim.len();
        let mut seen = vec![false; self.num_vars];
        let mut out_learnt: Vec<Literal> = Vec::with_capacity(16);
        out_learnt.push(Literal::pos(0)); // placeholder for the asserting literal
        let mut path_c = 0usize;
        let mut index = self.trail.len();
        let mut c = confl;
        // Resolve the conflict back to its first unique implication point. The
        // loop returns the asserting literal `p` via `break 'resolve`.
        let p = 'resolve: loop {
            for &lit in &self.clauses[c].literals {
                let v = lit.var;
                if !seen[v] && self.level[v] > 0 {
                    seen[v] = true;
                    if self.level[v] >= dl {
                        path_c += 1;
                    } else {
                        out_learnt.push(lit.negate());
                    }
                }
            }
            loop {
                index -= 1;
                let tl = self.trail[index];
                if seen[tl.var] && self.level[tl.var] >= dl {
                    break;
                }
            }
            let cur = self.trail[index];
            seen[cur.var] = false;
            match self.reason[cur.var] {
                Some(r) => {
                    c = r;
                    path_c -= 1;
                    // Stop at the first UIP: `path_c` is the number of literals
                    // still pending above the current decision level.
                    if path_c == 0 {
                        break 'resolve cur;
                    }
                }
                None => break 'resolve cur,
            }
        };
        out_learnt[0] = p.negate();
        let mut btlevel = 0usize;
        for lit in &out_learnt[1..] {
            let lv = self.level[lit.var];
            if lv > btlevel {
                btlevel = lv;
            }
        }
        (out_learnt, btlevel)
    }

    /// Solve the formula. Returns `true` if satisfiable (a model is left in
    /// `assignment`); `false` means UNSAT.
    fn solve(&mut self) -> bool {
        self.search(&[])
    }

    /// Solve with a set of assumption literals forced true at the root level.
    ///
    /// Returns `false` immediately if any assumption contradicts the formula or
    /// another assumption. Used for greedy "keep installed" optimisation.
    fn solve_with_assumptions(&mut self, assumptions: &[Literal]) -> bool {
        self.search(assumptions)
    }

    /// Solve using a parallel portfolio.
    ///
    /// `threads` independent copies of the solver are launched (one per Rayon
    /// job); each gets a distinct decision-order [`seed`](Solver::seed) so the
    /// workers explore different search spaces. The first worker to find a
    /// satisfying assignment wins and its model is copied back into `self`; if
    /// every worker reports UNSAT the formula is UNSAT.
    ///
    /// With `threads <= 1` this falls back to the single-threaded [`search`].
    fn solve_parallel(&mut self, threads: usize) -> bool {
        if threads <= 1 {
            return self.solve();
        }
        let winner: Option<Solver> = (0..threads as u64)
            .into_par_iter()
            .map(|s| {
                let mut worker = self.clone();
                worker.seed = s.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
                let sat = worker.search(&[]);
                (sat, worker)
            })
            .find_any(|(sat, _)| *sat)
            .map(|(_, w)| w);
        match winner {
            Some(w) => {
                *self = w;
                true
            }
            None => false,
        }
    }

    fn search(&mut self, assumptions: &[Literal]) -> bool {
        // Seed the queue with any unit clauses at the root level.
        let mut units = Vec::new();
        for (ci, clause) in self.clauses.iter().enumerate() {
            if clause.literals.len() == 1 {
                let l = clause.literals[0];
                if self.lit_value(l).is_none() {
                    units.push((l, ci));
                }
            }
        }
        for (l, ci) in units {
            self.assign(l, Some(ci));
            self.prop_queue.push_back(l);
        }

        // Force assumption literals at decision level 0.
        for &a in assumptions {
            match self.lit_value(a) {
                Some(true) => {}
                Some(false) => return false,
                None => {
                    self.assign(a, None);
                    self.prop_queue.push_back(a);
                }
            }
        }

        loop {
            if let Some(confl) = self.propagate() {
                // A conflict with no decisions on the stack is a genuine
                // root-level contradiction → the formula is unsatisfiable.
                if self.trail_lim.is_empty() {
                    return false;
                }
                let (learnt, btlevel) = self.analyze(confl);
                for &l in &learnt {
                    self.var_bump_activity(l.var);
                }
                self.cancel_until(btlevel);
                let asserting = learnt[0];
                let ci = self.add_clause(learnt);
                self.assign(asserting, Some(ci));
                self.prop_queue.push_back(asserting);
            } else if self.all_assigned() {
                return true;
            } else {
                self.var_decay_activity();
                let v = match self.pick_var() {
                    Some(v) => v,
                    None => return self.all_assigned(),
                };
                self.new_decision_level();
                let lit = Literal::pos(v);
                self.assign(lit, None);
                self.prop_queue.push_back(lit);
            }
        }
    }
}

// ─── Install plan ─────────────────────────────────────────────────────────────

/// The result of a dependency resolution.
#[derive(Debug)]
pub struct InstallPlan {
    /// Packages to install (not currently installed, or newly-upgraded versions).
    pub install: Vec<(String, Version)>,
    /// Packages to remove (currently installed versions that are dropped or
    /// replaced by an upgrade). Empty when no installed state was supplied.
    pub remove: Vec<(String, Version)>,
    /// Packages recommended by an installed package and pulled in automatically.
    pub recommended: Vec<(String, Version)>,
    /// Packages merely suggested by an installed package (informational).
    pub suggested: Vec<(String, Version)>,
}

// ─── Solver errors ────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SolverError {
    #[error("no solution: {0}")]
    NoSolution(String),
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("unknown package: {0}")]
    UnknownPackage(String),
}

// ─── Resolver ─────────────────────────────────────────────────────────────────

/// Dependency resolver operating over a [`Universe`].
pub struct Resolver {
    universe: Universe,
}

impl Resolver {
    pub fn new(universe: Universe) -> Self {
        Self { universe }
    }

    /// Resolve the given package names into an [`InstallPlan`].
    ///
    /// This is equivalent to [`Resolver::resolve_with_installed`] with an empty
    /// installed set (so `remove` is always empty).
    pub fn resolve(&self, requests: &[&str]) -> Result<InstallPlan, SolverError> {
        self.resolve_with_installed(requests, &[])
    }

    /// Resolve `requests` against the currently-installed packages.
    ///
    /// Installed versions are added to the universe as additional candidates so
    /// that the solver can model upgrades and removals. The resulting
    /// [`InstallPlan`] reports:
    /// * `install` — packages not previously installed (or newly-upgraded versions),
    /// * `remove` — installed versions that are dropped or replaced by an upgrade,
    /// * `recommended` / `suggested` — advisory packages pulled in from the
    ///   `Recommends` / `Suggests` relations of selected packages.
    pub fn resolve_with_installed(
        &self,
        requests: &[&str],
        installed: &[(String, Version)],
    ) -> Result<InstallPlan, SolverError> {
        for &name in requests {
            let known = self.universe.packages_named(name).count() > 0
                || self.universe.providers_of(name).count() > 0;
            if !known {
                return Err(SolverError::UnknownPackage(name.to_string()));
            }
        }

        // Build a working universe that also contains the currently-installed
        // package versions as additional candidates.
        let mut work = self.universe.clone();
        let installed_vars: Vec<usize> = installed
            .iter()
            .map(|(name, version)| {
                let idx = work.packages.len();
                work.packages.push(Package {
                    name: name.clone(),
                    version: version.clone(),
                    depends: vec![],
                    pre_depends: vec![],
                    conflicts: vec![],
                    breaks: vec![],
                    provides: vec![],
                    recommends: vec![],
                    suggests: vec![],
                });
                work.by_name.entry(name.clone()).or_default().push(idx);
                idx
            })
            .collect();

        let n = work.packages.len();
        if n == 0 {
            return Ok(InstallPlan {
                install: vec![],
                remove: vec![],
                recommended: vec![],
                suggested: vec![],
            });
        }

        let clauses = encode_clauses(&work, requests);

        // Bias each requested name toward its highest available version. This
        // makes `install A` upgrade an older installed `A` rather than keeping
        // it, matching apt's behaviour. Lower versions are tried as a fallback
        // only when the newest cannot be satisfied.
        let mut solver = Solver::new(n, clauses.clone());
        let mut pinned: Vec<usize> = Vec::new();
        for &name in requests {
            let mut cands: Vec<usize> = work.by_name.get(name).cloned().unwrap_or_default();
            cands.sort_by(|&a, &b| work.packages[b].version.cmp(&work.packages[a].version));
            for c in cands {
                let mut assumptions: Vec<Literal> =
                    pinned.iter().map(|&v| Literal::pos(v)).collect();
                assumptions.push(Literal::pos(c));
                let mut s = Solver::new(n, clauses.clone());
                if s.solve_with_assumptions(&assumptions) {
                    pinned.push(c);
                    solver = s;
                    break;
                }
            }
        }

        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        if !solver.solve_parallel(threads) {
            return Err(SolverError::NoSolution(
                "no consistent package selection satisfies all constraints".to_string(),
            ));
        }

        // Greedily retain as many installed packages as possible. Each installed
        // candidate not already chosen is re-tried with the current keep-set
        // forced as assumptions; if the relaxed problem is still satisfiable we
        // adopt it. This minimises spurious removals without an optimiser.
        let mut kept: Vec<usize> = (0..n)
            .filter(|&i| solver.assignment[i] == Some(true))
            .collect();
        for &k in &installed_vars {
            if kept.contains(&k) {
                continue;
            }
            let assumptions: Vec<Literal> = kept
                .iter()
                .map(|&v| Literal::pos(v))
                .chain(std::iter::once(Literal::pos(k)))
                .collect();
            let mut s = Solver::new(n, clauses.clone());
            if s.solve_with_assumptions(&assumptions) {
                kept = (0..n).filter(|&i| s.assignment[i] == Some(true)).collect();
                solver = s;
            }
        }

        // Pull in recommended packages (auto-install when the relaxed problem
        // stays satisfiable). Iterates so a recommend can itself satisfy another.
        let mut recommended_set: Vec<(String, Version)> = Vec::new();
        loop {
            let mut added = false;
            let current: Vec<usize> = (0..n)
                .filter(|&i| solver.assignment[i] == Some(true))
                .collect();
            for &i in &current {
                let mut stop = false;
                for group in &work.packages[i].recommends {
                    if let Some(j) = best_satisfying(&work, group) {
                        if solver.assignment[j] == Some(true) {
                            // Already selected: record it so minimisation keeps it.
                            recommended_set.push((
                                work.packages[j].name.clone(),
                                work.packages[j].version.clone(),
                            ));
                            continue;
                        }
                        let assumptions: Vec<Literal> = current
                            .iter()
                            .map(|&v| Literal::pos(v))
                            .chain(std::iter::once(Literal::pos(j)))
                            .collect();
                        let mut s = Solver::new(n, clauses.clone());
                        if s.solve_with_assumptions(&assumptions) {
                            recommended_set.push((
                                work.packages[j].name.clone(),
                                work.packages[j].version.clone(),
                            ));
                            solver = s;
                            added = true;
                            stop = true;
                            break;
                        }
                    }
                }
                if stop {
                    break;
                }
            }
            if !added {
                break;
            }
        }

        // Minimise: drop any selected package that is not a request target, not
        // an installed candidate, and not a pulled-in recommendation — unless it
        // is transitively required to keep the solution satisfiable.
        let request_names: std::collections::HashSet<&str> = requests.iter().copied().collect();
        let is_essential = |i: usize| -> bool {
            request_names.contains(work.packages[i].name.as_str())
                || installed_vars.contains(&i)
                || recommended_set
                    .iter()
                    .any(|(nm, v)| nm == &work.packages[i].name && v == &work.packages[i].version)
        };
        let mut model: Vec<usize> = (0..n)
            .filter(|&i| solver.assignment[i] == Some(true))
            .collect();
        for &v in &model.clone() {
            if is_essential(v) {
                continue;
            }
            let assumptions: Vec<Literal> = model
                .iter()
                .filter(|&&w| w != v)
                .map(|&w| Literal::pos(w))
                .chain(std::iter::once(Literal::neg(v)))
                .collect();
            let mut s = Solver::new(n, clauses.clone());
            if s.solve_with_assumptions(&assumptions) {
                solver = s;
                model = (0..n)
                    .filter(|&i| solver.assignment[i] == Some(true))
                    .collect();
            }
        }

        let selected: Vec<(String, Version)> = model
            .iter()
            .map(|&i| {
                (
                    work.packages[i].name.clone(),
                    work.packages[i].version.clone(),
                )
            })
            .collect();

        let install = selected
            .iter()
            .filter(|sel| !installed.iter().any(|(n, v)| n == &sel.0 && v == &sel.1))
            .cloned()
            .collect();
        let remove = installed
            .iter()
            .filter(|ins| !selected.iter().any(|(n, v)| n == &ins.0 && v == &ins.1))
            .cloned()
            .collect();

        let (recommended, suggested) = compute_advisories(&work, &solver.assignment);

        Ok(InstallPlan {
            install,
            remove,
            recommended,
            suggested,
        })
    }
}

/// Best (highest-version) real package index satisfying `group`, if any.
fn best_satisfying(u: &Universe, group: &DependencyGroup) -> Option<usize> {
    let mut best: Option<(usize, Version)> = None;
    for alt in &group.alternatives {
        for j in matching_package_indices(u, alt) {
            let ver = u.packages[j].version.clone();
            let take = match best {
                Some((_, ref b)) => ver < *b,
                None => true,
            };
            if take {
                best = Some((j, ver));
            }
        }
    }
    best.map(|(j, _)| j)
}

/// Encode the dependency problem for `u` as a set of CNF clauses.
///
/// Variables are package indices; literal `i` means "package `i` is selected".
fn encode_clauses(u: &Universe, requests: &[&str]) -> Vec<Clause> {
    let mut clauses: Vec<Clause> = Vec::new();

    // Clause 1: at least one version (or provider) of each request.
    for &name in requests {
        let mut lits: Vec<Literal> = u
            .by_name
            .get(name)
            .into_iter()
            .flat_map(|v| v.iter())
            .map(|&i| Literal::pos(i))
            .collect();
        if let Some(providers) = u.providers.get(name) {
            for &pi in providers {
                if !lits.iter().any(|l| l.var == pi) {
                    lits.push(Literal::pos(pi));
                }
            }
        }
        clauses.push(Clause { literals: lits });
    }

    // Clause 2: at-most-one version per package name.
    for indices in u.by_name.values() {
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                clauses.push(Clause {
                    literals: vec![Literal::neg(indices[i]), Literal::neg(indices[j])],
                });
            }
        }
    }

    // Clause 3: dependency implications.
    for (i, pkg) in u.packages.iter().enumerate() {
        for dep_groups in [&pkg.depends, &pkg.pre_depends] {
            for group in dep_groups {
                let sat_lits = satisfying_literals(u, group);
                if sat_lits.is_empty() {
                    // Unsatisfiable dep: package i cannot be installed.
                    clauses.push(Clause {
                        literals: vec![Literal::neg(i)],
                    });
                } else {
                    let mut lits = vec![Literal::neg(i)];
                    lits.extend(sat_lits);
                    clauses.push(Clause { literals: lits });
                }
            }
        }

        // Clause 4: conflicts and breaks.
        for dep_groups in [&pkg.conflicts, &pkg.breaks] {
            for group in dep_groups {
                for alt in &group.alternatives {
                    for j in matching_package_indices(u, alt) {
                        if i == j {
                            continue;
                        }
                        clauses.push(Clause {
                            literals: vec![Literal::neg(i), Literal::neg(j)],
                        });
                    }
                }
            }
        }
    }

    clauses
}

/// Variables (package indices) that satisfy a dependency group, as OR-literals.
fn satisfying_literals(u: &Universe, group: &DependencyGroup) -> Vec<Literal> {
    let mut lits = Vec::new();
    for alt in &group.alternatives {
        for j in matching_package_indices(u, alt) {
            if !lits.iter().any(|l: &Literal| l.var == j) {
                lits.push(Literal::pos(j));
            }
        }
    }
    lits
}

/// Package indices that match a dependency spec (real name or virtual provider).
fn matching_package_indices(u: &Universe, spec: &DependencySpec) -> Vec<usize> {
    let mut out = Vec::new();
    if let Some(indices) = u.by_name.get(&spec.name) {
        for &i in indices {
            let pkg = &u.packages[i];
            if spec
                .constraint
                .as_ref()
                .is_none_or(|c| c.satisfies(&pkg.version))
            {
                out.push(i);
            }
        }
    }
    if let Some(indices) = u.providers.get(&spec.name) {
        for &i in indices {
            if !out.contains(&i) {
                out.push(i);
            }
        }
    }
    out
}

/// Resolve the `Recommends` / `Suggests` relations of the selected packages to
/// concrete, best-version real packages.
#[allow(clippy::type_complexity)]
fn compute_advisories(
    u: &Universe,
    assignment: &[Option<bool>],
) -> (Vec<(String, Version)>, Vec<(String, Version)>) {
    let mut recommended = Vec::new();
    let mut suggested = Vec::new();

    let resolve_group =
        |u: &Universe, group: &DependencyGroup, out: &mut Vec<(String, Version)>| {
            // Prefer the highest-version real package satisfying the group.
            let mut best: Option<(usize, &Package)> = None;
            for alt in &group.alternatives {
                for j in matching_package_indices(u, alt) {
                    let pkg = &u.packages[j];
                    match best {
                        Some((_, b)) if b.version >= pkg.version => {}
                        _ => best = Some((j, pkg)),
                    }
                }
            }
            if let Some((_, pkg)) = best {
                if !out.iter().any(|(n, v)| n == &pkg.name && v == &pkg.version) {
                    out.push((pkg.name.clone(), pkg.version.clone()));
                }
            }
        };

    for (i, pkg) in u.packages.iter().enumerate() {
        if assignment[i] != Some(true) {
            continue;
        }
        for group in &pkg.recommends {
            resolve_group(u, group, &mut recommended);
        }
        for group in &pkg.suggests {
            resolve_group(u, group, &mut suggested);
        }
    }

    (recommended, suggested)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, version: &str) -> Package {
        Package {
            name: name.to_string(),
            version: Version::parse(version).unwrap(),
            depends: vec![],
            pre_depends: vec![],
            conflicts: vec![],
            breaks: vec![],
            provides: vec![],
            recommends: vec![],
            suggests: vec![],
        }
    }

    #[test]
    fn resolve_single_no_deps() {
        let mut u = Universe::new();
        u.add_package(pkg("hello", "1.0"));
        let plan = Resolver::new(u).resolve(&["hello"]).unwrap();
        assert_eq!(plan.install.len(), 1);
        assert_eq!(plan.install[0].0, "hello");
    }

    #[test]
    fn unknown_package_error() {
        let result = Resolver::new(Universe::new()).resolve(&["nonexistent"]);
        assert!(matches!(result, Err(SolverError::UnknownPackage(_))));
    }

    #[test]
    fn dependency_chain_a_b_c() {
        let mut u = Universe::new();
        u.add_package(Package {
            name: "A".to_string(),
            version: Version::parse("1.0").unwrap(),
            depends: DependencyGroup::parse("B").unwrap(),
            pre_depends: vec![],
            conflicts: vec![],
            breaks: vec![],
            provides: vec![],
            recommends: vec![],
            suggests: vec![],
        });
        u.add_package(Package {
            name: "B".to_string(),
            version: Version::parse("1.0").unwrap(),
            depends: DependencyGroup::parse("C").unwrap(),
            pre_depends: vec![],
            conflicts: vec![],
            breaks: vec![],
            provides: vec![],
            recommends: vec![],
            suggests: vec![],
        });
        u.add_package(pkg("C", "1.0"));

        let plan = Resolver::new(u).resolve(&["A"]).unwrap();
        let names: Vec<&str> = plan.install.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"A"), "plan={:?}", names);
        assert!(names.contains(&"B"), "plan={:?}", names);
        assert!(names.contains(&"C"), "plan={:?}", names);
    }

    #[test]
    fn conflicting_packages_no_solution() {
        let mut u = Universe::new();
        u.add_package(Package {
            name: "A".to_string(),
            version: Version::parse("1.0").unwrap(),
            depends: vec![],
            pre_depends: vec![],
            conflicts: DependencyGroup::parse("B").unwrap(),
            breaks: vec![],
            provides: vec![],
            recommends: vec![],
            suggests: vec![],
        });
        u.add_package(pkg("B", "1.0"));

        let result = Resolver::new(u).resolve(&["A", "B"]);
        assert!(matches!(result, Err(SolverError::NoSolution(_))));
    }

    #[test]
    fn virtual_package_satisfied_by_provider() {
        let mut u = Universe::new();
        u.add_package(Package {
            name: "A".to_string(),
            version: Version::parse("1.0").unwrap(),
            depends: vec![],
            pre_depends: vec![],
            conflicts: vec![],
            breaks: vec![],
            provides: vec!["X".to_string()],
            recommends: vec![],
            suggests: vec![],
        });
        u.add_package(Package {
            name: "B".to_string(),
            version: Version::parse("1.0").unwrap(),
            depends: DependencyGroup::parse("X").unwrap(),
            pre_depends: vec![],
            conflicts: vec![],
            breaks: vec![],
            provides: vec![],
            recommends: vec![],
            suggests: vec![],
        });

        let plan = Resolver::new(u).resolve(&["B"]).unwrap();
        let names: Vec<&str> = plan.install.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            names.contains(&"A"),
            "A (provider of X) missing: {:?}",
            names
        );
        assert!(names.contains(&"B"), "B missing: {:?}", names);
    }

    #[test]
    fn dependency_group_parse_alternatives() {
        let groups = DependencyGroup::parse("libc6 (>= 2.17) | libc6-amd64, libssl3").unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].alternatives.len(), 2);
        assert_eq!(groups[0].alternatives[0].name, "libc6");
        assert_eq!(groups[1].alternatives[0].name, "libssl3");
    }

    #[test]
    fn dep_spec_parse_no_constraint() {
        let spec = DependencySpec::parse("bash").unwrap();
        assert_eq!(spec.name, "bash");
        assert!(spec.constraint.is_none());
    }

    // ── CDCL correctness ────────────────────────────────────────────────────────

    /// Exhaustively check whether `clauses` over `n` variables is satisfiable.
    fn brute_force_sat(n: usize, clauses: &[Clause]) -> bool {
        let total = 1usize << n;
        for mask in 0..total {
            let mut ok = true;
            for c in clauses {
                let mut clause_ok = false;
                for l in &c.literals {
                    let bit = ((mask >> l.var) & 1) == 1;
                    let val = if l.negated { !bit } else { bit };
                    if val {
                        clause_ok = true;
                        break;
                    }
                }
                if !clause_ok {
                    ok = false;
                    break;
                }
            }
            if ok {
                return true;
            }
        }
        false
    }

    /// Tiny deterministic LCG so the randomized test needs no external RNG.
    fn lcg(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state >> 8
    }

    #[test]
    fn cdcl_matches_bruteforce_random() {
        let mut state: u64 = 0x1234_5678_9abc_def0;
        for trial in 0..300 {
            let n = 1 + (lcg(&mut state) as usize % 12); // 1..12 vars
            let nc = 4 + (lcg(&mut state) as usize % 24); // 4..27 clauses
            let mut clauses = Vec::new();
            for _ in 0..nc {
                let len = 1 + (lcg(&mut state) as usize % 3); // 1..3 literals
                let mut lits = Vec::new();
                for _ in 0..len {
                    let v = (lcg(&mut state) as usize) % n;
                    let neg = (lcg(&mut state) & 1) == 1;
                    lits.push(Literal {
                        var: v,
                        negated: neg,
                    });
                }
                clauses.push(Clause { literals: lits });
            }
            let mut solver = Solver::new(n, clauses.clone());
            let sat = solver.solve();
            let bf = brute_force_sat(n, &clauses);
            assert_eq!(sat, bf, "satisfiability mismatch on trial {trial} (n={n})");
            if sat {
                for c in &clauses {
                    let mut ok = false;
                    for l in &c.literals {
                        if solver.assignment[l.var] == Some(!l.negated) {
                            ok = true;
                            break;
                        }
                    }
                    assert!(ok, "model fails a clause on trial {trial}");
                }
            }
            if sat {
                for c in &clauses {
                    let mut ok = false;
                    for l in &c.literals {
                        if solver.assignment[l.var] == Some(!l.negated) {
                            ok = true;
                            break;
                        }
                    }
                    assert!(ok, "model fails a clause on trial {trial}");
                }
            }
        }
    }

    #[test]
    fn cdcl_unsat_unit_contradiction() {
        let clauses = vec![
            Clause {
                literals: vec![Literal::pos(0)],
            },
            Clause {
                literals: vec![Literal::neg(0)],
            },
        ];
        let mut solver = Solver::new(1, clauses);
        assert!(!solver.solve());
    }

    #[test]
    fn cdcl_unsat_requires_backtracking() {
        // (a) ∧ (¬a ∨ b) ∧ (¬b) — forces a conflict that must be learned.
        let clauses = vec![
            Clause {
                literals: vec![Literal::pos(0)],
            },
            Clause {
                literals: vec![Literal::neg(0), Literal::pos(1)],
            },
            Clause {
                literals: vec![Literal::neg(1)],
            },
        ];
        let mut solver = Solver::new(2, clauses);
        assert!(!solver.solve());
    }

    #[test]
    fn cdcl_sat_finds_model() {
        // (a ∨ b) ∧ (¬a ∨ ¬b) — satisfiable (e.g. a=true, b=false).
        let clauses = vec![
            Clause {
                literals: vec![Literal::pos(0), Literal::pos(1)],
            },
            Clause {
                literals: vec![Literal::neg(0), Literal::neg(1)],
            },
        ];
        let mut solver = Solver::new(2, clauses);
        assert!(solver.solve());
    }

    #[test]
    fn cdcl_parallel_matches_single() {
        // The parallel portfolio must agree with the single-threaded solver on
        // both satisfiability and model validity.
        let mut state: u64 = 0x0bad_c0de_1234_5678;
        for trial in 0..120 {
            let n = 1 + (lcg(&mut state) as usize % 14);
            let nc = 4 + (lcg(&mut state) as usize % 28);
            let mut clauses = Vec::new();
            for _ in 0..nc {
                let len = 1 + (lcg(&mut state) as usize % 3);
                let mut lits = Vec::new();
                for _ in 0..len {
                    let v = (lcg(&mut state) as usize) % n;
                    let neg = (lcg(&mut state) & 1) == 1;
                    lits.push(Literal {
                        var: v,
                        negated: neg,
                    });
                }
                clauses.push(Clause { literals: lits });
            }
            let sat_single = {
                let mut s = Solver::new(n, clauses.clone());
                s.solve()
            };
            let sat_par = {
                let mut s = Solver::new(n, clauses.clone());
                s.solve_parallel(4)
            };
            assert_eq!(
                sat_single, sat_par,
                "parallel/serial SAT mismatch on trial {trial} (n={n})"
            );
            if sat_par {
                let mut s = Solver::new(n, clauses.clone());
                assert!(s.solve_parallel(4));
                for c in &clauses {
                    let ok = c
                        .literals
                        .iter()
                        .any(|l| s.assignment[l.var] == Some(!l.negated));
                    assert!(ok, "parallel model fails clause on trial {trial}");
                }
            }
        }
    }

    // ── Installed-state modelling ──────────────────────────────────────────────

    fn pkg_with(name: &str, version: &str, recommends: &str, suggests: &str) -> Package {
        Package {
            name: name.to_string(),
            version: Version::parse(version).unwrap(),
            depends: vec![],
            pre_depends: vec![],
            conflicts: vec![],
            breaks: vec![],
            provides: vec![],
            recommends: DependencyGroup::parse(recommends).unwrap(),
            suggests: DependencyGroup::parse(suggests).unwrap(),
        }
    }

    #[test]
    fn recommends_pulled_in() {
        let mut u = Universe::new();
        u.add_package(pkg_with(
            "A", "1.0", "B", // recommends B
            "",
        ));
        u.add_package(pkg("B", "1.0"));

        let plan = Resolver::new(u).resolve(&["A"]).unwrap();
        let names: Vec<&str> = plan.install.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"A"));
        assert!(names.contains(&"B"), "recommended B missing: {:?}", names);
        let rec_names: Vec<&str> = plan.recommended.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            rec_names.contains(&"B"),
            "recommended list: {:?}",
            rec_names
        );
    }

    #[test]
    fn installed_packages_kept_by_default() {
        let mut u = Universe::new();
        u.add_package(pkg("A", "1.0"));
        u.add_package(pkg("B", "1.0"));

        // A already installed; asking to install nothing new beyond A keeps it.
        let plan = Resolver::new(u)
            .resolve_with_installed(&["A"], &[("A".to_string(), Version::parse("1.0").unwrap())])
            .unwrap();
        assert!(
            plan.remove.is_empty(),
            "spurious removal: {:?}",
            plan.remove
        );
        assert!(
            plan.install.is_empty(),
            "spurious install: {:?}",
            plan.install
        );
    }

    #[test]
    fn installed_package_removed_on_upgrade() {
        let mut u = Universe::new();
        u.add_package(pkg("A", "1.0"));
        u.add_package(pkg("A", "2.0"));

        let plan = Resolver::new(u)
            .resolve_with_installed(&["A"], &[("A".to_string(), Version::parse("1.0").unwrap())])
            .unwrap();
        assert!(
            plan.install
                .contains(&("A".to_string(), Version::parse("2.0").unwrap())),
            "plan={:?}",
            plan.install
        );
        assert!(
            plan.remove
                .contains(&("A".to_string(), Version::parse("1.0").unwrap())),
            "plan={:?}",
            plan.remove
        );
    }

    #[test]
    fn installed_package_removed_when_conflicting() {
        let mut u = Universe::new();
        u.add_package(Package {
            name: "A".to_string(),
            version: Version::parse("1.0").unwrap(),
            depends: vec![],
            pre_depends: vec![],
            conflicts: DependencyGroup::parse("old").unwrap(),
            breaks: vec![],
            provides: vec![],
            recommends: vec![],
            suggests: vec![],
        });
        u.add_package(pkg("old", "1.0"));
        u.add_package(pkg("new", "1.0"));

        // `old` is installed; installing `A` (which conflicts with `old`)
        // must remove `old`.
        let plan = Resolver::new(u)
            .resolve_with_installed(
                &["A", "new"],
                &[("old".to_string(), Version::parse("1.0").unwrap())],
            )
            .unwrap();
        assert!(
            plan.remove
                .contains(&("old".to_string(), Version::parse("1.0").unwrap())),
            "plan={:?}",
            plan.remove
        );
    }
}
