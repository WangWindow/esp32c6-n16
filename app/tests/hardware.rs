//! On-target hardware tests for the ESP32-C6-N16 board.
//!
//! Run with:
//!
//! ```text
//! cargo test -p esp32c6-n16-app --release --test hardware
//! ```
//!
//! `probe-rs` resets the chip between tests, so every test starts from a clean
//! `esp_hal::init`.

#![no_std]
#![no_main]

extern crate alloc;

use defmt_rtt as _;
use esp_hal::clock::CpuClock;
use esp32c6_n16_bsp::Board;

esp_bootloader_esp_idf::esp_app_desc!();

#[embedded_test::tests]
mod tests {
    use super::*;
    use alloc::vec;

    struct Context {
        board: Board,
    }

    #[init]
    fn init() -> Context {
        let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
        esp_alloc::heap_allocator!(size: 32 * 1024);

        let board = Board::new(
            peripherals.RMT,
            peripherals.GPIO8,
            peripherals.GPIO9,
            peripherals.TSENS,
            peripherals.WIFI,
            peripherals.BT,
        )
        .expect("board init");

        Context { board }
    }

    #[test]
    fn chip_identity(ctx: Context) {
        assert_eq!(ctx.board.info.chip, "esp32c6");
        assert_ne!(ctx.board.info.mac, [0; 6]);
    }

    #[test]
    fn heap_allocation(_ctx: Context) {
        let probe = vec![0x5Au8; 4 * 1024];
        assert_eq!(probe.len(), 4 * 1024);
    }

    #[test]
    fn temperature_is_plausible(ctx: Context) {
        assert!(ctx.board.temperature.is_plausible());
    }

    #[test]
    fn button_reads(_ctx: Context) {
        // Reading the pin must not panic; the pressed state depends on the user.
        let _ = _ctx.board.button.is_pressed();
    }

    #[test]
    fn rgb_led_writes(mut ctx: Context) {
        ctx.board
            .led
            .set(esp32c6_n16_bsp::color::GREEN)
            .expect("led write");
        ctx.board.led.off().expect("led off");
    }
}
