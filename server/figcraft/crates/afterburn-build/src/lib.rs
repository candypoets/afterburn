//! A platform-independent source identity shared by full and headless Afterburn builds.
//!
//! The digest covers the locked workspace manifests and app, node and shared-crate source.
//! It deliberately excludes Git metadata: committing an already-built source snapshot does
//! not change its identity, and an exported snapshot builds identically on another machine.

pub const BUILD_ID: &str = env!("AFTERBURN_BUILD_ID");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PROTOCOL: u64 = 1;
