use std::fmt;
use std::path::Path;

use crate::internal::ContainerSpec;
use crate::podman_cli;

#[derive(Debug)]
pub struct ExecError {
    message: String,
}

impl ExecError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ExecError {}

pub fn build_image(image: &str, context_dir: &Path) -> Result<(), ExecError> {
    podman_cli::build_image(image, context_dir).map_err(|err| ExecError::new(err.to_string()))
}

pub fn image_exists(image: &str) -> Result<bool, ExecError> {
    podman_cli::image_exists(image).map_err(|err| ExecError::new(err.to_string()))
}

pub fn run_container(spec: &ContainerSpec) -> Result<(), ExecError> {
    podman_cli::exec_run(spec).map_err(|err| ExecError::new(err.to_string()))
}
