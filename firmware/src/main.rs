//! SCPI relay controller for the Waveshare Pico-Relay-B with a Raspberry Pi
//! Pico 2 (RP2350).
//!
//! The host sends SCPI commands over USB CDC-ACM. See `relay-scpi` for the
//! command set.

#![no_std]
#![no_main]

mod beeper;
mod led;
mod relays;
mod usb;

use core::fmt::Write;

use defmt::{info, unwrap, warn};
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0, USB, WATCHDOG};
use embassy_rp::watchdog::Watchdog;
use embassy_rp::{Peri, bind_interrupts, dma, otp, pac, pio};
use embassy_time::{Duration, Ticker};
use heapless::String;
use relay_scpi::Instrument;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use crate::relays::Relays;

/// The firmware version. It goes into the `*IDN?` reply.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The watchdog resets the chip if the watchdog task does not run for this
/// time. After the reset, all relays are open.
const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(1);
const WATCHDOG_FEED: Duration = Duration::from_millis(250);

// The boot ROM of the RP2350 needs an IMAGE_DEF block to start the image.
// embassy-rp supplies it (secure EXE). Do not add a second block here.

/// Program data for `picotool info`.
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 3] = [
    embassy_rp::binary_info::rp_program_name!(c"clack"),
    embassy_rp::binary_info::rp_program_description!(
        c"SCPI relay controller for the Waveshare Pico-Relay-B"
    ),
    embassy_rp::binary_info::rp_cargo_version!(),
];

bind_interrupts!(pub struct Irqs {
    USBCTRL_IRQ => embassy_rp::usb::InterruptHandler<USB>;
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
});

static SERIAL: StaticCell<String<16>> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(embassy_rp::config::Config::default());
    info!("clack {} start", VERSION);

    // All relays are open (low) before the USB connection starts.
    let relays = Relays::new([
        Output::new(p.PIN_21, Level::Low),
        Output::new(p.PIN_20, Level::Low),
        Output::new(p.PIN_19, Level::Low),
        Output::new(p.PIN_18, Level::Low),
        Output::new(p.PIN_17, Level::Low),
        Output::new(p.PIN_16, Level::Low),
        Output::new(p.PIN_15, Level::Low),
        Output::new(p.PIN_14, Level::Low),
    ]);

    spawner.spawn(unwrap!(watchdog_task(p.WATCHDOG)));
    spawner.spawn(unwrap!(led::led_task(p.PIO0, p.DMA_CH0, p.PIN_13)));
    spawner.spawn(unwrap!(beeper::beeper_task(p.PWM_SLICE3, p.PIN_6)));

    let serial: &'static str = SERIAL.init(chip_serial());
    info!("serial number {}", serial);
    let instrument = Instrument::new(serial, VERSION);
    usb::start(&spawner, p.USB, serial, relays, instrument);
}

/// Gives the chip ID as 16 hexadecimal digits. If the OTP read fails, it
/// gives zeros.
fn chip_serial() -> String<16> {
    let id = otp::get_chipid().unwrap_or_else(|_| {
        warn!("cannot read the chip ID from OTP");
        0
    });
    let mut s = String::new();
    // 16 hexadecimal digits fill the string exactly.
    let _ = write!(s, "{id:016X}");
    s
}

#[embassy_executor::task]
async fn watchdog_task(wd: Peri<'static, WATCHDOG>) {
    let mut wd = Watchdog::new(wd);
    // Do not reset the chip while the debug probe stops the core.
    wd.pause_on_debug(true);
    wd.start(WATCHDOG_TIMEOUT);
    // embassy-rp 0.10 writes the RP2040 bit mask to PSM.WDSEL. On the
    // RP2350 the bits have other positions, thus that mask does not select
    // SRAM7 to SRAM9, XIP, SIO, ACCESSCTRL, PROC0 and PROC1. Select all
    // blocks except the two oscillators, the same as the Pico SDK.
    let mut select = pac::psm::regs::Wdsel(0x01ff_ffff);
    select.set_rosc(false);
    select.set_xosc(false);
    pac::PSM.wdsel().write_value(select);
    let mut ticker = Ticker::every(WATCHDOG_FEED);
    loop {
        ticker.next().await;
        wd.feed(WATCHDOG_TIMEOUT);
    }
}
