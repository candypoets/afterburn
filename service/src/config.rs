//! Paths and environment overrides.
//!
//! The crate lives inside the Mindmap repository during development, but the binary is meant to be
//! installed. `MINDMAP_ROOT` is the explicit override; otherwise we walk up from the current
//! directory to find the checkout that contains `service/assets/empty.figcraft`, and finally fall
//! back to the compile-time manifest parent (which is right for `cargo run` and for a checkout).

use std::env;
use std::path::{Path, PathBuf};

use crate::error::Result;

/// Everything the node needs to find its tools.
#[derive(Clone, Debug)]
pub struct Config {
    /// `target/release/figcraft-cli`, used only by `migrate`.
    pub figcraft_cli: PathBuf,
    /// `herdr` or an override.
    pub herdr_bin: PathBuf,
}

impl Config {
    /// Read the environment and discover the repository root.
    pub fn discover() -> Result<Self> {
        let current_dir = env::current_dir().map_err(|error| {
            crate::error::Error::new(format!("cannot read the current directory: {error}"))
        })?;
        let root = env_path("MINDMAP_ROOT")
            .or_else(|| {
                current_dir
                    .ancestors()
                    .find(|candidate| {
                        candidate
                            .join("service")
                            .join("assets")
                            .join("empty.figcraft")
                            .is_file()
                    })
                    .map(Path::to_path_buf)
            })
            .or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .map(Path::to_path_buf)
            })
            .unwrap_or(current_dir);

        Ok(Self {
            figcraft_cli: env_path("MINDMAP_FIGCRAFT_CLI")
                .unwrap_or_else(|| root.join("target/release/figcraft-cli")),
            herdr_bin: env_path("MINDMAP_HERDR").unwrap_or_else(|| PathBuf::from("herdr")),
        })
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}
