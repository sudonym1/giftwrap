use std::fs;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn help_outputs_flags_with_minimal_config() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = dir.path().join(".giftwrap");
    fs::write(&config_path, "gw_container test-image\n").expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_giftwrap"))
        .arg("--gw-help")
        .current_dir(dir.path())
        .output()
        .expect("run giftwrap");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("GW Flags:"));
}
