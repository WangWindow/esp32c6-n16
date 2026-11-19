//! Main application: Wi-Fi station + BLE NUS + HTTP status.
//!
//! This is the application to develop, not an example. It uses the shared
//! framework crates and can be modified freely. The example programs live in
//! `examples/` and have their own binaries.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![no_main]

use core::sync::atomic::{AtomicI32, Ordering};

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_futures::join::{join, join4};
use embassy_time::{Duration, Timer};
use esp32c6_n16_ble as ble;
use esp32c6_n16_bsp::{Board, color};
use esp32c6_n16_wifi::{Credentials, LinkState, Status, WifiService};

// The defmt RTT logger and panic handler must be linked into the final image.
use defmt_rtt as _;
use esp_backtrace as _;

// Required by probe-rs to recognize the ELF as an ESP-IDF application image.
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_hal::main]
async fn main(_spawner: Spawner) -> ! {
    let board = esp32c6_n16_runtime::init("app");

    let Board {
        mut led,
        temperature,
        wifi,
        bt,
        ..
    } = board;

    let credentials = Credentials::from_env();
    match credentials {
        Some(credentials) => info!("app: using SSID {}", credentials.ssid()),
        None => warn!(
            "app: WIFI_SSID/WIFI_PASSWORD not set at build time; Wi-Fi runs in scan-only mode"
        ),
    }

    let mut wifi = match WifiService::new(wifi, credentials) {
        Ok(wifi) => wifi,
        Err(err) => {
            info!("app: Wi-Fi controller init failed: {:?}", err);
            let _ = led.set(color::RED);
            loop {
                Timer::after(Duration::from_secs(60)).await;
            }
        }
    };

    info!("app: starting Wi-Fi, BLE NUS and HTTP status");
    let _ = led.set(color::CYAN);

    match wifi.start_network() {
        Some((stack, mut runner)) => {
            let rssi = AtomicI32::new(0);
            let state_led = {
                let rssi = &rssi;
                move |state| {
                    let color = match state {
                        LinkState::Connecting => color::CYAN,
                        LinkState::Connected {
                            rssi: connected_rssi,
                            ..
                        } => {
                            rssi.store(connected_rssi, Ordering::Relaxed);
                            color::GREEN
                        }
                        LinkState::Disconnected | LinkState::Failed => color::RED,
                    };
                    let _ = led.set(color);
                }
            };

            let status = || Status {
                temperature_c: temperature.celsius() as i16,
                rssi: rssi.load(Ordering::Relaxed),
                uptime_s: esp_hal::time::Instant::now()
                    .duration_since_epoch()
                    .as_secs(),
            };

            let wifi_tasks = join4(
                wifi.connection_loop(state_led),
                runner.run(),
                esp32c6_n16_wifi::monitor(stack),
                esp32c6_n16_wifi::serve_status(stack, status),
            );
            join(wifi_tasks, ble::run_peripheral(bt)).await;
        }
        None => {
            join(wifi.connection_loop(|_| {}), ble::run_peripheral(bt)).await;
        }
    }

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
