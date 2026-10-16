use std::process::Command;
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
    for item in ["HEAD", "index"] {
        if let Some(p) = git(&["rev-parse", "--git-path", item]) {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(p) = git(&["rev-parse", "--git-path", &branch]) {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    let revision = git(&["rev-parse", "HEAD"])
        .map(|r| {
            if git(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|s| !s.is_empty())
            {
                format!("{r}-dirty")
            } else {
                r
            }
        })
        .unwrap_or_else(|| "unavailable".into());
    println!("cargo:rustc-env=ROCC_BUILD_REVISION={revision}");
}
