//! The WS2812 RGB LED shows the status.

use embassy_rp::Peri;
use embassy_rp::peripherals::{DMA_CH0, PIN_13, PIO0};
use embassy_rp::pio::Pio;
use embassy_rp::pio_programs::ws2812::{PioWs2812, PioWs2812Program};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use smart_leds::RGB8;

use crate::Irqs;

/// The status that the LED shows.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum Status {
    /// No host has the serial port open. Dim green.
    Idle,
    /// A host has the serial port open and sets DTR. Blue.
    HostOpen,
    /// The SCPI error queue is not empty. Red.
    Error,
}

/// Send a new status to the LED task.
pub static STATUS: Signal<CriticalSectionRawMutex, Status> = Signal::new();

fn color(status: Status) -> RGB8 {
    match status {
        Status::Idle => RGB8::new(0, 16, 0),
        Status::HostOpen => RGB8::new(0, 0, 24),
        Status::Error => RGB8::new(24, 0, 0),
    }
}

#[embassy_executor::task]
pub async fn led_task(
    pio: Peri<'static, PIO0>,
    dma: Peri<'static, DMA_CH0>,
    pin: Peri<'static, PIN_13>,
) {
    let Pio {
        mut common, sm0, ..
    } = Pio::new(pio, Irqs);
    let program = PioWs2812Program::new(&mut common);
    let mut led: PioWs2812<'_, PIO0, 0, 1, _> =
        PioWs2812::new(&mut common, sm0, dma, Irqs, pin, &program);

    let mut status = Status::Idle;
    loop {
        led.write(&[color(status)]).await;
        status = STATUS.wait().await;
    }
}
