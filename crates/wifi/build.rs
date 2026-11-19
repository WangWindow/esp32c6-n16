fn main() {
    // `option_env!` in lib.rs is evaluated when this crate is compiled. Make
    // sure changing credentials in `.env` invalidates the cached artifact.
    println!("cargo:rerun-if-env-changed=WIFI_SSID");
    println!("cargo:rerun-if-env-changed=WIFI_PASSWORD");
}
