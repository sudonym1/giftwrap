use std::fmt;

/// High-level action requested by CLI flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliAction {
    Run,
    PrintCommand,
    PrintContext,
    PrintImage,
    ShowConfig,
    Help,
}

/// Parsed CLI options before config/compose processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOptions {
    pub action: CliAction,
    /// Force a specific context SHA/tag.
    pub use_ctx: Option<String>,
    /// Override the image name/tag.
    pub override_image: Option<String>,
    /// Rebuild the image before running.
    pub rebuild: bool,
    /// Extra args supplied via --gw-extra-args.
    pub extra_args: Vec<String>,
    /// Runtime args provided before the `--` delimiter.
    pub runtime_args: Vec<String>,
}

/// User command captured after the `--` delimiter (or remaining args).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserCommand {
    pub argv: Vec<String>,
}

#[derive(Debug)]
pub struct CliError {
    message: String,
}

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CliError {}

pub fn parse_args(args: &[String]) -> Result<(CliOptions, UserCommand), CliError> {
    let mut action = CliAction::Run;
    let mut use_ctx = None;
    let mut override_image = None;
    let mut rebuild = false;
    let mut extra_args = Vec::new();
    let mut runtime_args = Vec::new();

    let mut idx = 0;
    let mut terminal_action = false;
    while idx < args.len() {
        let arg = &args[idx];
        if !arg.starts_with("--gw-") {
            break;
        }

        if arg == "--gw-print" {
            action = CliAction::PrintCommand;
        } else if arg == "--gw-ctx" {
            action = CliAction::PrintContext;
            terminal_action = true;
        } else if arg == "--gw-print-image" {
            action = CliAction::PrintImage;
            terminal_action = true;
        } else if let Some(rest) = arg.strip_prefix("--gw-use-ctx=") {
            use_ctx = Some(rest.to_string());
        } else if let Some(rest) = arg.strip_prefix("--gw-img=") {
            override_image = Some(rest.to_string());
        } else if arg == "--gw-rebuild" {
            rebuild = true;
        } else if let Some(rest) = arg.strip_prefix("--gw-extra-args=") {
            let parts = shell_words::split(rest).map_err(|err| {
                CliError::new(format!("Error: failed to parse --gw-extra-args: {err}"))
            })?;
            extra_args.extend(parts);
        } else if arg == "--gw-show-config" {
            action = CliAction::ShowConfig;
            terminal_action = true;
        } else if arg == "--gw-help" {
            action = CliAction::Help;
            terminal_action = true;
        }

        idx += 1;
        if terminal_action {
            break;
        }
    }

    let remaining = if terminal_action {
        Vec::new()
    } else {
        args[idx..].to_vec()
    };

    let user_cmd = if terminal_action {
        Vec::new()
    } else if let Some(pos) = remaining.iter().position(|arg| arg == "--") {
        runtime_args = remaining[..pos].to_vec();
        remaining[pos + 1..].to_vec()
    } else {
        remaining
    };

    Ok((
        CliOptions {
            action,
            use_ctx,
            override_image,
            rebuild,
            extra_args,
            runtime_args,
        },
        UserCommand { argv: user_cmd },
    ))
}

#[cfg(test)]
mod tests {
    use super::{CliAction, CliOptions, UserCommand, parse_args};

    fn parse(args: &[&str]) -> (CliOptions, UserCommand) {
        let argv = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        parse_args(&argv).expect("parse_args failed")
    }

    fn parse_err(args: &[&str]) -> String {
        let argv = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        parse_args(&argv)
            .err()
            .expect("expected parse_args to fail")
            .to_string()
    }

    #[test]
    fn parse_defaults_when_no_args() {
        let (opts, cmd) = parse(&[]);
        assert_eq!(opts.action, CliAction::Run);
        assert!(opts.use_ctx.is_none());
        assert!(opts.override_image.is_none());
        assert!(!opts.rebuild);
        assert!(opts.extra_args.is_empty());
        assert!(opts.runtime_args.is_empty());
        assert!(cmd.argv.is_empty());
    }

    #[test]
    fn parse_print_allows_runtime_and_user_command() {
        let (opts, cmd) = parse(&["--gw-print", "--rm", "--", "echo", "hi"]);
        assert_eq!(opts.action, CliAction::PrintCommand);
        assert_eq!(opts.runtime_args, vec!["--rm"]);
        assert_eq!(cmd.argv, vec!["echo", "hi"]);
    }

    #[test]
    fn parse_ctx_is_terminal_and_ignores_following_args() {
        let (opts, cmd) = parse(&["--gw-ctx", "--gw-use-ctx=deadbeef", "--", "echo"]);
        assert_eq!(opts.action, CliAction::PrintContext);
        assert!(opts.use_ctx.is_none());
        assert!(opts.runtime_args.is_empty());
        assert!(cmd.argv.is_empty());
    }

    #[test]
    fn parse_ctx_preserves_preceding_flags() {
        let (opts, cmd) = parse(&["--gw-use-ctx=deadbeef", "--gw-ctx", "--", "echo"]);
        assert_eq!(opts.action, CliAction::PrintContext);
        assert_eq!(opts.use_ctx.as_deref(), Some("deadbeef"));
        assert!(opts.runtime_args.is_empty());
        assert!(cmd.argv.is_empty());
    }

    #[test]
    fn parse_terminal_actions_ignore_remaining_args() {
        let (opts, cmd) = parse(&["--gw-help", "--rm", "--", "echo"]);
        assert_eq!(opts.action, CliAction::Help);
        assert!(opts.runtime_args.is_empty());
        assert!(cmd.argv.is_empty());

        let (opts, cmd) = parse(&["--gw-print-image", "--rm", "--", "echo"]);
        assert_eq!(opts.action, CliAction::PrintImage);
        assert!(opts.runtime_args.is_empty());
        assert!(cmd.argv.is_empty());

        let (opts, cmd) = parse(&["--gw-show-config", "--rm", "--", "echo"]);
        assert_eq!(opts.action, CliAction::ShowConfig);
        assert!(opts.runtime_args.is_empty());
        assert!(cmd.argv.is_empty());
    }

    #[test]
    fn parse_use_ctx_image_and_rebuild() {
        let (opts, cmd) = parse(&[
            "--gw-use-ctx=abc123",
            "--gw-img=registry.local/app:tag",
            "--gw-rebuild",
            "--",
            "bash",
        ]);
        assert_eq!(opts.use_ctx.as_deref(), Some("abc123"));
        assert_eq!(opts.override_image.as_deref(), Some("registry.local/app:tag"));
        assert!(opts.rebuild);
        assert_eq!(cmd.argv, vec!["bash"]);
    }

    #[test]
    fn parse_extra_args_splits_shell_words() {
        let (opts, cmd) = parse(&[
            "--gw-extra-args=--env FOO=bar --flag \"two words\"",
            "--",
            "cmd",
        ]);
        assert_eq!(
            opts.extra_args,
            vec!["--env", "FOO=bar", "--flag", "two words"]
        );
        assert_eq!(cmd.argv, vec!["cmd"]);
    }

    #[test]
    fn parse_extra_args_errors_on_invalid_shell_words() {
        let message = parse_err(&["--gw-extra-args=--env 'unterminated"]);
        assert!(
            message.starts_with("Error: failed to parse --gw-extra-args:"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn parse_delimiter_splits_runtime_and_user_command() {
        let (opts, cmd) = parse(&[
            "--gw-use-ctx=abc123",
            "--volume=/src:/src",
            "--net=host",
            "--",
            "make",
            "test",
        ]);
        assert_eq!(opts.use_ctx.as_deref(), Some("abc123"));
        assert_eq!(opts.runtime_args, vec!["--volume=/src:/src", "--net=host"]);
        assert_eq!(cmd.argv, vec!["make", "test"]);
    }

    #[test]
    fn parse_without_delimiter_treats_remaining_as_user_command() {
        let (opts, cmd) = parse(&["--gw-use-ctx=abc123", "bash", "-lc", "true"]);
        assert_eq!(opts.use_ctx.as_deref(), Some("abc123"));
        assert!(opts.runtime_args.is_empty());
        assert_eq!(cmd.argv, vec!["bash", "-lc", "true"]);
    }
}
