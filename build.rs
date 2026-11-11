fn main() {
    const ICON_RESOURCE: &str = "res/icons/kdiff4.res";
    println!("cargo:rerun-if-changed={ICON_RESOURCE}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").expect("Build: Cargo did not supply CARGO_MANIFEST_DIR");
        let resource = std::path::Path::new(&manifest_dir).join(ICON_RESOURCE);
        println!("cargo:rustc-link-arg-bin=kdiff4={}", resource.display());
    }
}
