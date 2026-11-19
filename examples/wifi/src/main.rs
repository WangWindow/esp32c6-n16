//! Wi-Fi station example with a tiny HTTP status endpoint.
//!
//! Connects to the AP configured at build time, acquires an IPv4 address via
//! DHCP, reconnects automatically, and serves:
//!
//! ```text
//! curl http://<dhcp-ip>/
//! {"temperature_c":36,"rssi":-48,"uptime_s":42}
//! ```

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![no_main]

use core::sync::atomic::{AtomicI32, Ordering};

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_futures::join::join4;
use embassy_time::{Duration, Timer};
use esp32c6_n16_bsp::{Board, color};
use esp32c6_n16_wifi::{Credentials, LinkState, Status, WifiService};

use defmt_rtt as _;
use esp_backtrace as _;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_hal::main]
async fn main(spawner: Spawner) -> ! {
    let board = esp32c6_n16_runtime::init("wifi");
    run(board, spawner).await
}

/// Connect to the AP configured at build time and serve status.
#[allow(unreachable_code)]
async fn run(board: Board, _spawner: Spawner) -> ! {
    let Board {
        mut led,
        temperature,
        wifi,
        ..
    } = board;

    let credentials = Credentials::from_env();
    match credentials {
        Some(credentials) => info!("wifi: using SSID {}", credentials.ssid()),
        None => warn!("wifi: WIFI_SSID/WIFI_PASSWORD not set at build time; scan-only mode"),
    }

    let mut wifi = match WifiService::new(wifi, credentials) {
        Ok(wifi) => wifi,
        Err(err) => {
            info!("wifi: controller init failed: {:?}", err);
            let _ = led.set(color::RED);
            loop {
                Timer::after(Duration::from_secs(60)).await;
            }
        }
    };

    let Some((stack, mut runner)) = wifi.start_network() else {
        let _ = led.set(color::RED);
        loop {
            Timer::after(Duration::from_secs(60)).await;
        }
    };

    let rssi = AtomicI32::new(0);
    let state_led = {
        let rssi = &rssi;
        move |state| {
            let color = match state {
                LinkState::Connecting => color::BLUE,
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

    info!("wifi: serving status on port 80");
    join4(
        wifi.connection_loop(state_led),
        runner.run(),
        esp32c6_n16_wifi::monitor(stack),
        esp32c6_n16_wifi::serve_status(stack, status),
    )
    .await;

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
