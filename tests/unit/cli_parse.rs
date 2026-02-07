use clap::Parser;

use giftwrap::cli::{CacheCommands, Cli, Commands, PullPolicyArg};

#[test]
fn parses_run_with_all_flags() {
    let cli = Cli::try_parse_from([
        "giftwrap",
        "run",
        "--rebuild",
        "--reset",
        "--print",
        "--verbose",
        "--cache-dir",
        "/tmp/cache",
        "--pull",
        "always",
        "echo",
        "hello",
    ])
    .expect("cli parse should succeed");

    let Commands::Run(run) = cli.command else {
        panic!("expected run command");
    };

    assert!(run.rebuild);
    assert!(run.reset);
    assert!(run.print);
    assert!(run.verbose);
    assert_eq!(
        run.cache_dir.as_deref(),
        Some(std::path::Path::new("/tmp/cache"))
    );
    assert!(matches!(run.pull, PullPolicyArg::Always));
    assert_eq!(run.command, vec!["echo", "hello"]);
}

#[test]
fn accepts_missing_command() {
    let cli = Cli::try_parse_from(["giftwrap", "run"]).expect("cli parse should succeed");

    let Commands::Run(run) = cli.command else {
        panic!("expected run command");
    };

    assert!(run.command.is_empty());
}

#[test]
fn still_parses_with_double_dash_delimiter() {
    let cli = Cli::try_parse_from(["giftwrap", "run", "--", "echo", "hello"])
        .expect("cli parse should succeed");

    let Commands::Run(run) = cli.command else {
        panic!("expected run command");
    };

    assert_eq!(run.command, vec!["echo", "hello"]);
}

#[test]
fn parses_cache_gc_subcommand() {
    let cli = Cli::try_parse_from(["giftwrap", "cache", "gc", "--print", "--max-age-days", "30"])
        .expect("cli parse should succeed");

    let Commands::Cache(cache_args) = cli.command else {
        panic!("expected cache command");
    };

    let CacheCommands::Gc(gc) = cache_args.command else {
        panic!("expected cache gc subcommand");
    };
    assert!(gc.print);
    assert_eq!(gc.max_age_days, Some(30));
}

#[test]
fn accepts_legacy_reset_overlay_alias() {
    let cli = Cli::try_parse_from(["giftwrap", "run", "--reset-overlay"])
        .expect("cli parse should succeed");

    let Commands::Run(run) = cli.command else {
        panic!("expected run command");
    };

    assert!(run.reset);
}

#[test]
fn parses_cache_reset_subcommand() {
    let cli =
        Cli::try_parse_from(["giftwrap", "cache", "reset"]).expect("cli parse should succeed");

    let Commands::Cache(cache_args) = cli.command else {
        panic!("expected cache command");
    };

    assert!(matches!(cache_args.command, CacheCommands::Reset));
}

#[test]
fn parses_cache_gc_with_cache_dir() {
    let cli = Cli::try_parse_from([
        "giftwrap",
        "cache",
        "--cache-dir",
        "/tmp/cache",
        "gc",
        "--print",
    ])
    .expect("cli parse should succeed");

    let Commands::Cache(cache_args) = cli.command else {
        panic!("expected cache command");
    };

    assert_eq!(
        cache_args.cache_dir.as_deref(),
        Some(std::path::Path::new("/tmp/cache"))
    );
    let CacheCommands::Gc(gc) = cache_args.command else {
        panic!("expected cache gc subcommand");
    };
    assert!(gc.print);
}
