//! The eight relay outputs.

use embassy_rp::gpio::{Level, Output};

/// The relay outputs, CH1 first. A high output closes the relay.
pub struct Relays {
    pins: [Output<'static>; 8],
}

impl Relays {
    pub fn new(pins: [Output<'static>; 8]) -> Self {
        Self { pins }
    }

    /// Sets all relays from a mask. Bit 0 is CH1. A set bit closes the
    /// relay. All eight pins change in one pass, with no delay.
    pub fn apply(&mut self, mask: u8) {
        for (i, pin) in self.pins.iter_mut().enumerate() {
            let level = if mask & (1 << i) != 0 {
                Level::High
            } else {
                Level::Low
            };
            pin.set_level(level);
        }
    }
}
