use std::process::Command;

fn main() {
    let version = get_version();
    println!("cargo:rustc-env=NANOPROXY_VERSION={}", version);
}

fn get_version() -> String {
    if let Ok(output) = Command::new("git")
        .args(&["describe", "--tags", "--always", "--dirty"])
        .output()
    {
        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !version.is_empty() {
                return version;
            }
        }
    }

    if let Ok(output) = Command::new("git")
        .args(&["rev-parse", "--short", "HEAD"])
        .output()
    {
        if output.status.success() {
            let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !commit.is_empty() {
                return commit;
            }
        }
    }

    "unknown".to_string()
}
