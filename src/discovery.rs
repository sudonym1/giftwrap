use std::path::{Path, PathBuf};

use crate::errors::GiftwrapError;

pub const CONFIG_DIRNAME: &str = ".giftwrap";
pub const CONFIG_FILENAME: &str = "config.toml";
pub const CONFIG_REL_PATH: &str = ".giftwrap/config.toml";

#[derive(Debug, Clone)]
pub struct DiscoveredConfig {
    pub build_root: PathBuf,
    pub config_path: PathBuf,
}

pub fn discover(start: &Path) -> Result<DiscoveredConfig, GiftwrapError> {
    let mut current = start.canonicalize().map_err(|err| {
        GiftwrapError::config_hint(
            format!("failed to resolve current directory: {err}"),
            "run giftwrap from inside your project tree",
        )
    })?;

    loop {
        let candidate = current.join(CONFIG_DIRNAME).join(CONFIG_FILENAME);
        if candidate.is_file() {
            return Ok(DiscoveredConfig {
                build_root: current,
                config_path: candidate,
            });
        }

        if !current.pop() {
            return Err(GiftwrapError::config(format!(
                "could not find {CONFIG_REL_PATH} in current directory or any parent"
            )));
        }
    }
}
