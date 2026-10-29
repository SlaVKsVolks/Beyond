//! Fresh executable: the first expression in main observes runtime startup.
#![forbid(unsafe_code)]

#[global_allocator]
static ALLOC: mod_alloc::ModAlloc = mod_alloc::ModAlloc::new();

fn main() -> Result<(), mod_alloc::AccountingError> {
    let startup = ALLOC.try_snapshot()?;
    // If this target's runtime makes no pre-main allocation, this is a missing
    // native evidence layer, NOT permission to fabricate an unsafe constructor.
    assert!(
        startup.alloc_count > 0,
        "no pre-main allocation observed on this target"
    );
    println!(
        "pre_main_alloc_count={} pre_main_current_bytes={}",
        startup.alloc_count, startup.current_bytes
    );
    Ok(())
}
