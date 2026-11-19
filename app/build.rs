fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WEATHER_LAT");
    println!("cargo:rerun-if-env-changed=WEATHER_LON");
    println!("cargo:rerun-if-changed=embedded-test.x");

    // Keep the embedded-test section in integration test binaries only.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    println!("cargo:rustc-link-search=native={manifest_dir}");
    println!("cargo:rustc-link-arg-tests=-Tembedded-test.x");
    println!("cargo:rustc-check-cfg=cfg(rust_analyzer)");
}
