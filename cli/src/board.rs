//! The SCPI connection to one board.

use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use relay_scpi::identity::IDN_MODEL;
use serde::Serialize;
use serialport::{ClearBuffer, SerialPort};

use crate::channels::{all_channels, all_mask, scpi_list};

/// The maximum time for a reply from the board.
const REPLY_TIMEOUT: Duration = Duration::from_secs(1);

/// The time without input after which `discard_input` stops.
const QUIET_TIME: Duration = Duration::from_millis(50);

/// The maximum time for `discard_input`.
const DISCARD_LIMIT: Duration = Duration::from_millis(500);

/// The maximum number of old lines before the reply to the sync query.
const SYNC_LINES: usize = 16;

/// Sends lines to a board and receives lines from it.
pub trait Transport {
    /// Sends one line. The function adds the `\n`.
    fn write_line(&mut self, line: &str) -> Result<()>;
    /// Receives one line, without the `\n`.
    fn read_line(&mut self) -> Result<String>;
    /// Discards all input until no input arrives for a short time.
    fn discard_input(&mut self) -> Result<()>;
}

/// A board on a serial port.
pub struct SerialTransport {
    port: Box<dyn SerialPort>,
    pending: Vec<u8>,
}

impl SerialTransport {
    pub fn open(path: &str) -> Result<Self> {
        // The board ignores the baud rate. USB sets the speed.
        let port = serialport::new(path, 115_200)
            .timeout(QUIET_TIME)
            .open()
            .with_context(|| format!("cannot open {path}"))?;
        // Discard the input that the host has from an earlier program.
        // `Board::sync` discards the old replies that arrive later.
        port.clear(ClearBuffer::Input)
            .with_context(|| format!("cannot clear the input of {path}"))?;
        Ok(Self {
            port,
            pending: Vec::new(),
        })
    }
}

impl Transport for SerialTransport {
    fn write_line(&mut self, line: &str) -> Result<()> {
        self.port.write_all(line.as_bytes())?;
        self.port.write_all(b"\n")?;
        self.port.flush()?;
        Ok(())
    }

    fn read_line(&mut self) -> Result<String> {
        let start = Instant::now();
        loop {
            if let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
                let rest = self.pending.split_off(end.saturating_add(1));
                let mut line = std::mem::replace(&mut self.pending, rest);
                line.pop();
                let text =
                    String::from_utf8(line).context("the reply of the board is not UTF-8")?;
                return Ok(text.trim_end_matches('\r').to_owned());
            }
            if start.elapsed() >= REPLY_TIMEOUT {
                bail!(
                    "the board did not reply in {} ms",
                    REPLY_TIMEOUT.as_millis()
                );
            }
            let mut buf = [0u8; 64];
            match self.port.read(&mut buf) {
                Ok(n) => self
                    .pending
                    .extend_from_slice(buf.get(..n).unwrap_or_default()),
                Err(e) if e.kind() == ErrorKind::TimedOut => {}
                Err(e) => return Err(e).context("cannot read from the board"),
            }
        }
    }

    fn discard_input(&mut self) -> Result<()> {
        self.pending.clear();
        let start = Instant::now();
        let mut buf = [0u8; 64];
        // Each read waits for a maximum of QUIET_TIME (the port timeout).
        while start.elapsed() < DISCARD_LIMIT {
            match self.port.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::TimedOut => break,
                Err(e) => return Err(e).context("cannot read from the board"),
            }
        }
        Ok(())
    }
}

/// The identity from `*IDN?`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idn {
    pub serial: String,
    pub version: String,
}

/// An entry from the error queue of the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardError {
    pub code: i32,
    pub message: String,
}

impl fmt::Display for BoardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},\"{}\"", self.code, self.message)
    }
}

/// The SCPI commands of the board.
pub struct Board<T> {
    transport: T,
}

impl<T: Transport> Board<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    #[cfg(test)]
    pub fn transport(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Discards old replies, so that the next reply is the reply to the
    /// next query.
    ///
    /// An old reply stays in the board if an earlier program sent a query
    /// and closed the port before it read the reply. The board sends that
    /// reply when this program reads from the port. The function discards
    /// the input, sends `*OPC?` and reads lines until it gets `1`. Then it
    /// discards the input again, because the `1` can also be an old reply.
    pub fn sync(&mut self) -> Result<()> {
        self.transport.discard_input()?;
        self.transport.write_line("*OPC?")?;
        let mut synced = false;
        for _ in 0..SYNC_LINES {
            if self.transport.read_line()?.trim() == "1" {
                synced = true;
                break;
            }
        }
        if !synced {
            bail!("the board did not reply to *OPC?");
        }
        self.transport.discard_input()
    }

    /// Sends a line that has no query.
    pub fn send(&mut self, line: &str) -> Result<()> {
        self.transport.write_line(line)
    }

    /// Sends a line that has a query, and gives the reply.
    pub fn query(&mut self, line: &str) -> Result<String> {
        self.transport.write_line(line)?;
        self.transport.read_line()
    }

    pub fn idn(&mut self) -> Result<Idn> {
        let reply = self.query("*IDN?")?;
        let fields: Vec<&str> = reply.split(',').collect();
        match fields.as_slice() {
            [_, model, serial, version] if *model == IDN_MODEL => Ok(Idn {
                serial: (*serial).to_owned(),
                version: (*version).to_owned(),
            }),
            _ => bail!("the *IDN? reply is not from a {IDN_MODEL}: {reply}"),
        }
    }

    /// Clears the error queue, so that old errors do not stop this program.
    pub fn clear_errors(&mut self) -> Result<()> {
        self.send("*CLS")
    }

    /// Reads the oldest error. Gives `None` if the queue is empty.
    pub fn next_error(&mut self) -> Result<Option<BoardError>> {
        let reply = self.query("SYST:ERR?")?;
        let parsed = reply.split_once(',').and_then(|(code, message)| {
            let code = code.trim().parse::<i32>().ok()?;
            let message = message.trim().trim_matches('"').to_owned();
            Some(BoardError { code, message })
        });
        match parsed {
            Some(e) if e.code == 0 => Ok(None),
            Some(e) => Ok(Some(e)),
            None => bail!("the SYST:ERR? reply has an unknown format: {reply}"),
        }
    }

    /// Makes sure that the board executed the earlier lines without an
    /// error. The board executes the lines in sequence, thus the reply
    /// comes after the earlier lines are complete.
    pub fn check(&mut self) -> Result<()> {
        match self.next_error()? {
            None => Ok(()),
            Some(e) => bail!("the board reported the error {e}"),
        }
    }

    /// Gives the mask of the closed relays. Bit 0 is channel 1.
    pub fn closed(&mut self) -> Result<u8> {
        let reply = self.query(&format!("ROUT:CLOS? {}", scpi_list(all_mask())))?;
        let values: Vec<&str> = reply.split(',').map(str::trim).collect();
        if values.len() != all_channels().count() {
            bail!("the ROUT:CLOS? reply has an unknown format: {reply}");
        }
        let mut mask = 0u8;
        for (value, ch) in values.iter().zip(all_channels()) {
            match *value {
                "1" => mask |= ch.bit(),
                "0" => {}
                _ => bail!("the ROUT:CLOS? reply has an unknown format: {reply}"),
            }
        }
        Ok(mask)
    }

    /// Closes and opens relays with one line. The board switches all of
    /// them at the same time.
    pub fn switch(&mut self, close: u8, open: u8) -> Result<()> {
        let mut units = Vec::new();
        if close != 0 {
            units.push(format!(":ROUT:CLOS {}", scpi_list(close)));
        }
        if open != 0 {
            units.push(format!(":ROUT:OPEN {}", scpi_list(open)));
        }
        if units.is_empty() {
            return Ok(());
        }
        self.send(&units.join(";"))?;
        self.check()
    }

    /// Opens all relays and sets the beeper state to off (`*RST`).
    pub fn reset(&mut self) -> Result<()> {
        self.send("*RST")?;
        self.check()
    }

    /// Gives the beeper state: `true` if the board beeps on each relay change.
    pub fn beeper(&mut self) -> Result<bool> {
        let reply = self.query("SYST:BEEP:STAT?")?;
        match reply.trim() {
            "1" => Ok(true),
            "0" => Ok(false),
            _ => bail!("the SYST:BEEP:STAT? reply has an unknown format: {reply}"),
        }
    }

    pub fn set_beeper(&mut self, on: bool) -> Result<()> {
        self.send(if on {
            "SYST:BEEP:STAT ON"
        } else {
            "SYST:BEEP:STAT OFF"
        })?;
        self.check()
    }

    pub fn beep(&mut self) -> Result<()> {
        self.send("SYST:BEEP")?;
        self.check()
    }
}

#[cfg(test)]
pub mod fake {
    //! A board in memory. It uses the SCPI parser of the firmware.

    use std::collections::VecDeque;

    use anyhow::{Result, bail};
    use relay_scpi::Instrument;

    use super::Transport;

    pub const SERIAL: &str = "4E1F2A9C7B305D18";

    pub struct FakeBoard {
        pub instrument: Instrument,
        /// The replies that the client did not read.
        pub replies: VecDeque<String>,
        /// All lines that the client sent.
        pub sent: Vec<String>,
        /// Old replies that arrive after the first `discard_input`.
        pub late: Vec<String>,
        /// After a line that starts with this text, the next reply does not
        /// arrive. This occurs one time.
        pub lose_reply_after: Option<&'static str>,
        lose_next_reply: bool,
    }

    impl FakeBoard {
        pub fn new() -> Self {
            Self {
                instrument: Instrument::new(SERIAL, "0.1.0"),
                replies: VecDeque::new(),
                sent: Vec::new(),
                late: Vec::new(),
                lose_reply_after: None,
                lose_next_reply: false,
            }
        }
    }

    impl Transport for FakeBoard {
        fn write_line(&mut self, line: &str) -> Result<()> {
            self.sent.push(line.to_owned());
            if self
                .lose_reply_after
                .is_some_and(|start| line.starts_with(start))
            {
                self.lose_reply_after = None;
                self.lose_next_reply = true;
            }
            let mut out: heapless::String<256> = heapless::String::new();
            self.instrument.execute(line.as_bytes(), &mut out);
            for reply in out.lines() {
                self.replies.push_back(reply.to_owned());
            }
            Ok(())
        }

        fn read_line(&mut self) -> Result<String> {
            if self.lose_next_reply {
                self.lose_next_reply = false;
                self.replies.pop_front();
                bail!("the board did not reply");
            }
            match self.replies.pop_front() {
                Some(reply) => Ok(reply),
                None => bail!("the board did not reply"),
            }
        }

        fn discard_input(&mut self) -> Result<()> {
            self.replies.clear();
            self.replies.extend(self.late.drain(..));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeBoard, SERIAL};
    use super::*;

    fn board() -> Board<FakeBoard> {
        Board::new(FakeBoard::new())
    }

    #[test]
    fn sync_discards_old_replies() {
        let mut b = board();
        // An old reply waits in the board, and an old `1` arrives late.
        b.transport.replies.push_back("0,0,0,0,0,0,0,0".to_owned());
        b.transport.late.push("1".to_owned());
        b.sync().unwrap();
        assert_eq!(b.idn().unwrap().serial, SERIAL);
        assert_eq!(b.next_error().unwrap(), None);
    }

    #[test]
    fn identity() {
        let idn = board().idn().unwrap();
        assert_eq!(idn.serial, SERIAL);
        assert_eq!(idn.version, "0.1.0");
    }

    #[test]
    fn switch_in_one_line() {
        let mut b = board();
        b.switch(0b0000_0101, 0).unwrap();
        assert_eq!(b.closed().unwrap(), 0b0000_0101);
        b.switch(0b0000_0010, 0b0000_0001).unwrap();
        assert_eq!(b.closed().unwrap(), 0b0000_0110);
        assert!(
            b.transport
                .sent
                .contains(&":ROUT:CLOS (@2);:ROUT:OPEN (@1)".to_owned())
        );
    }

    #[test]
    fn errors_are_reported() {
        let mut b = board();
        b.send("FOO").unwrap();
        let err = b.check().unwrap_err().to_string();
        assert!(err.contains("-113"), "{err}");
        assert_eq!(b.next_error().unwrap(), None);
    }

    #[test]
    fn reset_and_beeper() {
        let mut b = board();
        b.set_beeper(true).unwrap();
        assert!(b.beeper().unwrap());
        b.switch(0xff, 0).unwrap();
        b.reset().unwrap();
        assert_eq!(b.closed().unwrap(), 0);
        assert!(!b.beeper().unwrap());
        b.beep().unwrap();
    }
}
