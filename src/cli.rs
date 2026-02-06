use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "charter",
    about = "Fast structural context generator for Rust codebases"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(help = "Project root (default: auto-detect from cwd)")]
    pub path: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Dump context to stdout for piping into an LLM session")]
    Read {
        #[arg(
            default_value = "default",
            help = "Context tier (quick, default, full)"
        )]
        tier: Tier,
        #[arg(long, short, help = "Focus on a specific module path (e.g., src/ecs/)")]
        focus: Option<String>,
        #[arg(long, help = "Show changes since git ref (e.g., HEAD~5, main, abc123)")]
        since: Option<String>,
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
    #[command(about = "Quick summary: crates, files, lines, last capture info")]
    Status {
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
    #[command(about = "Look up a single symbol across all charter data")]
    Lookup {
        #[arg(help = "Symbol name to look up")]
        symbol: String,
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
    #[command(about = "Search for symbols, types, and functions")]
    Query {
        #[arg(help = "Search query (e.g., 'callers of Cache::save', 'what handles errors')")]
        query: String,
        #[arg(long, short, default_value = "20", help = "Maximum number of results")]
        limit: usize,
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
    #[command(about = "Analyze external dependency usage")]
    Deps {
        #[arg(long, help = "Filter to a specific crate")]
        krate: Option<String>,
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
    #[command(about = "Map tests to source files")]
    Tests {
        #[arg(long, short, help = "Show tests for a specific file")]
        file: Option<String>,
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
    #[command(about = "Manage session state")]
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    #[command(about = "Start MCP server over stdio")]
    Serve {
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum SessionAction {
    #[command(about = "Start a new session")]
    Start {
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
    #[command(about = "End the current session")]
    End {
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
    #[command(about = "Show session status")]
    Status {
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum Tier {
    Quick,
    #[default]
    Default,
    Full,
}
