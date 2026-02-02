mod cache;
mod cli;
mod detect;
mod extract;
mod git;
mod output;
mod pipeline;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => {
            let root = detect::find_project_root(cli.path).await?;
            pipeline::capture(&root).await?;
        }
        Some(Commands::Read { tier, focus, path }) => {
            let root = detect::find_project_root(path).await?;
            output::peek(&root, tier, focus.as_deref()).await?;
        }
        Some(Commands::Status { path }) => {
            let root = detect::find_project_root(path).await?;
            output::stats(&root).await?;
        }
        Some(Commands::Lookup { symbol, path }) => {
            let root = detect::find_project_root(path).await?;
            output::lookup(&root, &symbol).await?;
        }
    }

    Ok(())
}
