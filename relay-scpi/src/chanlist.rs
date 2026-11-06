//! SCPI channel lists, for example `(@1,3,5:8)`.

use crate::error::ScpiError;

/// The lowest channel number.
pub const FIRST_CHANNEL: u8 = 1;
/// The highest channel number.
pub const LAST_CHANNEL: u8 = 8;

// The relay mask has 8 bits, thus there can be a maximum of 8 channels.
const _: () = assert!(LAST_CHANNEL - FIRST_CHANNEL < 8);

/// A channel number in the range `FIRST_CHANNEL..=LAST_CHANNEL`.
///
/// Only [`Channel::new`] and [`ChannelList`] make a `Channel`. Both check
/// the range. Thus each `Channel` has a correct number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Channel(u8);

impl Channel {
    /// Makes a channel. If the number is not in the range, it gives `None`.
    pub fn new(number: u8) -> Option<Self> {
        (FIRST_CHANNEL..=LAST_CHANNEL)
            .contains(&number)
            .then_some(Self(number))
    }

    /// Gives the channel number.
    pub fn number(self) -> u8 {
        self.0
    }

    /// Gives the bit for this channel in the relay mask. Bit 0 is
    /// `FIRST_CHANNEL`.
    pub fn bit(self) -> u8 {
        // The number is in the range, thus the shift is 0 to 7. The
        // wrapping operations cannot wrap.
        1u8.wrapping_shl(u32::from(self.0.wrapping_sub(FIRST_CHANNEL)))
    }
}

/// A channel list that has a correct syntax and correct channel numbers.
///
/// The list keeps the order of the text. A query gives one value for each
/// channel in this order. A range such as `8:5` goes down.
#[derive(Clone, Copy, Debug)]
pub struct ChannelList<'a> {
    items: &'a str,
}

impl<'a> ChannelList<'a> {
    /// Parses a channel list parameter.
    ///
    /// Spaces around the parentheses, the items and the separators are
    /// permitted.
    pub fn parse(param: &'a str) -> Result<Self, ScpiError> {
        let param = param.trim();
        let inner = param
            .strip_prefix('(')
            .ok_or(ScpiError::DataTypeError)?
            .trim_start()
            .strip_prefix('@')
            .ok_or(ScpiError::DataTypeError)?;
        let items = inner.strip_suffix(')').ok_or(ScpiError::SyntaxError)?;
        if items.trim().is_empty() {
            return Err(ScpiError::MissingParameter);
        }
        let list = Self { items };
        // Do a full check now, so that later iteration cannot fail.
        list.try_for_each(|_| Ok(()))?;
        Ok(list)
    }

    /// Gives a bit mask of the channels in the list. Bit 0 is channel 1.
    pub fn mask(&self) -> u8 {
        let mut mask = 0u8;
        self.for_each(|ch| mask |= ch.bit());
        mask
    }

    /// Calls `f` for each channel, in the order of the text.
    pub fn for_each(&self, mut f: impl FnMut(Channel)) {
        // `parse` did the check, thus there is no error here.
        let _ = self.try_for_each(|ch| {
            f(ch);
            Ok(())
        });
    }

    fn try_for_each(
        &self,
        mut f: impl FnMut(Channel) -> Result<(), ScpiError>,
    ) -> Result<(), ScpiError> {
        for item in self.items.split(',') {
            let (first, last) = if let Some((a, b)) = item.split_once(':') {
                (channel(a)?, channel(b)?)
            } else {
                let ch = channel(item)?;
                (ch, ch)
            };
            // Each number between two correct channels is also correct.
            if first.0 <= last.0 {
                for n in first.0..=last.0 {
                    f(Channel(n))?;
                }
            } else {
                for n in (last.0..=first.0).rev() {
                    f(Channel(n))?;
                }
            }
        }
        Ok(())
    }
}

fn channel(text: &str) -> Result<Channel, ScpiError> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ScpiError::SyntaxError);
    }
    // The text has only digits. Thus a parse error means that the value
    // is too large for a u8.
    let value: u8 = text.parse().map_err(|_| ScpiError::DataOutOfRange)?;
    Channel::new(value).ok_or(ScpiError::DataOutOfRange)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn order(text: &str) -> Vec<u8> {
        let mut v = Vec::new();
        ChannelList::parse(text)
            .unwrap()
            .for_each(|c| v.push(c.number()));
        v
    }

    #[test]
    fn single_and_lists() {
        assert_eq!(order("(@1)"), [1]);
        assert_eq!(order("(@1,3,5:8)"), [1, 3, 5, 6, 7, 8]);
        assert_eq!(order(" ( @ 2 , 4 : 3 ) "), [2, 4, 3]);
        assert_eq!(order("(@8:6)"), [8, 7, 6]);
        assert_eq!(order("(@1,1)"), [1, 1]);
    }

    #[test]
    fn mask() {
        assert_eq!(ChannelList::parse("(@1:8)").unwrap().mask(), 0xff);
        assert_eq!(ChannelList::parse("(@1,8)").unwrap().mask(), 0x81);
    }

    #[test]
    fn channel_range_and_bits() {
        assert_eq!(Channel::new(0), None);
        assert_eq!(Channel::new(9), None);
        assert_eq!(Channel::new(255), None);
        assert_eq!(Channel::new(1).map(Channel::bit), Some(0x01));
        assert_eq!(Channel::new(8).map(Channel::bit), Some(0x80));
        for n in FIRST_CHANNEL..=LAST_CHANNEL {
            let ch = Channel::new(n).unwrap();
            assert_eq!(ch.number(), n);
            assert_eq!(ch.bit().count_ones(), 1);
        }
    }

    #[test]
    fn errors() {
        use ScpiError::*;
        let e = |t| ChannelList::parse(t).unwrap_err();
        assert_eq!(e("1"), DataTypeError);
        assert_eq!(e("(1)"), DataTypeError);
        assert_eq!(e("(@1"), SyntaxError);
        assert_eq!(e("(@)"), MissingParameter);
        assert_eq!(e("(@0)"), DataOutOfRange);
        assert_eq!(e("(@9)"), DataOutOfRange);
        assert_eq!(e("(@1:9)"), DataOutOfRange);
        assert_eq!(e("(@256)"), DataOutOfRange);
        assert_eq!(e("(@99999999999)"), DataOutOfRange);
        assert_eq!(e("(@1,,2)"), SyntaxError);
        assert_eq!(e("(@a)"), SyntaxError);
        assert_eq!(e("(@-1)"), SyntaxError);
    }
}
