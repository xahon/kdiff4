use std::process::Command;


fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(text.trim().to_string())
}


fn rasterize_application_icon() {
    const SOURCE: &str = "../../res/icons/kdiff4.svg";
    println!("cargo:rerun-if-changed={SOURCE}");
    let svg = std::fs::read(SOURCE).expect("Build: could not read the application icon");
    let image = egui_extras::image::load_svg_bytes(&svg, &Default::default()).expect("Build: could not rasterize the application icon");
    assert_eq!(image.size, [64, 64], "Build: the application icon must be 64 by 64 pixels");
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|pixel| pixel.to_srgba_unmultiplied()).collect();
    let out_dir = std::env::var_os("OUT_DIR").expect("Build: Cargo did not supply OUT_DIR");
    std::fs::write(std::path::Path::new(&out_dir).join("kdiff4.rgba"), rgba).expect("Build: could not write the application icon raster");
}


fn main() {
    rasterize_application_icon();
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
