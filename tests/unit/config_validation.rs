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
