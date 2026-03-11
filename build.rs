fn main() {
    // Generate a build ID at compile time
    let build_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    println!("cargo:rustc-env=FERRITE_BUILD_ID={}", build_id);

    // Copy memory.x to OUT_DIR so cortex-m-rt's link.x finds it
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    std::fs::copy(
        std::path::Path::new(&manifest_dir).join("memory.x"),
        std::path::Path::new(&out_dir).join("memory.x"),
    )
    .expect("Failed to copy memory.x");
    println!("cargo:rustc-link-search={}", out_dir);
    println!("cargo:rerun-if-changed=memory.x");
}
