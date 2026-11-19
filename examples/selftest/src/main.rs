//! Power-on self-test.
//!
//! Checks the board, heap and both radios without needing network credentials.
//! Results are reported over defmt and with the on-board RGB LED.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![no_main]

extern crate alloc;

use alloc::vec;
use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp32c6_n16_ble as ble;
use esp32c6_n16_bsp::{Board, color};
use esp32c6_n16_testkit::Suite;
use esp32c6_n16_wifi::WifiService;

use defmt_rtt as _;
use esp_backtrace as _;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_hal::main]
async fn main(spawner: Spawner) -> ! {
    let board = esp32c6_n16_runtime::init("selftest");
    run(board, spawner).await
}

/// Run the self-test and keep the board alive afterwards.
async fn run(board: Board, _spawner: Spawner) -> ! {
    let Board {
        info: board_info,
        mut led,
        button,
        temperature,
        wifi,
        bt,
    } = board;

    let mut suite = Suite::new("selftest");

    // --- Board identity ---------------------------------------------------
    suite.check(board_info.revision_major <= 1, "chip revision sane");
    suite.check(board_info.mac != [0; 6], "MAC address present");
    info!(
        "board: {} rev {}.{}, boot button {}",
        board_info.chip,
        board_info.revision_major,
        board_info.revision_minor,
        if button.is_pressed() {
            "pressed"
        } else {
            "released"
        }
    );

    // --- RGB LED ----------------------------------------------------------
    for color in [color::RED, color::GREEN, color::BLUE, color::WHITE] {
        if led.set(color).is_err() {
            suite.check(false, "RGB LED write");
            break;
        }
        Timer::after(Duration::from_millis(120)).await;
    }
    let _ = led.off();

    // --- Temperature ------------------------------------------------------
    let celsius = temperature.celsius();
    info!("temperature: {} C (raw {})", celsius, temperature.raw());
    suite.check(temperature.is_plausible(), "temperature in plausible range");

    // --- Heap -------------------------------------------------------------
    let heap_probe = vec![0xA5u8; 8 * 1024];
    suite.check(heap_probe.len() == 8 * 1024, "8 KiB heap allocation");
    drop(heap_probe);

    // --- Wi-Fi ------------------------------------------------------------
    let wifi_result = WifiService::new(wifi, None);
    if let Some(mut wifi) = suite.check_ok(wifi_result, "Wi-Fi controller init") {
        match wifi.scan().await {
            Ok(aps) => {
                info!("wifi: scan found {} access point(s)", aps.len());
                for ap in aps.iter().take(5) {
                    info!(
                        "wifi: {} rssi={} dBm ch={}",
                        ap.ssid, ap.signal_strength, ap.channel
                    );
                }
                suite.check(true, "Wi-Fi scan");
            }
            Err(err) => {
                suite.check(false, "Wi-Fi scan");
                info!("wifi: scan error {:?}", err);
            }
        }
    }

    // --- BLE --------------------------------------------------------------
    let ble_result = ble::init(bt);
    match ble_result {
        Ok(controller) => {
            info!("ble: controller initialized, address {}", ble::address());
            suite.check(true, "BLE controller init");
            drop(controller);
        }
        Err(_) => {
            suite.check(false, "BLE controller init");
        }
    }

    // --- Summary ----------------------------------------------------------
    let ok = suite.finish();
    let _ = led.set(if ok { color::GREEN } else { color::RED });
    if !ok {
        info!("selftest: FAILED -- inspect the FAIL lines above");
    }

    loop {
        Timer::after(Duration::from_secs(5)).await;
        info!(
            "heartbeat: temp {} C, BOOT {}",
            temperature.celsius(),
            if button.is_pressed() {
                "pressed"
            } else {
                "released"
            }
        );
    }
}
