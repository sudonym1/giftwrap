use std::path::Path;

use serde_json::Value;

use crate::errors::GiftwrapError;
use crate::log::Logger;
use crate::process::{self, CommandFailureKind};

pub fn inspect_digest(image: &str, logger: &Logger) -> Result<String, GiftwrapError> {
    let args = inspect_command_args(image);
    let output = process::run_capture(
        "skopeo",
        &args,
        CommandFailureKind::Build,
        "inspect image digest",
        logger,
    )?;

    let value: Value = serde_json::from_slice(&output.stdout).map_err(|err| {
        GiftwrapError::build(format!("failed to parse skopeo inspect output: {err}"))
    })?;

    value
        .get("Digest")
        .and_then(Value::as_str)
        .map(|digest| digest.to_string())
        .ok_or_else(|| {
            GiftwrapError::build("failed to inspect image digest (missing Digest field)")
        })
}

pub fn pull_to_layout(
    image: &str,
    layout_dir: &Path,
    logger: &Logger,
) -> Result<(), GiftwrapError> {
    let args = pull_command_args(image, layout_dir);
    process::run_checked(
        "skopeo",
        &args,
        CommandFailureKind::Build,
        "pull OCI image",
        logger,
    )
}

pub fn pull_command_args(image: &str, layout_dir: &Path) -> Vec<String> {
    vec![
        "copy".to_string(),
        format!("docker://{image}"),
        format!("oci:{}:base", layout_dir.display()),
    ]
}

pub fn inspect_command_args(image: &str) -> Vec<String> {
    vec!["inspect".to_string(), format!("docker://{image}")]
}
