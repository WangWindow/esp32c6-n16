//! Shared startup sequence for the main application and the examples.
//!
//! Keeping this in a crate avoids copy-pasting the delicate ordering:
//! HAL init -> logging -> heap -> RTOS -> board devices. The radio drivers
//! must not be initialized before `esp_rtos::start`.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use defmt::error;
use esp_hal::{clock::CpuClock, ram, timer::timg::TimerGroup};
use esp32c6_n16_bsp::Board;

const INTERNAL_HEAP_BYTES: usize = 64 * 1024;

/// Full `.dram2_uninit` region, reclaimed from the bootloader for the heap.
const DRAM2_UNINIT_SIZE: usize = {
    let range: core::ops::Range<usize> = esp_metadata_generated::memory_range!("DRAM2_UNINIT");
    range.end - range.start
};

/// Initialize the chip, logging, heap, RTOS and board devices.
///
/// Call this from an async `#[esp_hal::main]` entry point. The app name is
/// shown in the boot banner.
pub fn init(app: &'static str) -> Board {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp32c6_n16_log::init();

    // `esp-radio` needs a heap before Wi-Fi/BLE controllers are created.
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: DRAM2_UNINIT_SIZE);
    esp_alloc::heap_allocator!(size: INTERNAL_HEAP_BYTES);
    esp32c6_n16_log::radio_banner(INTERNAL_HEAP_BYTES);

    // The scheduler must be running before esp-radio is initialized.
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let board = match Board::new(
        peripherals.RMT,
        peripherals.GPIO8,
        peripherals.GPIO9,
        peripherals.TSENS,
        peripherals.WIFI,
        peripherals.BT,
    ) {
        Ok(board) => board,
        Err(err) => {
            error!("board initialization failed: {:?}", err);
            panic!("board initialization failed");
        }
    };

    esp32c6_n16_log::boot_banner(&board.info, app, env!("GIT_HASH"), env!("BUILD_PROFILE"));

    board
}
