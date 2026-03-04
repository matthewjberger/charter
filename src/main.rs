use anyhow::Result;
use charter::cli::{Cli, Commands};
use charter::{detect, pipeline, serve};
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => {
            let root = detect::find_project_root(cli.path).await?;
            pipeline::capture(&root).await?;
        }
        Some(Commands::Serve { path, external }) => {
            let root = detect::find_project_root(path).await?;
            serve::serve(&root, external).await?;
        }
    }

    Ok(())
}
