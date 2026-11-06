//! SCPI command parser and instrument state for an 8-channel relay board.
//!
//! This crate has no hardware code. The firmware gives it the input bytes
//! and applies the effects to the relays, the LED and the buzzer.

#![cfg_attr(not(test), no_std)]

pub mod chanlist;
pub mod error;
pub mod identity;
pub mod instrument;
pub mod line;
pub mod parser;

pub use error::ScpiError;
pub use instrument::{Effects, Instrument};
pub use line::{LineBuffer, Push};
