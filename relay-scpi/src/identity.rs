//! The identity of the board on USB and in the `*IDN?` reply.
//!
//! The firmware sends these values. The client uses them to find the
//! boards. Thus both get them from this module.

/// Raspberry Pi vendor ID.
pub const USB_VID: u16 = 0x2E8A;

/// The product ID that the Pico SDK uses for CDC stdio on the RP2350. The
/// Waveshare test firmware for the Pico 2 uses the same ID. On the RP2040,
/// the Pico SDK uses 0x000A. Other programs also use this ID, thus the
/// client also compares the product string.
pub const USB_PID: u16 = 0x0009;

/// The USB manufacturer string.
pub const USB_MANUFACTURER: &str = "Waveshare";

/// The USB product string.
pub const USB_PRODUCT: &str = "Pico-Relay-B SCPI";

/// The manufacturer field of the `*IDN?` reply.
pub const IDN_MANUFACTURER: &str = "Waveshare";

/// The model field of the `*IDN?` reply.
pub const IDN_MODEL: &str = "Pico-Relay-B";
