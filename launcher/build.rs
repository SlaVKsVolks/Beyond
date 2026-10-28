//! Embeds Horizon's icon (`packaging/icons/horizon.ico`) in the Windows executables, as icon
//! resource 1: Explorer and the task bar show it, and the window takes it as its icon.
#![cfg_attr(windows, forbid(unsafe_code))]
fn main() {
    // Tests compile their isolated console fixture for this same native Cargo host.
    for (name, key) in [
        ("HORIZON_BUILD_HOST", "HOST"),
        ("HORIZON_BUILD_TARGET", "TARGET"),
    ] {
        println!("cargo:rustc-env={name}={}", std::env::var(key).unwrap());
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let ico = format!("{dir}/../packaging/icons/horizon.ico");
    println!("cargo:rerun-if-changed={ico}");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(&ico);
        if let Err(e) = resource.compile() {
            panic!("embedding {ico}: {e}");
        }
    }
}
