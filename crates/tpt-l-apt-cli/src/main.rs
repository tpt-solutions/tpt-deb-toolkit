use clap::Parser;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = tpt_l_apt_cli::Cli::parse();
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(tpt_l_apt_cli::run(cli))?;
    Ok(())
}
