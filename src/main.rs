#![no_std]
#![no_main]

use defmt::info;
use defmt_rtt as _;
use esp_backtrace as _;
use esp_hal::{
    efuse,
    gpio::{Input, InputConfig, Pull},
    main,
    rmt::Rmt,
    time::Rate,
    tsens::{Config as TsensConfig, TemperatureSensor},
};
use esp_hal_smartled::{RmtSmartLeds, WS2812B_TIMING, buffer_size, color_order::Grb};
use smart_leds::{RGB8, SmartLedsWrite};

defmt::timestamp!("{=u64:us}", 0);

// Required by probe-rs to recognize the ELF as an ESP-IDF application image.
esp_bootloader_esp_idf::esp_app_desc!();

/// Onboard test firmware for the ESP32-C6-N16 board.
///
/// Reports chip identity and drives the onboard WS2812 RGB LED (GPIO8).
#[main]
fn main() -> ! {
    info!("boot: before esp_hal::init");
    let peripherals = esp_hal::init(esp_hal::Config::default());
    info!("boot: after esp_hal::init");

    info!("ESP32-C6-N16 board test");
    info!("Chip: {}", esp_hal::chip!());

    let rev = efuse::chip_revision();
    info!("Chip revision: {}.{}", rev.major, rev.minor);

    let mac = efuse::base_mac_address();
    let m = mac.as_bytes();
    info!(
        "Base MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        m[0], m[1], m[2], m[3], m[4], m[5]
    );

    let reason = esp_hal::system::reset_reason();
    info!(
        "Reset reason code: 0x{:02x}",
        reason.map(|r| r as u8).unwrap_or(0)
    );

    let tsens = TemperatureSensor::new(peripherals.TSENS, TsensConfig::default()).unwrap();
    esp_hal::rom::ets_delay_us(200);
    let temp = tsens.get_temperature();
    info!(
        "Temperature: {} C (raw {})",
        temp.to_celsius() as i32,
        temp.raw_value
    );

    let boot = Input::new(
        peripherals.GPIO9,
        InputConfig::default().with_pull(Pull::Up),
    );
    info!("BOOT button pressed: {}", boot.is_low());

    // Onboard addressable RGB LED on GPIO8.
    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80)).unwrap();
    let mut led = RmtSmartLeds::<{ buffer_size::<RGB8>(1) }, _, RGB8, Grb>::new(
        WS2812B_TIMING,
        rmt.channel0,
        peripherals.GPIO8,
        Rate::from_mhz(80),
    )
    .unwrap();

    let colors = [
        RGB8::new(32, 0, 0),
        RGB8::new(0, 32, 0),
        RGB8::new(0, 0, 32),
        RGB8::new(32, 32, 32),
    ];

    loop {
        for color in colors {
            led.write([color].into_iter()).unwrap();
            esp_hal::rom::ets_delay_us(500_000);
        }

        let temp = tsens.get_temperature();
        info!(
            "alive: temp {} C, BOOT pressed: {}",
            temp.to_celsius() as i32,
            boot.is_low()
        );
    }
}
