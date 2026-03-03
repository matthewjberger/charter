use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "charter",
    about = "Structural codebase intelligence for LLMs, via MCP"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(help = "Project root (default: auto-detect from cwd)")]
    pub path: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Start MCP server over stdio")]
    Serve {
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
}
