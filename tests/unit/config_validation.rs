use std::fs;

use giftwrap::config;

#[test]
fn loads_valid_config() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let setup = tmp.path().join("setup.sh");
    fs::write(&setup, "#!/bin/sh\nexit 0\n").expect("write setup");

    let config_path = tmp.path().join(".giftwrap.toml");
    fs::write(
        &config_path,
        "image = \"docker.io/library/debian:bookworm-slim\"\nsetup_script = \"setup.sh\"\n",
    )
    .expect("write config");

    let cfg = config::load(&config_path).expect("config should load");
    assert_eq!(cfg.image, "docker.io/library/debian:bookworm-slim");
    assert_eq!(cfg.setup_script, std::path::PathBuf::from("setup.sh"));
    assert!(cfg.env.is_empty());
}

#[test]
fn loads_valid_config_with_env() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let setup = tmp.path().join("setup.sh");
    fs::write(&setup, "#!/bin/sh\nexit 0\n").expect("write setup");

    let config_path = tmp.path().join(".giftwrap.toml");
    fs::write(
        &config_path,
        "image = \"docker.io/library/debian:bookworm-slim\"\nsetup_script = \"setup.sh\"\n[env]\nPATH = \"/usr/local/bin:/usr/bin\"\nDEBIAN_FRONTEND = \"noninteractive\"\n",
    )
    .expect("write config");

    let cfg = config::load(&config_path).expect("config should load");
    assert_eq!(
        cfg.env.get("PATH"),
        Some(&"/usr/local/bin:/usr/bin".to_string())
    );
    assert_eq!(
        cfg.env.get("DEBIAN_FRONTEND"),
        Some(&"noninteractive".to_string())
    );
}

#[test]
fn rejects_unknown_keys() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let setup = tmp.path().join("setup.sh");
    fs::write(&setup, "#!/bin/sh\nexit 0\n").expect("write setup");

    let config_path = tmp.path().join(".giftwrap.toml");
    fs::write(
        &config_path,
        "image = \"alpine:3\"\nsetup_script = \"setup.sh\"\nworkdir = \"/tmp\"\n",
    )
    .expect("write config");

    let err = config::load(&config_path).expect_err("unknown key should fail");
    assert_eq!(err.to_string(), "invalid config key: workdir");
}

#[test]
fn rejects_missing_setup_script_path() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let config_path = tmp.path().join(".giftwrap.toml");
    fs::write(
        &config_path,
        "image = \"alpine:3\"\nsetup_script = \"missing.sh\"\n",
    )
    .expect("write config");

    let err = config::load(&config_path).expect_err("missing setup script should fail");
    assert!(err.to_string().contains("setup_script does not exist"));
}

#[test]
fn rejects_non_table_env() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let setup = tmp.path().join("setup.sh");
    fs::write(&setup, "#!/bin/sh\nexit 0\n").expect("write setup");

    let config_path = tmp.path().join(".giftwrap.toml");
    fs::write(
        &config_path,
        "image = \"alpine:3\"\nsetup_script = \"setup.sh\"\nenv = \"not-a-table\"\n",
    )
    .expect("write config");

    let err = config::load(&config_path).expect_err("env must be a table");
    assert!(err.to_string().contains("env must be a TOML table"));
}

#[test]
fn rejects_invalid_env_entries() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let setup = tmp.path().join("setup.sh");
    fs::write(&setup, "#!/bin/sh\nexit 0\n").expect("write setup");

    let config_path = tmp.path().join(".giftwrap.toml");
    fs::write(
        &config_path,
        "image = \"alpine:3\"\nsetup_script = \"setup.sh\"\n[env]\nPATH = 123\n",
    )
    .expect("write config");
    let err = config::load(&config_path).expect_err("env values must be strings");
    assert!(err
        .to_string()
        .contains("env value for PATH must be a string"));

    fs::write(
        &config_path,
        "image = \"alpine:3\"\nsetup_script = \"setup.sh\"\n[env]\nBAD-KEY = \"x\"\n",
    )
    .expect("write config");
    let err = config::load(&config_path).expect_err("env key must be shell-safe");
    assert!(err.to_string().contains("env key must match"));

    fs::write(
        &config_path,
        "image = \"alpine:3\"\nsetup_script = \"setup.sh\"\n[env]\nGW_CACHE_DIR = \"override\"\n",
    )
    .expect("write config");
    let err = config::load(&config_path).expect_err("GW_ keys are reserved");
    assert!(err.to_string().contains("env key is reserved for giftwrap"));
}
