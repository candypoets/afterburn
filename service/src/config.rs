//! Paths, addresses and environment overrides.
//!
//! The crate lives inside the Mindmap repository during development, but the binary is meant to be
//! installed. `MINDMAP_ROOT` is the explicit override; otherwise we walk up from the current
//! directory to find the checkout that contains `service/assets/empty.figcraft`, and finally fall
//! back to the compile-time manifest parent (which is right for `cargo run` and for a checkout).

use std::env;
use std::path::{Path, PathBuf};

use crate::error::Result;

/// Default public address of the document server, as used by the Python service.
pub const DEFAULT_SERVER: &str = "100.104.9.15:47910";
/// Where the local FigCraft server listens when the CLI publishes.
pub const DEFAULT_LOCAL_SERVER: &str = "ws://127.0.0.1:47910/";
/// The project-service TCP API.
pub const DEFAULT_BIND: &str = "0.0.0.0:47821";

/// Everything the service needs to find its files and peers.
#[derive(Clone, Debug)]
pub struct Config {
    /// State directory, `service/data` by default.
    pub data_dir: PathBuf,
    /// The Python service's state, imported once when the new store is empty.
    pub legacy_records: PathBuf,
    /// `service/assets/empty.figcraft`.
    pub empty_template: PathBuf,
    /// `server/data/server.key`.
    pub key_file: PathBuf,
    /// `server/figcraft/target/release/figcraft-cli`.
    pub figcraft_cli: PathBuf,
    /// `herdr` or an override.
    pub herdr_bin: PathBuf,
    /// Public `host:port` written into join links.
    pub public_server: String,
    /// Local server URL used while publishing, before the link is rewritten.
    pub local_server: String,
    /// `host:port` for the TCP API.
    pub bind: String,
    /// `skills/mindmap/SKILL.md`, quoted in the agent prompt.
    pub skill_file: PathBuf,
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

        let data_dir =
            env_path("MINDMAP_DATA").unwrap_or_else(|| root.join("service").join("data"));
        let legacy_records = env_path("MINDMAP_LEGACY_DATA")
            .unwrap_or_else(|| root.join("project").join("data"))
            .join("projects.json");
        let figcraft_cli = env_path("MINDMAP_FIGCRAFT_CLI")
            .unwrap_or_else(|| root.join("server/figcraft/target/release/figcraft-cli"));
        let herdr_bin = env_path("MINDMAP_HERDR").unwrap_or_else(|| PathBuf::from("herdr"));
        let public_server = env_string("MINDMAP_SERVER").unwrap_or_else(|| DEFAULT_SERVER.into());
        let local_server =
            env_string("MINDMAP_LOCAL_SERVER").unwrap_or_else(|| DEFAULT_LOCAL_SERVER.into());
        let bind = env_string("MINDMAP_BIND").unwrap_or_else(|| DEFAULT_BIND.into());

        Ok(Self {
            empty_template: root.join("service").join("assets").join("empty.figcraft"),
            key_file: root.join("server").join("data").join("server.key"),
            skill_file: root.join("skills").join("mindmap").join("SKILL.md"),
            data_dir,
            legacy_records,
            figcraft_cli,
            herdr_bin,
            public_server,
            local_server,
            bind,
        })
    }

    /// The `collab.publish` server URL, with the server key in its query string.
    pub fn publish_server_url(&self, key: &str) -> String {
        let base = self.local_server.trim_end_matches('?');
        let separator = if base.contains('?') { '&' } else { '?' };
        format!("{base}{separator}key={}", crate::link::encode(key))
    }
}

fn env_string(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            data_dir: PathBuf::from("/repo/service/data"),
            legacy_records: PathBuf::from("/repo/project/data/projects.json"),
            empty_template: PathBuf::from("/repo/service/assets/empty.figcraft"),
            key_file: PathBuf::from("/repo/server/data/server.key"),
            figcraft_cli: PathBuf::from("/repo/server/figcraft/target/release/figcraft-cli"),
            herdr_bin: PathBuf::from("herdr"),
            public_server: DEFAULT_SERVER.into(),
            local_server: DEFAULT_LOCAL_SERVER.into(),
            bind: DEFAULT_BIND.into(),
            skill_file: PathBuf::from("/repo/skills/mindmap/SKILL.md"),
        }
    }

    #[test]
    fn publish_url_has_one_query_separator() {
        assert_eq!(
            config().publish_server_url("abc_-"),
            "ws://127.0.0.1:47910/?key=abc_-"
        );
    }

    #[test]
    fn publish_url_keeps_an_existing_query() {
        let mut config = config();
        config.local_server = "ws://example.test:1/?x=1".into();
        assert_eq!(
            config.publish_server_url("abc"),
            "ws://example.test:1/?x=1&key=abc"
        );
    }
}
