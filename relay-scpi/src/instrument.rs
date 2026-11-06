//! The instrument state and the command execution.

use core::fmt::Write;

use heapless::String;

use crate::chanlist::{ChannelList, FIRST_CHANNEL, LAST_CHANNEL};
use crate::error::{ErrorQueue, ScpiError};
use crate::identity::{IDN_MANUFACTURER, IDN_MODEL};
use crate::parser::{Command, Parser, Unit};

/// The SCPI version that `SYSTem:VERSion?` gives.
pub const SCPI_VERSION: &str = "1999.0";

/// The side effects of one line, for the hardware.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Effects {
    /// The relay mask is different after the line.
    pub relays_changed: bool,
    /// Make one beep.
    pub beep: bool,
}

/// The state of the relay board, as SCPI sees it.
pub struct Instrument {
    relays: u8,
    beeper: bool,
    errors: ErrorQueue,
    serial: String<24>,
    version: &'static str,
}

impl Instrument {
    /// Makes an instrument with all relays open and the beeper off.
    ///
    /// `serial` and `version` go into the `*IDN?` reply. If `serial` is
    /// longer than 24 bytes, the instrument uses only the first 24 bytes.
    pub fn new(serial: &str, version: &'static str) -> Self {
        let mut s = String::new();
        for c in serial.chars() {
            if s.push(c).is_err() {
                break;
            }
        }
        Self {
            relays: 0,
            beeper: false,
            errors: ErrorQueue::new(),
            serial: s,
            version,
        }
    }

    /// Gives the relay mask. Bit 0 is channel 1. A set bit is a closed
    /// relay.
    pub fn relays(&self) -> u8 {
        self.relays
    }

    /// Gives `true` if the error queue is not empty.
    pub fn error_pending(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Adds an error to the error queue. Use this for errors that the
    /// transport finds, for example an input line that is too long.
    pub fn push_error(&mut self, err: ScpiError) {
        self.errors.push(err);
    }

    /// Executes one line of SCPI commands.
    ///
    /// The reply goes into `out`. The function clears `out` first. If the
    /// line has one or more queries, a `;` separates the replies,
    /// and a `\n` ends the reply. If the line has no query, `out` stays
    /// empty.
    ///
    /// At the first error, the function adds the error to the queue and
    /// stops. It does not execute the remaining units of the line.
    pub fn execute<const N: usize>(&mut self, line: &[u8], out: &mut String<N>) -> Effects {
        out.clear();
        let before = self.relays;
        let mut effects = Effects::default();

        let mut reply = Reply {
            out,
            started: false,
        };
        match core::str::from_utf8(line) {
            Ok(text) if text.is_ascii() => {
                for unit in Parser::new(text) {
                    let result = unit.and_then(|u| self.run(u, &mut reply, &mut effects));
                    if let Err(err) = result {
                        self.errors.push(err);
                        break;
                    }
                }
            }
            _ => self.errors.push(ScpiError::InvalidCharacter),
        }
        reply.finish();

        effects.relays_changed = self.relays != before;
        if self.beeper && effects.relays_changed {
            effects.beep = true;
        }
        effects
    }

    fn run<const N: usize>(
        &mut self,
        unit: Unit<'_>,
        reply: &mut Reply<'_, N>,
        effects: &mut Effects,
    ) -> Result<(), ScpiError> {
        let p = unit.params;
        match unit.command {
            Command::Identify => {
                no_params(p)?;
                let (serial, version) = (&self.serial, self.version);
                reply.write(|w| write!(w, "{IDN_MANUFACTURER},{IDN_MODEL},{serial},{version}"))
            }
            Command::Reset => {
                no_params(p)?;
                self.relays = 0;
                self.beeper = false;
                Ok(())
            }
            Command::ClearStatus => {
                no_params(p)?;
                self.errors.clear();
                Ok(())
            }
            Command::OperationComplete => {
                no_params(p)?;
                reply.write(|w| w.write_str("1"))
            }
            Command::SelfTest => {
                no_params(p)?;
                reply.write(|w| w.write_str("0"))
            }
            Command::RouteClose => {
                self.relays |= channel_list(p)?.mask();
                Ok(())
            }
            Command::RouteOpen => {
                self.relays &= !channel_list(p)?.mask();
                Ok(())
            }
            Command::RouteCloseQuery => self.route_query(p, reply, true),
            Command::RouteOpenQuery => self.route_query(p, reply, false),
            Command::RouteOpenAll => {
                no_params(p)?;
                self.relays = 0;
                Ok(())
            }
            Command::ErrorQuery => {
                no_params(p)?;
                // Remove the error from the queue only after the reply has
                // it. If the reply does not fit, the error stays first in
                // the queue.
                let err = self.errors.peek();
                reply.write(|w| match err {
                    Some(e) => write!(w, "{},\"{}\"", e.code(), e.text()),
                    None => w.write_str("0,\"No error\""),
                })?;
                self.errors.pop();
                Ok(())
            }
            Command::VersionQuery => {
                no_params(p)?;
                reply.write(|w| w.write_str(SCPI_VERSION))
            }
            Command::Beep => {
                no_params(p)?;
                effects.beep = true;
                Ok(())
            }
            Command::BeepState => {
                self.beeper = boolean(p)?;
                Ok(())
            }
            Command::BeepStateQuery => {
                no_params(p)?;
                let on = self.beeper;
                reply.write(|w| w.write_str(if on { "1" } else { "0" }))
            }
        }
    }

    fn route_query<const N: usize>(
        &self,
        params: &str,
        reply: &mut Reply<'_, N>,
        closed: bool,
    ) -> Result<(), ScpiError> {
        let list = channel_list(params)?;
        let relays = self.relays;
        reply.write(|w| {
            let mut first = true;
            let mut result = Ok(());
            list.for_each(|ch| {
                let is_closed = relays & ch.bit() != 0;
                let value = if is_closed == closed { "1" } else { "0" };
                if !first {
                    result = result.and_then(|()| w.write_char(','));
                }
                first = false;
                result = result.and_then(|()| w.write_str(value));
            });
            result
        })
    }
}

/// Collects the replies of one line into the output buffer.
///
/// It keeps one byte free for the `\n` at the end, thus a reply always has
/// a terminator. If a reply does not fit, it gives `TooMuchData`.
struct Reply<'a, const N: usize> {
    out: &'a mut String<N>,
    /// At least one reply is in the buffer.
    started: bool,
}

impl<const N: usize> Reply<'_, N> {
    fn write(
        &mut self,
        f: impl FnOnce(&mut Bounded<'_, N>) -> core::fmt::Result,
    ) -> Result<(), ScpiError> {
        let mark = self.out.len();
        let mut w = Bounded { out: self.out };
        let mut result = Ok(());
        if self.started {
            result = w.write_char(';');
        }
        result = result.and_then(|()| f(&mut w));
        if result.is_err() {
            // Remove the part of the reply that did not fit.
            self.out.truncate(mark);
            return Err(ScpiError::TooMuchData);
        }
        self.started = true;
        Ok(())
    }

    fn finish(self) {
        if self.started {
            // `Bounded` kept one byte free for this.
            let _ = self.out.push('\n');
        }
    }
}

/// A writer that keeps the last byte of the buffer free.
struct Bounded<'a, const N: usize> {
    out: &'a mut String<N>,
}

impl<const N: usize> Write for Bounded<'_, N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        // Keep one byte free. Thus `s` must be shorter than the free space.
        if s.len() >= N.saturating_sub(self.out.len()) {
            return Err(core::fmt::Error);
        }
        self.out.push_str(s).map_err(|_| core::fmt::Error)
    }
}

fn no_params(params: &str) -> Result<(), ScpiError> {
    if params.is_empty() {
        Ok(())
    } else {
        Err(ScpiError::ParameterNotAllowed)
    }
}

fn channel_list(params: &str) -> Result<ChannelList<'_>, ScpiError> {
    if params.is_empty() {
        return Err(ScpiError::MissingParameter);
    }
    ChannelList::parse(params)
}

fn boolean(params: &str) -> Result<bool, ScpiError> {
    match params {
        "" => Err(ScpiError::MissingParameter),
        "1" => Ok(true),
        "0" => Ok(false),
        p if p.eq_ignore_ascii_case("ON") => Ok(true),
        p if p.eq_ignore_ascii_case("OFF") => Ok(false),
        _ => Err(ScpiError::DataTypeError),
    }
}

// The channel numbers must agree with the 8-bit relay mask.
const _: () = assert!(LAST_CHANNEL - FIRST_CHANNEL + 1 == 8);

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::String as StdString;

    struct Fixture {
        inst: Instrument,
        out: String<128>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                inst: Instrument::new("E66038B713554D2F", "0.1.0"),
                out: String::new(),
            }
        }

        fn run(&mut self, line: &str) -> (StdString, Effects) {
            let e = self.inst.execute(line.as_bytes(), &mut self.out);
            (StdString::from(self.out.as_str()), e)
        }

        fn query(&mut self, line: &str) -> StdString {
            self.run(line).0
        }

        fn err(&mut self) -> StdString {
            self.query("SYST:ERR?")
        }
    }

    #[test]
    fn identify() {
        let mut f = Fixture::new();
        assert_eq!(
            f.query("*IDN?"),
            "Waveshare,Pico-Relay-B,E66038B713554D2F,0.1.0\n"
        );
    }

    #[test]
    fn close_open_and_query() {
        let mut f = Fixture::new();
        let (out, e) = f.run("ROUT:CLOS (@1,3,5:8)");
        assert_eq!(out, "");
        assert!(e.relays_changed);
        assert_eq!(f.inst.relays(), 0b1111_0101);
        assert_eq!(f.query("ROUT:CLOS? (@1:4)"), "1,0,1,0\n");
        assert_eq!(f.query("ROUT:OPEN? (@1:4)"), "0,1,0,1\n");
        f.run("ROUT:OPEN (@5:6)");
        assert_eq!(f.inst.relays(), 0b1100_0101);
        let (_, e) = f.run("ROUT:OPEN (@5:6)");
        assert!(!e.relays_changed);
        f.run("ROUT:OPEN:ALL");
        assert_eq!(f.inst.relays(), 0);
        assert_eq!(f.err(), "0,\"No error\"\n");
    }

    #[test]
    fn several_queries_in_one_line() {
        let mut f = Fixture::new();
        assert_eq!(
            f.query("ROUT:CLOS (@2);CLOS? (@1,2);*OPC?;:SYST:VERS?"),
            "0,1;1;1999.0\n"
        );
    }

    #[test]
    fn reset_and_clear() {
        let mut f = Fixture::new();
        f.run("ROUT:CLOS (@1:8);SYST:BEEP:STAT ON");
        f.run("BOGUS");
        assert!(f.inst.error_pending());
        f.run("*RST");
        assert_eq!(f.inst.relays(), 0);
        assert_eq!(f.query("SYST:BEEP:STAT?"), "0\n");
        // *RST does not clear the error queue.
        assert!(f.inst.error_pending());
        f.run("*CLS");
        assert!(!f.inst.error_pending());
    }

    #[test]
    fn error_queue_order() {
        let mut f = Fixture::new();
        f.run("BOGUS");
        f.run("ROUT:CLOS (@9)");
        f.run("ROUT:CLOS");
        f.run("*RST 1");
        assert_eq!(f.err(), "-113,\"Undefined header\"\n");
        assert_eq!(f.err(), "-222,\"Data out of range\"\n");
        assert_eq!(f.err(), "-109,\"Missing parameter\"\n");
        assert_eq!(f.err(), "-108,\"Parameter not allowed\"\n");
        assert_eq!(f.err(), "0,\"No error\"\n");
    }

    #[test]
    fn error_stops_the_line() {
        let mut f = Fixture::new();
        let (out, _) = f.run("*OPC?;ROUT:CLOS (@9);ROUT:CLOS (@1)");
        assert_eq!(out, "1\n");
        assert_eq!(f.inst.relays(), 0);
        assert_eq!(f.err(), "-222,\"Data out of range\"\n");
    }

    #[test]
    fn beeper() {
        let mut f = Fixture::new();
        let (_, e) = f.run("SYST:BEEP");
        assert!(e.beep);
        let (_, e) = f.run("ROUT:CLOS (@1)");
        assert!(!e.beep);
        f.run("SYST:BEEP:STAT ON");
        assert_eq!(f.query("SYST:BEEP:STAT?"), "1\n");
        let (_, e) = f.run("ROUT:CLOS (@2)");
        assert!(e.beep);
        // No change, thus no beep.
        let (_, e) = f.run("ROUT:CLOS (@2)");
        assert!(!e.beep);
        f.run("SYST:BEEP:STAT off");
        assert_eq!(f.query("SYST:BEEP:STAT?"), "0\n");
        f.run("SYST:BEEP:STAT maybe");
        assert_eq!(f.err(), "-104,\"Data type error\"\n");
    }

    #[test]
    fn non_ascii_input() {
        let mut f = Fixture::new();
        f.inst.execute(&[0xff, b'\n'], &mut f.out);
        assert_eq!(f.err(), "-101,\"Invalid character\"\n");
        f.run("ROUT:CLOS (@1)é");
        assert_eq!(f.err(), "-101,\"Invalid character\"\n");
    }

    #[test]
    fn reply_too_long() {
        let mut inst = Instrument::new("X", "0");
        let mut out: String<8> = String::new();
        inst.execute(b"*OPC?;ROUT:CLOS? (@1:8)", &mut out);
        // The first reply fits. The second reply does not.
        assert_eq!(out.as_str(), "1\n");
        assert!(inst.error_pending());
        inst.execute(b"SYST:ERR?", &mut out);
        // This reply is also too long for 8 bytes. The error stays.
        assert_eq!(out.as_str(), "");
        assert!(inst.error_pending());
    }

    #[test]
    fn error_query_that_does_not_fit_keeps_the_order() {
        let mut inst = Instrument::new("X", "0");
        let mut out: String<32> = String::new();
        inst.execute(b"BOGUS", &mut out);
        inst.execute(b"ROUT:CLOS (@9)", &mut out);
        // The first reply uses 15 bytes. The error reply does not fit after it.
        inst.execute(b"ROUT:CLOS? (@1:8);:SYST:ERR?", &mut out);
        let mut next = || {
            inst.execute(b"SYST:ERR?", &mut out);
            StdString::from(out.as_str())
        };
        assert_eq!(next(), "-113,\"Undefined header\"\n");
        assert_eq!(next(), "-222,\"Data out of range\"\n");
        assert_eq!(next(), "-223,\"Too much data\"\n");
        assert_eq!(next(), "0,\"No error\"\n");
    }
}
