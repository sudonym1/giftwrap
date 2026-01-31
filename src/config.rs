use std::collections::HashMap;
use std::env;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

/// Parsed configuration plus build-root discovery metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Directory containing the config file (build root).
    pub root_dir: PathBuf,
    /// Full path to the config file that was loaded.
    pub config_path: PathBuf,
    /// Raw parameter map after applying env overrides.
    pub params: HashMap<String, Vec<String>>,
    /// Optional UUID used to scope GW_USER_OPT_* overrides.
    pub uuid: Option<String>,
}

#[derive(Debug)]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ConfigError {}

const CONFIG_NAMES: [&str; 2] = [".giftwrap", "giftwrap"];
const ENV_SET_PREFIX: &str = "GW_USER_OPT_SET_";
const ENV_ADD_PREFIX: &str = "GW_USER_OPT_ADD_";
const ENV_DEL_PREFIX: &str = "GW_USER_OPT_DEL_";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnvOpt {
    Set,
    Add,
    Del,
}

pub fn load_from(start_dir: &Path) -> Result<Config, ConfigError> {
    let (root_dir, config_path) = discover_config(start_dir)?;
    let mut params = parse_config(&config_path)?;

    let uuid = params
        .get("uuid")
        .and_then(|vals| vals.first())
        .map(|v| v.replace('-', ""));

    apply_env_overrides(&mut params, uuid.as_deref())?;

    if !params.contains_key("gw_container") {
        return Err(ConfigError::new(format!(
            "Error: gw_container must be specified in {}",
            config_path.display()
        )));
    }

    if params.contains_key("prefix_cmd") && params.contains_key("prefix_cmd_quiet") {
        return Err(ConfigError::new(
            "Error: must specify at most one of prefix_cmd and prefix_cmd_quiet",
        ));
    }

    Ok(Config {
        root_dir,
        config_path,
        params,
        uuid,
    })
}

fn discover_config(start_dir: &Path) -> Result<(PathBuf, PathBuf), ConfigError> {
    let mut cwd = start_dir
        .canonicalize()
        .map_err(|err| ConfigError::new(format!("Error: failed to resolve cwd: {err}")))?;
    let root = Path::new("/");

    while cwd != root {
        for name in CONFIG_NAMES {
            let candidate = cwd.join(name);
            if candidate.is_file() {
                return Ok((cwd, candidate));
            }
        }
        let parent = cwd
            .parent()
            .ok_or_else(|| ConfigError::new("Error: never found a config file"))?;
        cwd = parent.to_path_buf();
    }

    Err(ConfigError::new("Error: never found a config file"))
}

fn parse_config(config_path: &Path) -> Result<HashMap<String, Vec<String>>, ConfigError> {
    let content = std::fs::read_to_string(config_path).map_err(|err| {
        ConfigError::new(format!(
            "Error: failed to read config file {}: {err}",
            config_path.display()
        ))
    })?;

    let mut params = HashMap::new();
    for (idx, raw_line) in content.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let parts = shell_words::split(line).map_err(|err| {
            ConfigError::new(format!(
                "Error: failed to parse config line {}: {err}",
                idx + 1
            ))
        })?;

        if parts.is_empty() {
            continue;
        }

        let key = parts[0].clone();
        let values = parts[1..].to_vec();
        params.insert(key, values);
    }

    Ok(params)
}

fn apply_env_overrides(
    params: &mut HashMap<String, Vec<String>>,
    uuid: Option<&str>,
) -> Result<(), ConfigError> {
    for (key, value) in env::vars() {
        let Some((op, opt)) = handle_env_opt(&key, uuid) else {
            continue;
        };

        match op {
            EnvOpt::Del => {
                params.remove(&opt);
            }
            EnvOpt::Add => {
                let parts = shell_words::split(&value).map_err(|err| {
                    ConfigError::new(format!("Error: failed to parse env override {key}: {err}"))
                })?;
                params.entry(opt).or_default().extend(parts);
            }
            EnvOpt::Set => {
                let parts = shell_words::split(&value).map_err(|err| {
                    ConfigError::new(format!("Error: failed to parse env override {key}: {err}"))
                })?;
                params.insert(opt, parts);
            }
        }
    }
    Ok(())
}

fn handle_env_opt(key: &str, uuid: Option<&str>) -> Option<(EnvOpt, String)> {
    let (op, rest) = if let Some(stripped) = key.strip_prefix(ENV_SET_PREFIX) {
        (EnvOpt::Set, stripped)
    } else if let Some(stripped) = key.strip_prefix(ENV_ADD_PREFIX) {
        (EnvOpt::Add, stripped)
    } else if let Some(stripped) = key.strip_prefix(ENV_DEL_PREFIX) {
        (EnvOpt::Del, stripped)
    } else {
        return None;
    };

    if !rest.starts_with("UUID_") {
        return Some((op, rest.to_string()));
    }

    let uuid = uuid?;
    let expected = format!("UUID_{uuid}_");
    if !rest.starts_with(&expected) {
        return None;
    }

    Some((op, rest[expected.len()..].to_string()))
}
