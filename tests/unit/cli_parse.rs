use clap::Parser;

use giftwrap::cli::{self, CacheCommands, Cli, Commands, PullPolicyArg};

#[test]
fn parses_run_with_all_flags() {
    let cli = Cli::try_parse_from([
        "giftwrap",
        "run",
        "--rebuild",
        "--print",
        "--verbose",
        "--cache-dir",
        "/tmp/cache",
        "--pull",
        "always",
        "--setup-only",
        "--",
        "echo",
        "hello",
    ])
    .expect("cli parse should succeed");

    let Commands::Run(run) = cli.command else {
        panic!("expected run command");
    };

    assert!(run.rebuild);
    assert!(run.print);
    assert!(run.verbose);
    assert!(run.setup_only);
    assert_eq!(
        run.cache_dir.as_deref(),
        Some(std::path::Path::new("/tmp/cache"))
    );
    assert!(matches!(run.pull, PullPolicyArg::Always));
    assert_eq!(run.command, vec!["echo", "hello"]);
}

#[test]
fn rejects_missing_command() {
    let cli =
        Cli::try_parse_from(["giftwrap", "run", "--setup-only"]).expect("cli parse should succeed");

    let Commands::Run(run) = cli.command else {
        panic!("expected run command");
    };

    let raw = vec![
        "giftwrap".to_string(),
        "run".to_string(),
        "--setup-only".to_string(),
    ];
    let err = cli::validate_run_invocation(&run, &raw).expect_err("missing command should fail");
    assert_eq!(
        err.to_string(),
        "no command specified; use 'giftwrap run -- <command ...>'"
    );
}

#[test]
fn rejects_missing_double_dash() {
    let cli = Cli::try_parse_from(["giftwrap", "run", "echo", "hello"])
        .expect("cli parse should succeed");

    let Commands::Run(run) = cli.command else {
        panic!("expected run command");
    };

    let raw = vec![
        "giftwrap".to_string(),
        "run".to_string(),
        "echo".to_string(),
        "hello".to_string(),
    ];
    let err = cli::validate_run_invocation(&run, &raw)
        .expect_err("missing command delimiter should fail");
    assert_eq!(
        err.to_string(),
        "no command specified; use 'giftwrap run -- <command ...>'"
    );
}

#[test]
fn parses_cache_gc_subcommand() {
    let cli = Cli::try_parse_from(["giftwrap", "cache", "gc", "--print", "--max-age-days", "30"])
        .expect("cli parse should succeed");

    let Commands::Cache(cache_args) = cli.command else {
        panic!("expected cache command");
    };

    let CacheCommands::Gc(gc) = cache_args.command;
    assert!(gc.print);
    assert_eq!(gc.max_age_days, Some(30));
}
