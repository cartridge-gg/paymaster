use std::fmt::Write;

/// The latest version from Cargo.toml.
const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Suffix indicating if it is a dev build.
///
/// A build is considered a dev build if the working tree is dirty
/// or if the current git revision is not on a tag.
const DEV_BUILD_SUFFIX: &str = env!("DEV_BUILD_SUFFIX");

/// The SHA of the latest commit.
const GIT_SHA: &str = env!("GIT_SHA");

/// The build timestamp.
const BUILD_TIMESTAMP: &str = env!("BUILD_TIMESTAMP");

/// Short version string, e.g. `1.0.0-dev (77d4800)`
pub fn short() -> String {
    format!("{CARGO_PKG_VERSION}{DEV_BUILD_SUFFIX} ({GIT_SHA})")
}

/// Long version string with build metadata.
pub fn long() -> String {
    let mut out = String::new();
    writeln!(out, "{}", short()).unwrap();
    writeln!(out).unwrap();
    write!(out, "built on: {BUILD_TIMESTAMP}").unwrap();
    out
}
