//! A momentary close of relays, for example a press of a reset button.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::board::{Board, Transport};
use crate::channels::describe;

/// How the pulse ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Pulse {
    /// The relays stayed closed for the full time.
    Complete,
    /// `stop` became true before the end of the time. The relays are open.
    Stopped,
}

/// Closes the relays in `mask`, waits for `time` and opens them again.
///
/// If a relay in `mask` is already closed, the function changes nothing
/// and gives an error. If `stop` is true before the close, the function
/// does not close the relays. If `stop` becomes true during the wait, the
/// function opens the relays immediately. If the close gives an error,
/// the function tries to open the relays before it gives the error.
pub fn pulse<T: Transport>(
    board: &mut Board<T>,
    mask: u8,
    time: Duration,
    stop: &AtomicBool,
) -> Result<Pulse> {
    let closed = board.closed()? & mask;
    if closed != 0 {
        bail!(
            "{} already closed. The pulse did not start",
            if closed.is_power_of_two() {
                format!("{} is", describe(closed))
            } else {
                format!("{} are", describe(closed))
            }
        );
    }

    if stop.load(Ordering::SeqCst) {
        return Ok(Pulse::Stopped);
    }
    if let Err(err) = board.switch(mask, 0) {
        // The close command can be on the board already, also if its
        // check failed. Thus send the open command before the error goes
        // to the caller.
        return Err(match board.switch(0, mask) {
            Ok(()) => err.context("the pulse did not complete. The relays are open"),
            Err(open_err) => err.context(format!(
                "the pulse did not complete, and the relays can stay closed \
                 ({open_err:#})"
            )),
        });
    }
    let start = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        match time.checked_sub(start.elapsed()) {
            Some(left) if !left.is_zero() => {
                std::thread::sleep(left.min(Duration::from_millis(10)));
            }
            _ => break,
        }
    }
    let stopped = stop.load(Ordering::SeqCst);
    board.switch(0, mask)?;
    Ok(if stopped {
        Pulse::Stopped
    } else {
        Pulse::Complete
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::fake::FakeBoard;

    #[test]
    fn closes_then_opens() {
        let mut b = Board::new(FakeBoard::new());
        let stop = AtomicBool::new(false);
        let start = Instant::now();
        let result = pulse(&mut b, 0b0000_0100, Duration::from_millis(30), &stop).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(30));
        assert_eq!(result, Pulse::Complete);
        assert_eq!(b.closed().unwrap(), 0);
    }

    #[test]
    fn refuses_a_closed_relay() {
        let mut b = Board::new(FakeBoard::new());
        b.switch(0b0000_0100, 0).unwrap();
        let stop = AtomicBool::new(false);
        let err = pulse(&mut b, 0b0000_0110, Duration::ZERO, &stop).unwrap_err();
        assert!(err.to_string().contains("CH3 is already closed"), "{err}");
        // Nothing changed.
        assert_eq!(b.closed().unwrap(), 0b0000_0100);
    }

    fn sent_close(b: &mut Board<FakeBoard>) -> bool {
        b.transport().sent.iter().any(|line| line.contains("CLOS "))
    }

    #[test]
    fn stop_during_the_wait_opens_the_relays() {
        let mut b = Board::new(FakeBoard::new());
        let stop = AtomicBool::new(false);
        let start = Instant::now();
        let result = std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(30));
                stop.store(true, Ordering::SeqCst);
            });
            pulse(&mut b, 0b0000_0001, Duration::from_secs(10), &stop).unwrap()
        });
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(result, Pulse::Stopped);
        assert!(sent_close(&mut b));
        assert_eq!(b.closed().unwrap(), 0);
    }

    #[test]
    fn stop_before_the_close_does_not_close() {
        let mut b = Board::new(FakeBoard::new());
        let stop = AtomicBool::new(true);
        let result = pulse(&mut b, 0b0000_0001, Duration::from_secs(10), &stop).unwrap();
        assert_eq!(result, Pulse::Stopped);
        assert!(!sent_close(&mut b));
        assert_eq!(b.closed().unwrap(), 0);
    }

    #[test]
    fn failed_close_check_opens_the_relays() {
        let mut b = Board::new(FakeBoard::new());
        // The reply to the check after the close does not arrive.
        b.transport().lose_reply_after = Some(":ROUT:CLOS");
        let stop = AtomicBool::new(false);
        let err = pulse(&mut b, 0b0000_0100, Duration::from_secs(10), &stop).unwrap_err();
        assert!(
            format!("{err:#}").contains("The relays are open"),
            "{err:#}"
        );
        assert_eq!(b.closed().unwrap(), 0);
    }
}
