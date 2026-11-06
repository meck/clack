//! Finds the boards on the USB serial ports.

use anyhow::{Context, Result};
use relay_scpi::identity::{IDN_MODEL, USB_PID, USB_PRODUCT, USB_VID};
use serialport::{SerialPortType, UsbPortInfo};

use crate::board::{Board, Idn, SerialTransport};

/// A serial port that has the USB ID and the product string of the board.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub port: String,
    /// The USB serial number. The firmware sets it to the chip ID, the same
    /// value as in the `*IDN?` reply.
    pub serial: Option<String>,
}

/// Gives the serial ports that have the USB ID and the product string of
/// the board. The function does not open the ports.
pub fn candidates() -> Result<Vec<Candidate>> {
    let ports = serialport::available_ports().context("cannot list the serial ports")?;
    let mut found: Vec<Candidate> = ports
        .into_iter()
        .filter(|p| !skip_port(&p.port_name))
        .filter_map(|p| match p.port_type {
            SerialPortType::UsbPort(usb) if is_board(&usb) => Some(Candidate {
                port: p.port_name,
                serial: usb.serial_number.map(|s| s.to_ascii_uppercase()),
            }),
            _ => None,
        })
        .collect();
    found.sort_by(|a, b| a.port.cmp(&b.port));
    found.dedup_by(|a, b| a.port == b.port);
    Ok(found)
}

/// Other RP2350 programs that use the Pico SDK have the same USB ID, thus
/// the product string is also necessary.
fn is_board(usb: &UsbPortInfo) -> bool {
    // udev can give the product string with `_` in place of each space.
    usb.vid == USB_VID
        && usb.pid == USB_PID
        && usb
            .product
            .as_deref()
            .is_some_and(|p| p.replace('_', " ") == USB_PRODUCT)
}

/// macOS gives two names for each port. Use only the `cu` name, because
/// the `tty` name waits for a carrier signal.
fn skip_port(name: &str) -> bool {
    cfg!(target_os = "macos") && name.starts_with("/dev/tty.")
}

/// Opens a port and makes sure that a board with the firmware replies.
pub fn open(port: &str) -> Result<(Board<SerialTransport>, Idn)> {
    let mut board = Board::new(SerialTransport::open(port)?);
    let idn = board
        .sync()
        .and_then(|()| board.idn())
        .with_context(|| format!("{port} does not reply as a {IDN_MODEL}"))?;
    Ok((board, idn))
}
