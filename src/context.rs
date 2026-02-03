use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use sha1::{Digest, Sha1};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSha {
    pub sha: String,
    pub files: Vec<String>,
    pub sha_file: PathBuf,
    pub cached: bool,
}

const CONFIG_NAME: &str = ".giftwrap.toml";

#[derive(Debug)]
pub struct ContextError {
    message: String,
}

impl ContextError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ContextError {}

#[derive(Clone, Debug)]
struct GwPattern {
    base_dir: PathBuf,
    include: bool,
    dir_only: bool,
    anchored: bool,
    has_slash: bool,
    tokens: Vec<Token>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Star,
    DoubleStar,
    Qm,
    Char(char),
}

pub fn load_from_config(
    root_dir: &Path,
    params: &HashMap<String, Vec<String>>,
) -> Result<Option<ContextSha>, ContextError> {
    let Some(ctx) = params.get("version_by_build_context") else {
        return Ok(None);
    };
    if ctx.len() != 1 {
        return Err(ContextError::new(
            "Error: version_by_build_context requires a single sha file path",
        ));
    }
    let Some(rules) = params.get("gw_context_rules") else {
        return Err(ContextError::new(
            "Error: version_by_build_context requires gw_context_rules in .giftwrap.toml",
        ));
    };

    let sha_file = root_dir.join(&ctx[0]);
    let context = build_context_sha(root_dir, &sha_file, rules)?;
    Ok(Some(context))
}

pub fn build_context_sha(
    root_dir: &Path,
    sha_file: &Path,
    rules: &[String],
) -> Result<ContextSha, ContextError> {
    let sha_file = if sha_file.is_absolute() {
        sha_file.to_path_buf()
    } else {
        root_dir.join(sha_file)
    };
    let files = build_context_file_list(root_dir, rules)?;

    let dirty = is_sha_file_dirty(&sha_file, &files, root_dir)?;
    let sha = if dirty {
        let sha = compute_sha(root_dir, &files)?;
        write_sha_file(&sha_file, &sha, &files)?;
        sha
    } else {
        read_sha_file(&sha_file)?
    };

    Ok(ContextSha {
        sha,
        files,
        sha_file,
        cached: !dirty,
    })
}

fn build_context_file_list(
    root_dir: &Path,
    rules: &[String],
) -> Result<Vec<String>, ContextError> {
    let files = collect_files(root_dir)?;
    let patterns = parse_rules(rules);

    let mut selected = BTreeSet::new();
    for rel_path in &files {
        let mut included = false;
        for pattern in &patterns {
            if !rel_path.starts_with(&pattern.base_dir) {
                continue;
            }
            let rel_to_base = rel_path.strip_prefix(&pattern.base_dir).unwrap_or(rel_path);
            if gw_pattern_matches(pattern, rel_to_base) {
                included = pattern.include;
            }
        }
        if included {
            selected.insert(path_to_slash(rel_path));
        }
    }

    selected.insert(CONFIG_NAME.to_string());

    Ok(selected.into_iter().collect())
}

fn collect_files(root_dir: &Path) -> Result<Vec<PathBuf>, ContextError> {
    let mut files = Vec::new();
    walk_dir(root_dir, root_dir, &mut files)?;
    Ok(files)
}

fn walk_dir(root_dir: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), ContextError> {
    for entry in fs::read_dir(dir).map_err(|err| {
        ContextError::new(format!(
            "Error: failed to read directory {}: {err}",
            dir.display()
        ))
    })? {
        let entry = entry.map_err(|err| {
            ContextError::new(format!(
                "Error: failed to read directory entry {}: {err}",
                dir.display()
            ))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|err| {
            ContextError::new(format!(
                "Error: failed to read entry type {}: {err}",
                path.display()
            ))
        })?;

        if file_type.is_dir() {
            walk_dir(root_dir, &path, files)?;
        } else if file_type.is_file() || file_type.is_symlink() {
            let rel = path
                .strip_prefix(root_dir)
                .map_err(|_| {
                    ContextError::new(format!(
                        "Error: failed to relativize path {}",
                        path.display()
                    ))
                })?
                .to_path_buf();
            files.push(rel);
        }
    }
    Ok(())
}

fn parse_rules(rules: &[String]) -> Vec<GwPattern> {
    let mut patterns = Vec::new();
    let base_dir = PathBuf::new();
    for raw_line in rules {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let (include, pattern_raw) = if let Some(rest) = line.strip_prefix('!') {
            (false, rest.trim())
        } else {
            (true, line)
        };
        let mut anchored = false;
        let mut pattern = pattern_raw.to_string();
        if let Some(rest) = pattern.strip_prefix('/') {
            anchored = true;
            pattern = rest.to_string();
        }
        let dir_only = pattern.ends_with('/');
        if dir_only {
            pattern.truncate(pattern.trim_end_matches('/').len());
        }
        if pattern.is_empty() {
            continue;
        }
        let has_slash = pattern.contains('/');
        let tokens = tokenize(&pattern);
        patterns.push(GwPattern {
            base_dir: base_dir.clone(),
            include,
            dir_only,
            anchored,
            has_slash,
            tokens,
        });
    }

    patterns
}

fn gw_pattern_matches(pattern: &GwPattern, rel_path: &Path) -> bool {
    let rel_str = path_to_slash(rel_path);
    let components = split_components(&rel_str);

    if pattern.dir_only {
        if components.len() < 2 {
            return false;
        }
        let dir_components = &components[..components.len() - 1];
        if pattern.anchored || pattern.has_slash {
            for idx in 1..=dir_components.len() {
                let prefix = join_components(&dir_components[..idx]);
                if glob_match_tokens(&pattern.tokens, &prefix) {
                    return true;
                }
            }
            false
        } else {
            dir_components
                .iter()
                .any(|component| glob_match_tokens(&pattern.tokens, component))
        }
    } else if pattern.anchored || pattern.has_slash {
        glob_match_tokens(&pattern.tokens, &rel_str)
    } else {
        components
            .iter()
            .any(|component| glob_match_tokens(&pattern.tokens, component))
    }
}

fn tokenize(pattern: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '*' {
            if matches!(chars.peek(), Some('*')) {
                while matches!(chars.peek(), Some('*')) {
                    chars.next();
                }
                tokens.push(Token::DoubleStar);
            } else {
                tokens.push(Token::Star);
            }
        } else if ch == '?' {
            tokens.push(Token::Qm);
        } else {
            tokens.push(Token::Char(ch));
        }
    }
    tokens
}

fn glob_match_tokens(tokens: &[Token], text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut memo = vec![vec![None; chars.len() + 1]; tokens.len() + 1];
    glob_match_inner(tokens, &chars, 0, 0, &mut memo)
}

fn glob_match_inner(
    tokens: &[Token],
    text: &[char],
    ti: usize,
    si: usize,
    memo: &mut [Vec<Option<bool>>],
) -> bool {
    if let Some(result) = memo[ti][si] {
        return result;
    }
    let result = if ti == tokens.len() {
        si == text.len()
    } else {
        match tokens[ti] {
            Token::Char(c) => {
                si < text.len()
                    && text[si] == c
                    && glob_match_inner(tokens, text, ti + 1, si + 1, memo)
            }
            Token::Qm => {
                si < text.len()
                    && text[si] != '/'
                    && glob_match_inner(tokens, text, ti + 1, si + 1, memo)
            }
            Token::Star => {
                if glob_match_inner(tokens, text, ti + 1, si, memo) {
                    true
                } else {
                    let mut idx = si;
                    while idx < text.len() && text[idx] != '/' {
                        idx += 1;
                        if glob_match_inner(tokens, text, ti + 1, idx, memo) {
                            return true;
                        }
                    }
                    false
                }
            }
            Token::DoubleStar => {
                if glob_match_inner(tokens, text, ti + 1, si, memo) {
                    true
                } else {
                    let mut idx = si;
                    while idx < text.len() {
                        idx += 1;
                        if glob_match_inner(tokens, text, ti + 1, idx, memo) {
                            return true;
                        }
                    }
                    false
                }
            }
        }
    };
    memo[ti][si] = Some(result);
    result
}

fn is_sha_file_dirty(
    sha_file: &Path,
    file_list: &[String],
    root_dir: &Path,
) -> Result<bool, ContextError> {
    if !sha_file.exists() {
        return Ok(true);
    }
    let contents = fs::read_to_string(sha_file).map_err(|err| {
        ContextError::new(format!(
            "Error: failed to read sha file {}: {err}",
            sha_file.display()
        ))
    })?;
    let mut lines = contents.lines();
    let _existing_sha = match lines.next() {
        Some(line) => line.trim(),
        None => return Ok(true),
    };
    let stored_files: Vec<String> = lines.map(|line| line.trim().to_string()).collect();
    if stored_files != file_list {
        return Ok(true);
    }

    let sha_mtime = fs::metadata(sha_file)
        .and_then(|meta| meta.modified())
        .map_err(|err| {
            ContextError::new(format!(
                "Error: failed to stat sha file {}: {err}",
                sha_file.display()
            ))
        })?;

    for rel in file_list {
        let path = root_dir.join(rel);
        let meta = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(_) => return Ok(true),
        };
        let mtime = match meta.modified() {
            Ok(mtime) => mtime,
            Err(_) => return Ok(true),
        };
        if mtime > sha_mtime {
            return Ok(true);
        }
    }

    Ok(false)
}

fn compute_sha(root_dir: &Path, file_list: &[String]) -> Result<String, ContextError> {
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    for rel in file_list {
        let path = root_dir.join(rel);
        if !path.is_file() {
            continue;
        }
        let mut file = fs::File::open(&path).map_err(|err| {
            ContextError::new(format!(
                "Error: failed to read file {}: {err}",
                path.display()
            ))
        })?;
        loop {
            let read = file.read(&mut buf).map_err(|err| {
                ContextError::new(format!(
                    "Error: failed to read file {}: {err}",
                    path.display()
                ))
            })?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
        }
    }
    let digest = hasher.finalize();
    Ok(format!("{:x}", digest))
}

fn write_sha_file(sha_file: &Path, sha: &str, file_list: &[String]) -> Result<(), ContextError> {
    let mut output = String::new();
    output.push_str(sha);
    output.push('\n');
    if !file_list.is_empty() {
        output.push_str(&file_list.join("\n"));
    }
    fs::write(sha_file, output).map_err(|err| {
        ContextError::new(format!(
            "Error: failed to write sha file {}: {err}",
            sha_file.display()
        ))
    })
}

fn read_sha_file(sha_file: &Path) -> Result<String, ContextError> {
    let contents = fs::read_to_string(sha_file).map_err(|err| {
        ContextError::new(format!(
            "Error: failed to read sha file {}: {err}",
            sha_file.display()
        ))
    })?;
    contents
        .lines()
        .next()
        .map(|line| line.trim().to_string())
        .ok_or_else(|| {
            ContextError::new(format!("Error: sha file {} is empty", sha_file.display()))
        })
}

fn path_to_slash(path: &Path) -> String {
    let mut parts = Vec::new();
    for comp in path.components() {
        if let Component::Normal(os) = comp {
            parts.push(os.to_string_lossy());
        }
    }
    parts.join("/")
}

fn split_components(path: &str) -> Vec<&str> {
    if path.is_empty() {
        Vec::new()
    } else {
        path.split('/').collect()
    }
}

fn join_components(components: &[&str]) -> String {
    components.join("/")
}

#[cfg(test)]
mod tests {
    use super::{build_context_file_list, build_context_sha, compute_sha, load_from_config};
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn write_file(root: &Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn build_context_file_list_applies_include_exclude_rules() {
        let temp = tempdir().unwrap();
        write_file(temp.path(), ".giftwrap.toml", "gw_container = \"test\"\n");
        let rules = vec![
            "/src/*.rs".to_string(),
            "build/".to_string(),
            "docs/*.md".to_string(),
            "!/docs/secret.md".to_string(),
        ];
        write_file(temp.path(), "src/main.rs", "fn main() {}\n");
        write_file(temp.path(), "nested/src/other.rs", "fn other() {}\n");
        write_file(temp.path(), "build/output.log", "log\n");
        write_file(temp.path(), "nested/build/inner.txt", "inner\n");
        write_file(temp.path(), "docs/readme.md", "readme\n");
        write_file(temp.path(), "docs/secret.md", "secret\n");
        write_file(temp.path(), "nested/docs/extra.md", "extra\n");
        write_file(temp.path(), "notes.txt", "notes\n");

        let files = build_context_file_list(temp.path(), &rules).unwrap();

        let expected = vec![
            ".giftwrap.toml",
            "build/output.log",
            "docs/readme.md",
            "nested/build/inner.txt",
            "src/main.rs",
        ];
        assert_eq!(files, expected);
    }

    #[test]
    fn build_context_file_list_applies_rule_order_overrides() {
        let temp = tempdir().unwrap();
        write_file(temp.path(), ".giftwrap.toml", "gw_container = \"test\"\n");
        let rules = vec!["*.txt".to_string(), "!secret.txt".to_string()];
        write_file(temp.path(), "notes.txt", "notes\n");
        write_file(temp.path(), "nested/keep.txt", "keep\n");
        write_file(temp.path(), "nested/secret.txt", "secret\n");

        let files = build_context_file_list(temp.path(), &rules).unwrap();

        let expected = vec![".giftwrap.toml", "nested/keep.txt", "notes.txt"];
        assert_eq!(files, expected);
    }

    #[test]
    fn compute_sha_is_stable_and_changes_with_content() {
        let temp = tempdir().unwrap();
        write_file(temp.path(), ".giftwrap.toml", "gw_container = \"test\"\n");
        let rules = vec!["*.txt".to_string()];
        write_file(temp.path(), "a.txt", "alpha\n");
        write_file(temp.path(), "b.txt", "beta\n");

        let files = build_context_file_list(temp.path(), &rules).unwrap();

        let first = compute_sha(temp.path(), &files).unwrap();
        let second = compute_sha(temp.path(), &files).unwrap();
        assert_eq!(first, second);

        write_file(temp.path(), "b.txt", "beta2\n");
        let third = compute_sha(temp.path(), &files).unwrap();
        assert_ne!(first, third);
    }

    #[test]
    fn load_from_config_errors_without_context_rules() {
        let temp = tempdir().unwrap();
        write_file(temp.path(), ".giftwrap.toml", "gw_container = \"test\"\n");

        let mut params = HashMap::new();
        params.insert(
            "version_by_build_context".to_string(),
            vec![".ctx".to_string()],
        );

        let err = load_from_config(temp.path(), &params).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Error: version_by_build_context requires gw_context_rules in .giftwrap.toml"
        );
    }

    #[test]
    fn build_context_sha_uses_cache_when_unchanged() {
        let temp = tempdir().unwrap();
        write_file(temp.path(), ".giftwrap.toml", "gw_container = \"test\"\n");
        write_file(temp.path(), "data.txt", "alpha\n");
        let rules = vec!["/data.txt".to_string()];
        let sha_file = temp.path().join(".gwcontext");

        let first = build_context_sha(temp.path(), &sha_file, &rules).unwrap();
        assert!(!first.cached);

        let second = build_context_sha(temp.path(), &sha_file, &rules).unwrap();
        assert!(second.cached);
        assert_eq!(first.sha, second.sha);
    }

    #[test]
    fn build_context_sha_recalculates_when_file_list_changes() {
        let temp = tempdir().unwrap();
        write_file(temp.path(), ".giftwrap.toml", "gw_container = \"test\"\n");
        write_file(temp.path(), "a.txt", "alpha\n");
        let rules = vec!["*.txt".to_string()];
        let sha_file = temp.path().join(".gwcontext");

        let first = build_context_sha(temp.path(), &sha_file, &rules).unwrap();
        assert!(!first.cached);

        write_file(temp.path(), "b.txt", "beta\n");
        let second = build_context_sha(temp.path(), &sha_file, &rules).unwrap();
        assert!(!second.cached);
        assert_ne!(first.sha, second.sha);
    }
}
