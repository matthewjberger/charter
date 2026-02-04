mod cache;
mod cli;
mod deps;
mod detect;
mod extract;
mod git;
mod output;
mod pipeline;
mod query;
mod session;
mod tests;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, SessionAction};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => {
            let root = detect::find_project_root(cli.path).await?;
            pipeline::capture(&root).await?;
        }
        Some(Commands::Read {
            tier,
            focus,
            since,
            path,
        }) => {
            let root = detect::find_project_root(path).await?;
            output::peek(&root, tier, focus.as_deref(), since.as_deref()).await?;
        }
        Some(Commands::Status { path }) => {
            let root = detect::find_project_root(path).await?;
            output::stats(&root).await?;
        }
        Some(Commands::Lookup { symbol, path }) => {
            let root = detect::find_project_root(path).await?;
            output::lookup(&root, &symbol).await?;
        }
        Some(Commands::Query {
            query: query_str,
            limit,
            path,
        }) => {
            let root = detect::find_project_root(path).await?;
            query::query(&root, &query_str, limit).await?;
        }
        Some(Commands::Deps { krate, path }) => {
            let root = detect::find_project_root(path).await?;
            deps::deps(&root, krate.as_deref()).await?;
        }
        Some(Commands::Tests { file, path }) => {
            let root = detect::find_project_root(path).await?;
            tests::tests(&root, file.as_deref()).await?;
        }
        Some(Commands::Session { action }) => match action {
            SessionAction::Start { path } => {
                let root = detect::find_project_root(path).await?;
                session::start_session(&root).await?;
            }
            SessionAction::End { path } => {
                let root = detect::find_project_root(path).await?;
                session::end_session(&root).await?;
            }
            SessionAction::Status { path } => {
                let root = detect::find_project_root(path).await?;
                session::session_status(&root).await?;
            }
        },
    }

    Ok(())
}
