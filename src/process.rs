use std::process::{Command, Output};

use crate::errors::GiftwrapError;
use crate::log::Logger;

#[derive(Clone, Copy)]
pub enum CommandFailureKind {
    Tooling,
    Build,
    Runtime,
}

pub fn run_checked(
    program: &str,
    args: &[String],
    kind: CommandFailureKind,
    phase: &str,
    logger: &Logger,
) -> Result<(), GiftwrapError> {
    logger.command(program, args);

    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|err| command_spawn_error(kind, phase, program, err.to_string()))?;

    if status.success() {
        return Ok(());
    }

    Err(match kind {
        CommandFailureKind::Tooling => GiftwrapError::tooling(format!(
            "failed to {phase} ({})",
            crate::errors::exit_status_desc(&status)
        )),
        CommandFailureKind::Build => GiftwrapError::build_command_failed(phase, &status),
        CommandFailureKind::Runtime => GiftwrapError::runtime_command_failed(phase, &status),
    })
}

pub fn run_capture(
    program: &str,
    args: &[String],
    kind: CommandFailureKind,
    phase: &str,
    logger: &Logger,
) -> Result<Output, GiftwrapError> {
    logger.command(program, args);

    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| command_spawn_error(kind, phase, program, err.to_string()))?;

    if output.status.success() {
        return Ok(output);
    }

    Err(match kind {
        CommandFailureKind::Tooling => GiftwrapError::tooling(format!(
            "failed to {phase} ({})",
            crate::errors::exit_status_desc(&output.status)
        )),
        CommandFailureKind::Build => GiftwrapError::build_command_failed(phase, &output.status),
        CommandFailureKind::Runtime => GiftwrapError::runtime_command_failed(phase, &output.status),
    })
}

fn command_spawn_error(
    kind: CommandFailureKind,
    phase: &str,
    program: &str,
    detail: String,
) -> GiftwrapError {
    let message = format!("failed to execute {program} while attempting to {phase}: {detail}");
    match kind {
        CommandFailureKind::Tooling => GiftwrapError::tooling(message),
        CommandFailureKind::Build => GiftwrapError::build(message),
        CommandFailureKind::Runtime => GiftwrapError::runtime(message),
    }
}
