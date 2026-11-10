use std::process::Command;


fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(text.trim().to_string())
}


fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for name in ["HEAD", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let commit = git(&["rev-parse", "--verify", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=KDIFF4_BUILD_COMMIT={commit}");
    for (source, destination) in [("TARGET", "KDIFF4_BUILD_TARGET"), ("PROFILE", "KDIFF4_BUILD_PROFILE")] {
        let value = std::env::var(source).expect("Build: Cargo did not supply build metadata");
        println!("cargo:rustc-env={destination}={value}");
    }
    let mut features: Vec<_> = std::env::vars()
        .filter_map(|(key, _)| key.strip_prefix("CARGO_FEATURE_").map(|feature| feature.to_lowercase().replace('_', "-")))
        .collect();
    features.sort();
    let features = if features.is_empty() { "None".to_string() } else { features.join(", ") };
    println!("cargo:rustc-env=KDIFF4_BUILD_FEATURES={features}");
}
