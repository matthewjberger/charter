use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "atlas",
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
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
    #[command(about = "Quick summary: crates, files, lines, last capture info")]
    Status {
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
    #[command(about = "Add atlas instructions to CLAUDE.md for post-compaction recovery")]
    Inject {
        #[arg(help = "Project root (default: auto-detect from cwd)")]
        path: Option<PathBuf>,
    },
    #[command(about = "Look up a single symbol across all atlas data")]
    Lookup {
        #[arg(help = "Symbol name to look up")]
        symbol: String,
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
