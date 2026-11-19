//! Main application: Wi-Fi station + BLE NUS + HTTP status.
//!
//! This is the application to develop, not an example. It uses the shared
//! framework crates and can be modified freely. The example programs live in
//! `examples/` and have their own binaries.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![no_main]

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU16, Ordering};

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_futures::join::{join, join5};
use embassy_net::Stack;
use embassy_time::{Duration, Timer};
use esp32c6_n16_ble as ble;
use esp32c6_n16_bsp::{Board, color};
use esp32c6_n16_wifi::{Credentials, LinkState, Status, WifiService, fetch_weather};

// The defmt RTT logger and panic handler must be linked into the final image.
use defmt_rtt as _;
use esp_backtrace as _;

// Required by probe-rs to recognize the ELF as an ESP-IDF application image.
esp_bootloader_esp_idf::esp_app_desc!();

/// Weather location used by the public Open-Meteo API. Override at build time:
/// `WEATHER_LAT=... WEATHER_LON=... cargo run --release`.
const WEATHER_LAT: &str = match option_env!("WEATHER_LAT") {
    Some(value) => value,
    None => "31.82",
};
const WEATHER_LON: &str = match option_env!("WEATHER_LON") {
    Some(value) => value,
    None => "117.23",
};

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
            let weather_temperature = AtomicI32::new(0);
            let weather_code = AtomicU8::new(0);
            let weather_time = AtomicU16::new(0);
            let weather_valid = AtomicBool::new(false);
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
                weather_temperature_c: weather_valid
                    .load(Ordering::Relaxed)
                    .then(|| (weather_temperature.load(Ordering::Relaxed) / 10) as i16),
                weather_code: weather_valid
                    .load(Ordering::Relaxed)
                    .then(|| weather_code.load(Ordering::Relaxed)),
                local_time: weather_valid.load(Ordering::Relaxed).then(|| {
                    let minutes = weather_time.load(Ordering::Relaxed);
                    ((minutes / 60) as u8, (minutes % 60) as u8)
                }),
            };

            let wifi_tasks = join5(
                wifi.connection_loop(state_led),
                runner.run(),
                esp32c6_n16_wifi::monitor(stack),
                esp32c6_n16_wifi::serve_status(stack, status),
                weather_loop(
                    stack,
                    &weather_temperature,
                    &weather_code,
                    &weather_time,
                    &weather_valid,
                ),
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

async fn weather_loop(
    stack: Stack<'static>,
    temperature: &AtomicI32,
    code: &AtomicU8,
    time: &AtomicU16,
    valid: &AtomicBool,
) -> ! {
    stack.wait_config_up().await;

    loop {
        match fetch_weather(stack, WEATHER_LAT, WEATHER_LON).await {
            Ok(weather) => {
                let tenths = (weather.temperature_c * 10.0) as i32;
                temperature.store(tenths, Ordering::Relaxed);
                code.store(weather.code, Ordering::Relaxed);
                if let Some((hour, minute)) = weather.hour_minute() {
                    time.store(u16::from(hour) * 60 + u16::from(minute), Ordering::Relaxed);
                }
                valid.store(true, Ordering::Relaxed);
                info!(
                    "weather: {} C, {} ({}), local {}",
                    weather.temperature_c,
                    weather.description(),
                    weather.code,
                    weather.local_time()
                );
                Timer::after(Duration::from_secs(15 * 60)).await;
            }
            Err(err) => {
                warn!("weather: fetch failed: {:?}", err);
                Timer::after(Duration::from_secs(30)).await;
            }
        }
    }
}
