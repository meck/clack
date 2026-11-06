//! Collects input bytes into lines.

/// The result of one byte that goes into a [`LineBuffer`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Push {
    /// The line is not complete.
    Pending,
    /// A line is complete. Read it with [`LineBuffer::line`].
    Line,
    /// A line was too long. The buffer discarded it.
    Overrun,
}

/// Collects bytes into lines of a maximum of `N` bytes.
///
/// A line ends with `\n`, `\r\n` or `\r`. Thus a terminal that sends only
/// `\r` for the Enter key also operates. Empty lines do not give
/// [`Push::Line`]. Thus the `\n` after a `\r` has no effect.
/// If a line is longer than `N` bytes, the buffer discards the bytes
/// until the next line end, and then gives [`Push::Overrun`] one time.
pub struct LineBuffer<const N: usize> {
    buf: [u8; N],
    len: usize,
    complete: bool,
    overrun: bool,
}

impl<const N: usize> Default for LineBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> LineBuffer<N> {
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
            complete: false,
            overrun: false,
        }
    }

    /// Adds one byte.
    pub fn push(&mut self, byte: u8) -> Push {
        if self.complete {
            self.complete = false;
            self.len = 0;
        }
        if byte == b'\r' || byte == b'\n' {
            if self.overrun {
                self.overrun = false;
                self.len = 0;
                return Push::Overrun;
            }
            if self.len == 0 {
                return Push::Pending;
            }
            self.complete = true;
            return Push::Line;
        }
        if self.overrun {
            return Push::Pending;
        }
        let Some(slot) = self.buf.get_mut(self.len) else {
            self.overrun = true;
            return Push::Pending;
        };
        *slot = byte;
        // There was a free slot, thus len < N. The addition cannot overflow.
        self.len = self.len.saturating_add(1);
        Push::Pending
    }

    /// Gives the line after [`Push::Line`]. The line has no terminator.
    /// The next call to [`LineBuffer::push`] removes it.
    pub fn line(&self) -> &[u8] {
        if self.complete {
            self.buf.get(..self.len).unwrap_or_default()
        } else {
            &[]
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn lines<const N: usize>(input: &[u8]) -> Vec<Result<Vec<u8>, ()>> {
        let mut lb = LineBuffer::<N>::new();
        let mut out = Vec::new();
        for &b in input {
            match lb.push(b) {
                Push::Pending => {}
                Push::Line => out.push(Ok(lb.line().to_vec())),
                Push::Overrun => out.push(Err(())),
            }
        }
        out
    }

    #[test]
    fn terminators() {
        let out = lines::<16>(b"*IDN?\n*RST\r\n\n\r\nX\n");
        assert_eq!(
            out,
            [
                Ok(b"*IDN?".to_vec()),
                Ok(b"*RST".to_vec()),
                Ok(b"X".to_vec())
            ]
        );
    }

    #[test]
    fn carriage_return_ends_a_line() {
        let out = lines::<16>(b"*IDN?\r*RST\r\rX\r\n");
        assert_eq!(
            out,
            [
                Ok(b"*IDN?".to_vec()),
                Ok(b"*RST".to_vec()),
                Ok(b"X".to_vec())
            ]
        );
    }

    #[test]
    fn overrun_discards_until_newline() {
        let out = lines::<4>(b"12345678\nabc\n");
        assert_eq!(out, [Err(()), Ok(b"abc".to_vec())]);
    }

    #[test]
    fn overrun_with_crlf_gives_one_overrun() {
        let out = lines::<4>(b"12345678\r\nabc\r\n");
        assert_eq!(out, [Err(()), Ok(b"abc".to_vec())]);
    }

    #[test]
    fn exact_length_is_ok() {
        let out = lines::<4>(b"abcd\nabcd\r\n");
        assert_eq!(out, [Ok(b"abcd".to_vec()), Ok(b"abcd".to_vec())]);
    }
}
