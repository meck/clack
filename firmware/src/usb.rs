//! The USB CDC-ACM interface that carries the SCPI commands.

use defmt::{debug, info, unwrap, warn};
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_rp::Peri;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::Driver;
use embassy_time::{Duration, with_timeout};
use embassy_usb::class::cdc_acm::{CdcAcmClass, Sender, State};
use embassy_usb::{Builder, UsbDevice};
use heapless::String;
use relay_scpi::identity::{USB_MANUFACTURER, USB_PID, USB_PRODUCT, USB_VID};
use relay_scpi::{Instrument, LineBuffer, Push, ScpiError};
use static_cell::StaticCell;

use crate::Irqs;
use crate::beeper::BEEP;
use crate::led::{STATUS, Status};
use crate::relays::Relays;

/// The maximum length of one input line, without the terminator.
const LINE_LEN: usize = 256;
/// The maximum length of the reply to one line, with the terminator.
const REPLY_LEN: usize = 1024;
/// The USB packet size of the CDC-ACM data endpoints.
const PACKET_SIZE: u16 = 64;
/// The current in mA that the device tells the host. With eight closed
/// relays, the board takes approximately 900 mA at 5 V. 500 mA is the
/// maximum value for USB 2.0. Thus the board needs a 5 V supply on P17
/// when many relays are closed.
const MAX_POWER_MA: u16 = 500;
/// The maximum time that the firmware waits for an empty packet buffer.
/// The write of a packet stops when the packet is in the buffer. Thus the
/// timeout occurs when the host did not read the packet before it. Then
/// the firmware stops the reply. A packet that is in the buffer stays
/// there. The host receives it at its next read, also after the port
/// closes and opens again. The clack client discards such old replies.
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);

type UsbDriver = Driver<'static, USB>;

static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
static CDC_STATE: StaticCell<State> = StaticCell::new();

/// Makes the USB device and starts the USB tasks.
pub fn start(
    spawner: &Spawner,
    usb: Peri<'static, USB>,
    serial: &'static str,
    relays: Relays,
    instrument: Instrument,
) {
    let driver = Driver::new(usb, Irqs);

    let mut config = embassy_usb::Config::new(USB_VID, USB_PID);
    config.manufacturer = Some(USB_MANUFACTURER);
    config.product = Some(USB_PRODUCT);
    config.serial_number = Some(serial);
    config.max_power = MAX_POWER_MA;

    let mut builder = Builder::new(
        driver,
        config,
        CONFIG_DESC.init([0; 256]),
        BOS_DESC.init([0; 256]),
        &mut [],
        CONTROL_BUF.init([0; 64]),
    );
    let class = CdcAcmClass::new(&mut builder, CDC_STATE.init(State::new()), PACKET_SIZE);
    let device = builder.build();

    spawner.spawn(unwrap!(usb_device_task(device)));
    spawner.spawn(unwrap!(scpi_task(class, relays, instrument)));
}

#[embassy_executor::task]
async fn usb_device_task(mut device: UsbDevice<'static, UsbDriver>) -> ! {
    device.run().await
}

#[embassy_executor::task]
async fn scpi_task(
    class: CdcAcmClass<'static, UsbDriver>,
    mut relays: Relays,
    mut instrument: Instrument,
) {
    let (mut tx, mut rx, control) = class.split_with_control();
    let mut lines: LineBuffer<LINE_LEN> = LineBuffer::new();
    let mut reply: String<REPLY_LEN> = String::new();
    let mut packet = [0u8; PACKET_SIZE as usize];
    let mut shown: Option<Status> = None;

    loop {
        show_status(&mut shown, false, &instrument);
        rx.wait_connection().await;
        info!("USB configured");

        loop {
            show_status(&mut shown, control.dtr(), &instrument);
            let n = match select(rx.read_packet(&mut packet), control.control_changed()).await {
                Either::First(Ok(n)) => n,
                Either::First(Err(_)) => break,
                Either::Second(()) => continue,
            };

            for &byte in packet.iter().take(n) {
                match lines.push(byte) {
                    Push::Pending => {}
                    Push::Overrun => {
                        warn!("input line is longer than {} bytes", LINE_LEN);
                        instrument.push_error(ScpiError::InputBufferOverrun);
                    }
                    Push::Line => {
                        let line = lines.line();
                        debug!("command {=[u8]:a}", line);
                        let effects = instrument.execute(line, &mut reply);
                        if effects.relays_changed {
                            relays.apply(instrument.relays());
                            info!("relays {=u8:08b}", instrument.relays());
                        }
                        if effects.beep {
                            BEEP.signal(());
                        }
                        if !reply.is_empty()
                            && write_reply(&mut tx, reply.as_bytes()).await.is_err()
                        {
                            warn!("host did not read the reply");
                        }
                    }
                }
            }
        }

        info!("USB disconnected");
        // Discard a part line from the old connection.
        lines = LineBuffer::new();
    }
}

/// Sends the LED status if it changed.
fn show_status(shown: &mut Option<Status>, dtr: bool, instrument: &Instrument) {
    let status = if instrument.error_pending() {
        Status::Error
    } else if dtr {
        Status::HostOpen
    } else {
        Status::Idle
    };
    if *shown != Some(status) {
        *shown = Some(status);
        STATUS.signal(status);
    }
}

/// Writes a reply in packets. If the last packet is full, a zero-length
/// packet ends the transfer.
async fn write_reply(tx: &mut Sender<'static, UsbDriver>, data: &[u8]) -> Result<(), ()> {
    let max = tx.max_packet_size() as usize;
    for chunk in data.chunks(max) {
        with_timeout(WRITE_TIMEOUT, tx.write_packet(chunk))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
    }
    if data.len().is_multiple_of(max) {
        with_timeout(WRITE_TIMEOUT, tx.write_packet(&[]))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
    }
    Ok(())
}
