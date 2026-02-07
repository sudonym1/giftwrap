use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::sqfs_cache::PullPolicy;

#[derive(Debug, Parser)]
#[command(
    name = "giftwrap",
    disable_version_flag = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Run(RunArgs),
    PrintConfig,
    Cache(CacheArgs),
    Version,
}

#[derive(Debug, Args)]
pub struct CacheArgs {
    #[command(subcommand)]
    pub command: CacheCommands,
}

#[derive(Debug, Subcommand)]
pub enum CacheCommands {
    Gc(CacheGcArgs),
}

#[derive(Debug, Args)]
pub struct CacheGcArgs {
    #[arg(long)]
    pub print: bool,
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
    #[arg(long)]
    pub max_age_days: Option<u64>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PullPolicyArg {
    Missing,
    Always,
    Never,
}

impl PullPolicyArg {
    pub fn as_pull_policy(self) -> PullPolicy {
        match self {
            Self::Missing => PullPolicy::Missing,
            Self::Always => PullPolicy::Always,
            Self::Never => PullPolicy::Never,
        }
    }
}

#[derive(Debug, Args)]
pub struct RunArgs {
    #[arg(long)]
    pub rebuild: bool,
    #[arg(long)]
    pub reset_overlay: bool,
    #[arg(long)]
    pub print: bool,
    #[arg(long)]
    pub verbose: bool,
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = PullPolicyArg::Missing)]
    pub pull: PullPolicyArg,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

pub fn parse() -> Cli {
    Cli::parse()
}
