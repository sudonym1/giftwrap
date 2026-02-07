use std::fs;

use giftwrap::discovery;

#[test]
fn discovers_config_upward() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("project");
    let nested = project.join("a/b/c");
    fs::create_dir_all(&nested).expect("create nested dirs");
    fs::create_dir_all(project.join(".giftwrap")).expect("create config dir");

    let config_path = project.join(".giftwrap/config.toml");
    fs::write(
        &config_path,
        "image = \"alpine\"\nsetup_script = \"setup.sh\"\n",
    )
    .expect("write config");

    let discovered = discovery::discover(&nested).expect("should discover config");
    assert_eq!(
        discovered.build_root,
        project.canonicalize().expect("canonical")
    );
    assert_eq!(
        discovered.config_path,
        config_path.canonicalize().expect("canonical")
    );
}

#[test]
fn fails_when_config_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let err = discovery::discover(tmp.path()).expect_err("missing config should fail");
    assert!(err
        .to_string()
        .contains("could not find .giftwrap/config.toml in current directory or any parent"));
}

#[test]
fn does_not_fallback_to_legacy_config_location() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = tmp.path().join("project");
    let nested = project.join("a/b/c");
    fs::create_dir_all(&nested).expect("create nested dirs");

    let legacy_config = project.join(".giftwrap.toml");
    fs::write(
        &legacy_config,
        "image = \"alpine\"\nsetup_script = \"setup.sh\"\n",
    )
    .expect("write legacy config");

    let err = discovery::discover(&nested).expect_err("legacy path should not be discovered");
    assert!(err
        .to_string()
        .contains("could not find .giftwrap/config.toml in current directory or any parent"));
}
