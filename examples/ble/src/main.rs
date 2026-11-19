//! BLE NUS peripheral example.
//!
//! Advertises `ESP32-C6-N16` with the Nordic UART Service. Connect with any
//! NUS client (for example nRF Connect) and write to the RX characteristic;
//! the device answers on TX notifications.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use esp32c6_n16_ble as ble;
use esp32c6_n16_bsp::{Board, color};

use defmt_rtt as _;
use esp_backtrace as _;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_hal::main]
async fn main(_spawner: Spawner) -> ! {
    let board = esp32c6_n16_runtime::init("ble");

    let Board { mut led, bt, .. } = board;
    let _ = led.set(color::PURPLE);
    info!("ble: starting NUS peripheral");
    ble::run_peripheral(bt).await
}
