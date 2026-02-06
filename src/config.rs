use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::errors::GiftwrapError;

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub image: String,
    pub setup_script: PathBuf,
}

impl Config {
    pub fn resolve_setup_script(&self, build_root: &Path) -> PathBuf {
        if self.setup_script.is_absolute() {
            self.setup_script.clone()
        } else {
            build_root.join(&self.setup_script)
        }
    }
}

pub fn load(path: &Path) -> Result<Config, GiftwrapError> {
    let text = fs::read_to_string(path).map_err(|err| {
        GiftwrapError::config_hint(
            format!("failed to read config {}: {err}", path.display()),
            "ensure .giftwrap.toml exists and is readable",
        )
    })?;

    let value: toml::Value = toml::from_str(&text)
        .map_err(|err| GiftwrapError::config(format!("failed to parse config: {err}")))?;

    let table = value
        .as_table()
        .ok_or_else(|| GiftwrapError::config("config root must be a TOML table"))?;

    for key in table.keys() {
        if key != "image" && key != "setup_script" {
            return Err(GiftwrapError::config(format!("invalid config key: {key}")));
        }
    }

    let image = table
        .get("image")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| GiftwrapError::config("missing required key: image"))?
        .trim()
        .to_string();

    if image.is_empty() {
        return Err(GiftwrapError::config("image must be non-empty"));
    }

    let setup_script = table
        .get("setup_script")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| GiftwrapError::config("missing required key: setup_script"))?
        .trim()
        .to_string();

    if setup_script.is_empty() {
        return Err(GiftwrapError::config("setup_script must be non-empty"));
    }

    let config = Config {
        image,
        setup_script: PathBuf::from(setup_script),
    };

    let build_root = path.parent().ok_or_else(|| {
        GiftwrapError::config(format!(
            "could not resolve build root from {}",
            path.display()
        ))
    })?;

    let setup_path = config.resolve_setup_script(build_root);
    if !setup_path.exists() {
        return Err(GiftwrapError::config_hint(
            format!("setup_script does not exist: {}", setup_path.display()),
            "set setup_script to a valid relative or absolute path",
        ));
    }

    Ok(config)
}
