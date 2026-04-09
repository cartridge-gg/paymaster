use std::process::Command;

fn main() {
    // Git SHA
    let git_sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_SHA={git_sha}");

    // Build timestamp
    let build_timestamp = Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_TIMESTAMP={build_timestamp}");

    // Dev build suffix: "-dev" if the working tree is dirty or HEAD is not on a tag
    let dirty = Command::new("git")
        .args(["diff", "--quiet"])
        .status()
        .map(|s| !s.success())
        .unwrap_or(true);

    let on_tag = Command::new("git")
        .args(["describe", "--exact-match", "--tags", "HEAD"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let dev_suffix = if dirty || !on_tag { "-dev" } else { "" };
    println!("cargo:rustc-env=DEV_BUILD_SUFFIX={dev_suffix}");

    // Rerun if git state changes
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
}
