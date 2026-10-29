//! Embeds Horizon's icon (`packaging/icons/horizon.ico`) in the Windows executables, as icon
//! resource 1: Explorer and the task bar show it, and the window takes it as its icon.
fn main() {
    println!("cargo:rustc-check-cfg=cfg(horizon_bench_count_allocations)");
    println!("cargo:rerun-if-env-changed=HORIZON_BENCH_COUNT_ALLOCATIONS");
    if std::env::var_os("HORIZON_BENCH_COUNT_ALLOCATIONS").is_some() {
        println!("cargo:rustc-cfg=horizon_bench_count_allocations");
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let ico = format!("{dir}/../../packaging/icons/horizon.ico");
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
