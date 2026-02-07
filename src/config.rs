use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::discovery::CONFIG_REL_PATH;
use crate::errors::GiftwrapError;

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub image: String,
    pub setup_script: PathBuf,
    pub env: BTreeMap<String, String>,
    #[serde(skip_serializing)]
    config_dir: PathBuf,
}

impl Config {
    pub fn resolve_setup_script(&self) -> PathBuf {
        if self.setup_script.is_absolute() {
            self.setup_script.clone()
        } else {
            self.config_dir.join(&self.setup_script)
        }
    }
}

pub fn load(path: &Path) -> Result<Config, GiftwrapError> {
    let text = fs::read_to_string(path).map_err(|err| {
        GiftwrapError::config_hint(
            format!("failed to read config {}: {err}", path.display()),
            format!("ensure {CONFIG_REL_PATH} exists and is readable"),
        )
    })?;

    let value: toml::Value = toml::from_str(&text)
        .map_err(|err| GiftwrapError::config(format!("failed to parse config: {err}")))?;

    let table = value
        .as_table()
        .ok_or_else(|| GiftwrapError::config("config root must be a TOML table"))?;

    for key in table.keys() {
        if key != "image" && key != "setup_script" && key != "env" {
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

    let env = parse_env(table.get("env"))?;

    let config_dir = path.parent().ok_or_else(|| {
        GiftwrapError::config(format!(
            "could not resolve config directory from {}",
            path.display()
        ))
    })?;

    let config = Config {
        image,
        setup_script: PathBuf::from(setup_script),
        env,
        config_dir: config_dir.to_path_buf(),
    };

    let setup_path = config.resolve_setup_script();
    if !setup_path.exists() {
        return Err(GiftwrapError::config_hint(
            format!("setup_script does not exist: {}", setup_path.display()),
            "set setup_script to a valid path relative to .giftwrap/config.toml or absolute path",
        ));
    }

    Ok(config)
}

fn parse_env(value: Option<&toml::Value>) -> Result<BTreeMap<String, String>, GiftwrapError> {
    let mut env = BTreeMap::new();

    let Some(value) = value else {
        return Ok(env);
    };

    let env_table = value.as_table().ok_or_else(|| {
        GiftwrapError::config("env must be a TOML table of string key/value pairs")
    })?;

    for (key, raw_value) in env_table {
        if !is_valid_env_key(key) {
            return Err(GiftwrapError::config(format!(
                "env key must match [A-Za-z_][A-Za-z0-9_]*: {key}"
            )));
        }
        if key.starts_with("GW_") {
            return Err(GiftwrapError::config(format!(
                "env key is reserved for giftwrap: {key}"
            )));
        }

        let value = raw_value.as_str().ok_or_else(|| {
            GiftwrapError::config(format!("env value for {key} must be a string"))
        })?;
        env.insert(key.clone(), value.to_string());
    }

    Ok(env)
}

fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if first != '_' && !first.is_ascii_alphabetic() {
        return false;
    }

    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
