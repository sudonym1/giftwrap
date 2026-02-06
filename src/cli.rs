use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::errors::GiftwrapError;
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
    pub print: bool,
    #[arg(long)]
    pub verbose: bool,
    #[arg(long)]
    pub cache_dir: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = PullPolicyArg::Missing)]
    pub pull: PullPolicyArg,
    #[arg(long)]
    pub setup_only: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

pub struct ParsedCli {
    pub cli: Cli,
    pub raw_args: Vec<String>,
}

pub fn parse() -> ParsedCli {
    let raw_args = std::env::args().collect::<Vec<_>>();
    let cli = Cli::parse();
    ParsedCli { cli, raw_args }
}

pub fn validate_run_invocation(
    run_args: &RunArgs,
    raw_args: &[String],
) -> Result<(), GiftwrapError> {
    if run_args.command.is_empty() || !run_has_delimiter(raw_args) {
        return Err(GiftwrapError::usage(
            "no command specified; use 'giftwrap run -- <command ...>'",
        ));
    }

    Ok(())
}

pub fn run_has_delimiter(raw_args: &[String]) -> bool {
    let mut saw_run = false;

    for arg in raw_args.iter().skip(1) {
        if !saw_run {
            if arg == "run" {
                saw_run = true;
            }
            continue;
        }

        if arg == "--" {
            return true;
        }
    }

    false
}
