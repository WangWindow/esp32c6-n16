//! Logging support for the ESP32-C6-N16 firmware.
//!
//! All logs are emitted through `defmt` over RTT. `defmt-rtt` is linked by the
//! final binary; this crate provides the timestamp implementation, a boot
//! banner and a few formatting helpers used across the workspace.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use defmt::{info, warn};
use esp32c6_n16_bsp::BoardInfo;

// Install the defmt timestamp. The timestamp is microseconds since boot.
//
// `Instant::now()` is only reliable after `esp_hal::init`, so log after the
// HAL has been initialized.
defmt::timestamp!(
    "{=u64:us}",
    esp_hal::time::Instant::now()
        .duration_since_epoch()
        .as_micros()
);

/// Log a startup marker. Call after `esp_hal::init` so timestamps are valid.
pub fn init() {
    info!("logger: defmt over RTT, level filter from DEFMT_LOG");
}

/// Print the boot banner with chip, build and reset information.
pub fn boot_banner(info: &BoardInfo, app: &str, git_hash: &str, profile: &str) {
    info!("============================================================");
    info!("ESP32-C6-N16 firmware");
    info!("app: {}", app);
    info!("build: {} ({})", git_hash, profile);
    info!(
        "chip: {} rev {}.{}",
        info.chip, info.revision_major, info.revision_minor
    );
    info!(
        "mac: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        info.mac[0], info.mac[1], info.mac[2], info.mac[3], info.mac[4], info.mac[5]
    );

    match info.reset_reason {
        Some(reason) => info!("reset reason: {:?}", defmt::Debug2Format(&reason)),
        None => warn!("reset reason: unknown"),
    }

    info!("============================================================");
}

/// Log heap and radio configuration values at boot.
pub fn radio_banner(heap_bytes: usize) {
    info!(
        "heap regions: reclaimed DRAM2 + {} bytes internal",
        heap_bytes
    );
    info!("radio: feature set selected by the application");
}
