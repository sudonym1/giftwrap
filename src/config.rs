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

#[cfg(test)]
mod tests {
    use super::{apply_env_overrides, discover_config, load_from, parse_config};
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().expect("env lock poisoned")
    }

    struct EnvVarGuard {
        key: String,
        prior: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &str, value: &str) -> Self {
            let prior = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key: key.to_string(),
                prior,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = self.prior.take() {
                unsafe {
                    std::env::set_var(&self.key, value);
                }
            } else {
                unsafe {
                    std::env::remove_var(&self.key);
                }
            }
        }
    }

    fn write_config(dir: &Path, name: &str) {
        let path = dir.join(name);
        fs::write(path, "gw_container test").unwrap();
    }

    fn write_config_contents(dir: &Path, name: &str, contents: &str) {
        let path = dir.join(name);
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn discover_config_finds_dot_giftwrap_in_start_dir() {
        let temp = tempfile::tempdir().unwrap();
        write_config(temp.path(), ".giftwrap");

        let (root_dir, config_path) = discover_config(temp.path()).unwrap();
        let canonical_root = temp.path().canonicalize().unwrap();

        assert_eq!(root_dir, canonical_root);
        assert_eq!(config_path, canonical_root.join(".giftwrap"));
    }

    #[test]
    fn discover_config_walks_up_to_parent() {
        let temp = tempfile::tempdir().unwrap();
        write_config(temp.path(), "giftwrap");

        let nested = temp.path().join("child/grandchild");
        fs::create_dir_all(&nested).unwrap();

        let (root_dir, config_path) = discover_config(&nested).unwrap();
        let canonical_root = temp.path().canonicalize().unwrap();

        assert_eq!(root_dir, canonical_root);
        assert_eq!(config_path, canonical_root.join("giftwrap"));
    }

    #[test]
    fn discover_config_prefers_dot_giftwrap_over_giftwrap() {
        let temp = tempfile::tempdir().unwrap();
        write_config(temp.path(), ".giftwrap");
        write_config(temp.path(), "giftwrap");

        let (root_dir, config_path) = discover_config(temp.path()).unwrap();
        let canonical_root = temp.path().canonicalize().unwrap();

        assert_eq!(root_dir, canonical_root);
        assert_eq!(config_path, canonical_root.join(".giftwrap"));
    }

    #[test]
    fn discover_config_errors_when_missing() {
        let temp = tempfile::tempdir().unwrap();

        let err = discover_config(temp.path()).unwrap_err();

        assert_eq!(err.to_string(), "Error: never found a config file");
    }

    #[test]
    fn parse_config_skips_comments_and_parses_values() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("giftwrap");
        fs::write(
            &path,
            r#"
# comment
gw_container test
extra_args "one two" three
empty_key
"#,
        )
        .unwrap();

        let params = parse_config(&path).unwrap();

        assert_eq!(
            params.get("gw_container").unwrap(),
            &vec!["test".to_string()]
        );
        assert_eq!(
            params.get("extra_args").unwrap(),
            &vec!["one two".to_string(), "three".to_string()]
        );
        assert_eq!(params.get("empty_key").unwrap(), &Vec::<String>::new());
    }

    #[test]
    fn parse_config_reports_line_number_on_error() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("giftwrap");
        fs::write(&path, "gw_container test\nbad \"unterminated\n").unwrap();

        let err = parse_config(&path).unwrap_err();

        assert!(err
            .to_string()
            .starts_with("Error: failed to parse config line 2:"));
    }

    #[test]
    fn apply_env_overrides_set_add_and_del() {
        let _lock = lock_env();
        let _set = EnvVarGuard::set("GW_USER_OPT_SET_suite5_param_x1c9", "new1 new2");
        let _add = EnvVarGuard::set("GW_USER_OPT_ADD_suite5_list_x1c9", "b2 'b three'");
        let _del = EnvVarGuard::set("GW_USER_OPT_DEL_suite5_remove_x1c9", "ignored");

        let mut params = HashMap::new();
        params.insert("suite5_param_x1c9".to_string(), vec!["old".to_string()]);
        params.insert("suite5_list_x1c9".to_string(), vec!["a".to_string()]);
        params.insert("suite5_remove_x1c9".to_string(), vec!["keep".to_string()]);

        apply_env_overrides(&mut params, None).unwrap();

        assert_eq!(
            params.get("suite5_param_x1c9").unwrap(),
            &vec!["new1".to_string(), "new2".to_string()]
        );
        assert_eq!(
            params.get("suite5_list_x1c9").unwrap(),
            &vec!["a".to_string(), "b2".to_string(), "b three".to_string()]
        );
        assert!(params.get("suite5_remove_x1c9").is_none());
    }

    #[test]
    fn apply_env_overrides_respects_uuid_scoping() {
        let _lock = lock_env();
        let _match_scoped = EnvVarGuard::set("GW_USER_OPT_SET_UUID_abc123_scoped_x1c9", "scoped");
        let _mismatch_scoped =
            EnvVarGuard::set("GW_USER_OPT_SET_UUID_other_scoped_x1c9", "ignored");
        let _other = EnvVarGuard::set("GW_USER_OPT_SET_UUID_abc123_other_x1c9", "other");

        let mut params = HashMap::new();
        params.insert("scoped_x1c9".to_string(), vec!["base".to_string()]);

        apply_env_overrides(&mut params, Some("abc123")).unwrap();

        assert_eq!(
            params.get("scoped_x1c9").unwrap(),
            &vec!["scoped".to_string()]
        );
        assert_eq!(
            params.get("other_x1c9").unwrap(),
            &vec!["other".to_string()]
        );
    }

    #[test]
    fn apply_env_overrides_ignores_uuid_scoped_without_uuid() {
        let _lock = lock_env();
        let _guard = EnvVarGuard::set("GW_USER_OPT_SET_UUID_abc123_scoped_x1c9", "scoped");

        let mut params = HashMap::new();
        params.insert("scoped_x1c9".to_string(), vec!["base".to_string()]);

        apply_env_overrides(&mut params, None).unwrap();

        assert_eq!(
            params.get("scoped_x1c9").unwrap(),
            &vec!["base".to_string()]
        );
    }

    #[test]
    fn apply_env_overrides_reports_bad_shell_words() {
        let _lock = lock_env();
        let _guard = EnvVarGuard::set("GW_USER_OPT_SET_suite5_bad_x1c9", "\"unterminated");

        let mut params = HashMap::new();
        let err = apply_env_overrides(&mut params, None).unwrap_err();

        assert!(err
            .to_string()
            .starts_with("Error: failed to parse env override GW_USER_OPT_SET_suite5_bad_x1c9:"));
    }

    #[test]
    fn load_from_applies_uuid_overrides_after_dash_stripping() {
        let _lock = lock_env();
        let temp = TempDir::new().unwrap();
        write_config_contents(
            temp.path(),
            ".giftwrap",
            "gw_container test\nuuid 1234-5678\nextra_args base\n",
        );
        let _guard = EnvVarGuard::set("GW_USER_OPT_ADD_UUID_12345678_extra_args", "more");

        let config = load_from(temp.path()).unwrap();

        assert_eq!(config.uuid.as_deref(), Some("12345678"));
        assert_eq!(
            config.params.get("extra_args").unwrap(),
            &vec!["base".to_string(), "more".to_string()]
        );
    }

    #[test]
    fn load_from_errors_without_gw_container() {
        let temp = TempDir::new().unwrap();
        write_config_contents(temp.path(), "giftwrap", "extra_args base\n");

        let err = load_from(temp.path()).unwrap_err();

        assert_eq!(
            err.to_string(),
            format!(
                "Error: gw_container must be specified in {}",
                temp.path().join("giftwrap").display()
            )
        );
    }

    #[test]
    fn load_from_errors_on_prefix_conflict() {
        let temp = TempDir::new().unwrap();
        write_config_contents(
            temp.path(),
            "giftwrap",
            "gw_container test\nprefix_cmd echo\nprefix_cmd_quiet echo\n",
        );

        let err = load_from(temp.path()).unwrap_err();

        assert_eq!(
            err.to_string(),
            "Error: must specify at most one of prefix_cmd and prefix_cmd_quiet"
        );
    }
}
