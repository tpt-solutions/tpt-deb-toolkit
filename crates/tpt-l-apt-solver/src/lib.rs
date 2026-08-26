//! DPLL SAT-based dependency resolver for Debian packages.
//!
//! # Architecture
//!
//! 1. Build a [`Universe`] from an `apt-cache` `Packages` index.
//! 2. Create a [`Resolver`] over that universe.
//! 3. Call [`Resolver::resolve`] with the names of packages to install.
//!
//! The solver encodes the dependency problem as propositional clauses and uses
//! the classic DPLL algorithm (unit propagation + backtracking) to find a
//! satisfying assignment.
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
//! });
//!
//! let resolver = Resolver::new(u);
//! let plan = resolver.resolve(&["hello"]).unwrap();
//! assert_eq!(plan.install.len(), 1);
//! ```

use std::collections::HashMap;

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
}

#[derive(Debug, Clone)]
struct Clause {
    literals: Vec<Literal>,
}

// ─── DPLL solver ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Assign {
    Unset,
    True,
    False,
}

struct Solver {
    num_vars: usize,
    clauses: Vec<Clause>,
    assignment: Vec<Assign>,
    /// `watch[var]` → clause indices that mention `var` (occurrence lists).
    watch: Vec<Vec<usize>>,
}

impl Solver {
    fn new(num_vars: usize, clauses: Vec<Clause>) -> Self {
        let mut watch = vec![Vec::new(); num_vars];
        for (ci, clause) in clauses.iter().enumerate() {
            for lit in &clause.literals {
                watch[lit.var].push(ci);
            }
        }
        Self {
            num_vars,
            clauses,
            assignment: vec![Assign::Unset; num_vars],
            watch,
        }
    }

    fn lit_value(&self, lit: Literal) -> Option<bool> {
        match self.assignment[lit.var] {
            Assign::Unset => None,
            Assign::True => Some(!lit.negated),
            Assign::False => Some(lit.negated),
        }
    }

    /// `Some(true)` = satisfied, `Some(false)` = falsified, `None` = pending.
    fn clause_status(&self, clause: &Clause) -> Option<bool> {
        let mut all_false = true;
        for &lit in &clause.literals {
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

    /// Unit propagation. Returns `false` on conflict.
    fn unit_propagate(&mut self) -> bool {
        loop {
            let mut changed = false;
            for ci in 0..self.clauses.len() {
                match self.clause_status(&self.clauses[ci]) {
                    Some(true) => continue,
                    Some(false) => return false,
                    None => {
                        let unset: Vec<Literal> = self.clauses[ci]
                            .literals
                            .iter()
                            .copied()
                            .filter(|&l| self.lit_value(l).is_none())
                            .collect();
                        if unset.len() == 1 {
                            let forced = unset[0];
                            self.assignment[forced.var] = if forced.negated {
                                Assign::False
                            } else {
                                Assign::True
                            };
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        !self
            .clauses
            .iter()
            .any(|c| self.clause_status(c) == Some(false))
    }

    /// Choose the unset variable with the highest occurrence count (VSIDS-like).
    fn choose_var(&self) -> Option<usize> {
        (0..self.num_vars)
            .filter(|&v| self.assignment[v] == Assign::Unset)
            .max_by_key(|&v| {
                self.watch[v]
                    .iter()
                    .filter(|&&ci| self.clause_status(&self.clauses[ci]).is_none())
                    .count()
            })
    }

    /// DPLL recursive search. Returns `true` iff satisfiable.
    fn dpll(&mut self) -> bool {
        if !self.unit_propagate() {
            return false;
        }
        if self
            .clauses
            .iter()
            .all(|c| self.clause_status(c) == Some(true))
        {
            return true;
        }
        let var = match self.choose_var() {
            None => {
                return !self
                    .clauses
                    .iter()
                    .any(|c| self.clause_status(c) == Some(false));
            }
            Some(v) => v,
        };
        let saved = self.assignment.clone();
        self.assignment[var] = Assign::True;
        if self.dpll() {
            return true;
        }
        self.assignment = saved;
        self.assignment[var] = Assign::False;
        if self.dpll() {
            return true;
        }
        self.assignment[var] = Assign::Unset;
        false
    }
}

// ─── Install plan ─────────────────────────────────────────────────────────────

/// The result of a dependency resolution.
#[derive(Debug)]
pub struct InstallPlan {
    /// Packages to install.
    pub install: Vec<(String, Version)>,
    /// Packages to remove (always empty — solver does not model installed state).
    pub remove: Vec<(String, Version)>,
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
    pub fn resolve(&self, requests: &[&str]) -> Result<InstallPlan, SolverError> {
        for &name in requests {
            let known = self.universe.packages_named(name).count() > 0
                || self.universe.providers_of(name).count() > 0;
            if !known {
                return Err(SolverError::UnknownPackage(name.to_string()));
            }
        }

        let n = self.universe.packages.len();
        if n == 0 {
            return Ok(InstallPlan {
                install: vec![],
                remove: vec![],
            });
        }

        let mut clauses: Vec<Clause> = Vec::new();

        // Clause 1: at least one version (or provider) of each request.
        for &name in requests {
            let mut lits: Vec<Literal> = self
                .universe
                .by_name
                .get(name)
                .into_iter()
                .flat_map(|v| v.iter())
                .map(|&i| Literal::pos(i))
                .collect();
            for &pi in self
                .universe
                .providers
                .get(name)
                .into_iter()
                .flat_map(|v| v.iter())
            {
                if !lits.iter().any(|l| l.var == pi) {
                    lits.push(Literal::pos(pi));
                }
            }
            clauses.push(Clause { literals: lits });
        }

        // Clause 2: at-most-one version per package name.
        for indices in self.universe.by_name.values() {
            for i in 0..indices.len() {
                for j in (i + 1)..indices.len() {
                    clauses.push(Clause {
                        literals: vec![Literal::neg(indices[i]), Literal::neg(indices[j])],
                    });
                }
            }
        }

        // Clause 3: dependency implications.
        for (i, pkg) in self.universe.packages.iter().enumerate() {
            for dep_groups in [&pkg.depends, &pkg.pre_depends] {
                for group in dep_groups {
                    let sat_lits = self.satisfying_literals(group);
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
                        for j in self.matching_package_indices(alt) {
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

        let mut solver = Solver::new(n, clauses);
        if !solver.dpll() {
            return Err(SolverError::NoSolution(
                "no consistent package selection satisfies all constraints".to_string(),
            ));
        }

        let install = self
            .universe
            .packages
            .iter()
            .enumerate()
            .filter(|(i, _)| solver.assignment[*i] == Assign::True)
            .map(|(_, pkg)| (pkg.name.clone(), pkg.version.clone()))
            .collect();

        Ok(InstallPlan {
            install,
            remove: vec![],
        })
    }

    fn satisfying_literals(&self, group: &DependencyGroup) -> Vec<Literal> {
        let mut lits = Vec::new();
        for alt in &group.alternatives {
            for j in self.matching_package_indices(alt) {
                if !lits.iter().any(|l: &Literal| l.var == j) {
                    lits.push(Literal::pos(j));
                }
            }
        }
        lits
    }

    fn matching_package_indices(&self, spec: &DependencySpec) -> Vec<usize> {
        let mut out = Vec::new();
        if let Some(indices) = self.universe.by_name.get(&spec.name) {
            for &i in indices {
                let pkg = &self.universe.packages[i];
                if spec
                    .constraint
                    .as_ref()
                    .is_none_or(|c| c.satisfies(&pkg.version))
                {
                    out.push(i);
                }
            }
        }
        if let Some(indices) = self.universe.providers.get(&spec.name) {
            for &i in indices {
                if !out.contains(&i) {
                    out.push(i);
                }
            }
        }
        out
    }
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
        });
        u.add_package(Package {
            name: "B".to_string(),
            version: Version::parse("1.0").unwrap(),
            depends: DependencyGroup::parse("C").unwrap(),
            pre_depends: vec![],
            conflicts: vec![],
            breaks: vec![],
            provides: vec![],
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
        });
        u.add_package(Package {
            name: "B".to_string(),
            version: Version::parse("1.0").unwrap(),
            depends: DependencyGroup::parse("X").unwrap(),
            pre_depends: vec![],
            conflicts: vec![],
            breaks: vec![],
            provides: vec![],
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
}
