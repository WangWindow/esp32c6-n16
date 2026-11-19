//! Board support package for the ESP32-C6-N16 minimal development board.
//!
//! The board exposes:
//!
//! - a WS2812 addressable RGB LED on GPIO8, driven through RMT,
//! - a BOOT button on GPIO9 (active low, pulled up),
//! - an on-chip temperature sensor.
//!
//! This crate deliberately does not call [`esp_hal::init`] or take ownership of
//! the runtime. The main application initializes the HAL, the heap and
//! `esp-rtos`, then builds a [`Board`] from the remaining peripherals.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use esp_hal::{
    Blocking, efuse,
    gpio::{Input, InputConfig, Pull},
    peripherals::{BT, GPIO8, GPIO9, RMT, TSENS, WIFI},
    rmt::Rmt,
    rtc_cntl::SocResetReason,
    time::Rate,
    tsens::{Config as TemperatureConfig, TemperatureSensor},
};
use esp_hal_smartled::{RmtSmartLeds, WS2812B_TIMING, buffer_size, color_order::Grb};
use smart_leds::{RGB8, SmartLedsWrite};

/// Errors that can occur while bringing up on-board devices.
#[derive(Debug)]
#[non_exhaustive]
pub enum BoardError {
    /// Failed to configure the RMT channel.
    RmtConfig(esp_hal::rmt::ConfigError),
    /// Failed to initialize the smart LED adapter.
    LedInit(esp_hal_smartled::Error),
    /// Failed to write to the smart LED.
    LedWrite(esp_hal_smartled::AdapterError),
    /// Failed to initialize the temperature sensor.
    Temperature(esp_hal::tsens::ConfigError),
}

impl defmt::Format for BoardError {
    fn format(&self, fmt: defmt::Formatter<'_>) {
        match self {
            Self::RmtConfig(err) => {
                defmt::write!(fmt, "RmtConfig({:?})", defmt::Debug2Format(err))
            }
            Self::LedInit(err) => defmt::write!(fmt, "LedInit({:?})", defmt::Debug2Format(err)),
            Self::LedWrite(err) => defmt::write!(fmt, "LedWrite({:?})", defmt::Debug2Format(err)),
            Self::Temperature(err) => {
                defmt::write!(fmt, "Temperature({:?})", defmt::Debug2Format(err))
            }
        }
    }
}

impl From<esp_hal::rmt::ConfigError> for BoardError {
    fn from(err: esp_hal::rmt::ConfigError) -> Self {
        Self::RmtConfig(err)
    }
}

impl From<esp_hal_smartled::Error> for BoardError {
    fn from(err: esp_hal_smartled::Error) -> Self {
        Self::LedInit(err)
    }
}

impl From<esp_hal_smartled::AdapterError> for BoardError {
    fn from(err: esp_hal_smartled::AdapterError) -> Self {
        Self::LedWrite(err)
    }
}

impl From<esp_hal::tsens::ConfigError> for BoardError {
    fn from(err: esp_hal::tsens::ConfigError) -> Self {
        Self::Temperature(err)
    }
}

/// Helpful status colors for the on-board RGB LED.
pub mod color {
    use smart_leds::RGB8;

    /// LED off.
    pub const OFF: RGB8 = RGB8::new(0, 0, 0);
    /// Boot / idle.
    pub const BLUE: RGB8 = RGB8::new(0, 0, 32);
    /// Wi-Fi activity or success.
    pub const GREEN: RGB8 = RGB8::new(0, 32, 0);
    /// BLE activity.
    pub const PURPLE: RGB8 = RGB8::new(24, 0, 32);
    /// Wi-Fi + BLE coexistence.
    pub const CYAN: RGB8 = RGB8::new(0, 24, 24);
    /// Error.
    pub const RED: RGB8 = RGB8::new(32, 0, 0);
    /// Test passed.
    pub const WHITE: RGB8 = RGB8::new(32, 32, 32);
}

/// Static information about the chip and the last reset.
#[derive(Debug, Clone, Copy)]
pub struct BoardInfo {
    /// Chip model name, e.g. `"esp32c6"`.
    pub chip: &'static str,
    /// Major chip revision.
    pub revision_major: u8,
    /// Minor chip revision.
    pub revision_minor: u8,
    /// Base MAC address assigned to the chip.
    pub mac: [u8; 6],
    /// Reason for the previous reset, if it could be determined.
    pub reset_reason: Option<SocResetReason>,
}

impl BoardInfo {
    /// Read chip identity from eFuse and the reset reason from RTC.
    #[must_use]
    pub fn read() -> Self {
        let revision = efuse::chip_revision();
        let mac = efuse::base_mac_address();
        let mut mac_bytes = [0u8; 6];
        mac_bytes.copy_from_slice(mac.as_bytes());

        Self {
            chip: esp_hal::chip!(),
            revision_major: revision.major,
            revision_minor: revision.minor,
            mac: mac_bytes,
            reset_reason: esp_hal::system::reset_reason(),
        }
    }
}

/// On-board WS2812 RGB LED.
pub struct StatusLed {
    led: RmtSmartLeds<'static, { buffer_size::<RGB8>(1) }, Blocking, RGB8, Grb>,
}

impl StatusLed {
    /// Create the LED driver from the RMT peripheral and the GPIO8 pin.
    pub fn new(rmt: RMT<'static>, pin: GPIO8<'static>) -> Result<Self, BoardError> {
        let rmt = Rmt::new(rmt, Rate::from_mhz(80))?;
        let led = RmtSmartLeds::new(WS2812B_TIMING, rmt.channel0, pin, Rate::from_mhz(80))?;

        Ok(Self { led })
    }

    /// Set the LED to a single color.
    pub fn set(&mut self, color: RGB8) -> Result<(), BoardError> {
        self.led.write([color])?;
        Ok(())
    }

    /// Turn the LED off.
    pub fn off(&mut self) -> Result<(), BoardError> {
        self.set(color::OFF)
    }
}

/// On-board BOOT button.
pub struct Button {
    input: Input<'static>,
}

impl Button {
    /// Create the button driver. The pin is configured as input with pull-up.
    #[must_use]
    pub fn new(pin: GPIO9<'static>) -> Self {
        Self {
            input: Input::new(pin, InputConfig::default().with_pull(Pull::Up)),
        }
    }

    /// Returns `true` while the button is held down.
    #[must_use]
    pub fn is_pressed(&self) -> bool {
        self.input.is_low()
    }
}

/// On-chip temperature sensor.
pub struct Temperature {
    sensor: TemperatureSensor<'static>,
}

impl Temperature {
    /// Create the temperature sensor driver.
    pub fn new(tsens: TSENS<'static>) -> Result<Self, BoardError> {
        let sensor = TemperatureSensor::new(tsens, TemperatureConfig::default())?;
        Ok(Self { sensor })
    }

    /// Read the current temperature in degrees Celsius.
    #[must_use]
    pub fn celsius(&self) -> f32 {
        self.sensor.get_temperature().to_celsius()
    }

    /// Read the raw sensor value.
    #[must_use]
    pub fn raw(&self) -> u8 {
        self.sensor.get_temperature().raw_value
    }

    /// A coarse sanity check for the sensor reading.
    #[must_use]
    pub fn is_plausible(&self) -> bool {
        (-20.0..=100.0).contains(&self.celsius())
    }
}

/// All board devices plus the radio peripherals, ready to be handed to an app.
pub struct Board {
    /// Chip and reset information.
    pub info: BoardInfo,
    /// On-board RGB LED.
    pub led: StatusLed,
    /// On-board BOOT button.
    pub button: Button,
    /// On-chip temperature sensor.
    pub temperature: Temperature,
    /// Wi-Fi peripheral, unused until an app creates a controller.
    pub wifi: WIFI<'static>,
    /// BLE peripheral, unused until an app creates a controller.
    pub bt: BT<'static>,
}

impl Board {
    /// Build the board support structure.
    ///
    /// `esp_hal::init`, the heap and `esp_rtos::start` must already have run.
    pub fn new(
        rmt: RMT<'static>,
        led_pin: GPIO8<'static>,
        button_pin: GPIO9<'static>,
        tsens: TSENS<'static>,
        wifi: WIFI<'static>,
        bt: BT<'static>,
    ) -> Result<Self, BoardError> {
        Ok(Self {
            info: BoardInfo::read(),
            led: StatusLed::new(rmt, led_pin)?,
            button: Button::new(button_pin),
            temperature: Temperature::new(tsens)?,
            wifi,
            bt,
        })
    }
}
