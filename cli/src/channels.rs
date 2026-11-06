//! Channel arguments, for example `1,3,5-8`, `all` and names from the
//! config file.

use anyhow::{Result, anyhow, bail};
use relay_scpi::chanlist::{Channel, FIRST_CHANNEL, LAST_CHANNEL};

use crate::config::Config;

/// The pulse time for a channel that has no `pulse_ms` in the config file.
pub const DEFAULT_PULSE_MS: u64 = 200;

/// The channels of one command.
#[derive(Debug, PartialEq, Eq)]
pub struct Selection {
    /// The relay mask. Bit 0 is channel 1.
    pub mask: u8,
    /// The serial number of the board that the channel names select.
    /// `None` if the argument has no names.
    pub serial: Option<String>,
    /// The longest pulse time of the channels in the selection.
    pub pulse_ms: u64,
}

impl Selection {
    /// No channels, for a command that does not switch relays.
    pub fn none() -> Self {
        Self {
            mask: 0,
            serial: None,
            pulse_ms: DEFAULT_PULSE_MS,
        }
    }
}

/// Gives all channels, from the first to the last.
pub fn all_channels() -> impl Iterator<Item = Channel> {
    (FIRST_CHANNEL..=LAST_CHANNEL).filter_map(Channel::new)
}

/// Gives the mask with all channels.
pub fn all_mask() -> u8 {
    all_channels().fold(0, |mask, ch| mask | ch.bit())
}

/// Gives the channels in a mask, from the first to the last.
pub fn in_mask(mask: u8) -> impl Iterator<Item = Channel> {
    all_channels().filter(move |ch| mask & ch.bit() != 0)
}

/// Gives a text such as `CH1, CH3` for the channels in a mask.
pub fn describe(mask: u8) -> String {
    in_mask(mask)
        .map(|ch| format!("CH{}", ch.number()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parses a channel argument. Items are numbers (`3`), ranges (`5-8`),
/// `all` and channel names from the config file. A comma separates the
/// items.
pub fn parse(arg: &str, config: &Config) -> Result<Selection> {
    let mut mask = 0u8;
    let mut serial: Option<String> = None;
    let mut pulse_ms = 0u64;
    for item in arg.split(',') {
        let item = item.trim();
        if item.is_empty() {
            bail!("the channel list '{arg}' has an empty item");
        }
        if let Some(named) = config.channel(item) {
            match &serial {
                Some(s) if *s != named.serial => {
                    bail!("the names in '{arg}' are on more than one board");
                }
                _ => serial = Some(named.serial.clone()),
            }
            mask |= named.channel.bit();
            pulse_ms = pulse_ms.max(named.pulse_ms.unwrap_or(DEFAULT_PULSE_MS));
        } else {
            mask |= if item.eq_ignore_ascii_case("all") {
                all_mask()
            } else {
                range(item)?
            };
            pulse_ms = pulse_ms.max(DEFAULT_PULSE_MS);
        }
    }
    Ok(Selection {
        mask,
        serial,
        pulse_ms,
    })
}

/// Parses `3` or `5-8`. A range can also go down, for example `8-5`.
fn range(item: &str) -> Result<u8> {
    let (first, last) = item.split_once('-').unwrap_or((item, item));
    let a = number(first, item)?.number();
    let b = number(last, item)?.number();
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    Ok((low..=high)
        .filter_map(Channel::new)
        .fold(0, |mask, ch| mask | ch.bit()))
}

fn number(text: &str, item: &str) -> Result<Channel> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        bail!("'{item}' is not a channel number, a range, 'all' or a configured name");
    }
    text.parse::<u8>()
        .ok()
        .and_then(Channel::new)
        .ok_or_else(|| {
            anyhow!("channel {text} is not in the range {FIRST_CHANNEL} to {LAST_CHANNEL}")
        })
}

/// Gives the SCPI channel list for a mask, for example `(@1,3,5:8)`.
pub fn scpi_list(mask: u8) -> String {
    let mut items: Vec<String> = Vec::new();
    let mut run: Option<(u8, u8)> = None;
    for n in in_mask(mask).map(Channel::number) {
        run = match run {
            Some((first, last)) if last.checked_add(1) == Some(n) => Some((first, n)),
            Some(done) => {
                items.push(list_item(done));
                Some((n, n))
            }
            None => Some((n, n)),
        };
    }
    if let Some(done) = run {
        items.push(list_item(done));
    }
    format!("(@{})", items.join(","))
}

fn list_item((first, last): (u8, u8)) -> String {
    if first == last {
        first.to_string()
    } else {
        format!("{first}:{last}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIAL_A: &str = "E66038B713554D2F";
    const SERIAL_B: &str = "4E1F2A9C7B305D18";

    fn config() -> Config {
        Config::parse(&format!(
            r#"
            [boards]
            rack1 = "{SERIAL_A}"
            [channels]
            dut1-reset = {{ board = "rack1", channel = 3, pulse_ms = 300 }}
            dut1-power = {{ board = "rack1", channel = 4 }}
            other = {{ board = "{SERIAL_B}", channel = 1 }}
            "#
        ))
        .unwrap()
    }

    fn mask(arg: &str) -> u8 {
        parse(arg, &Config::default()).unwrap().mask
    }

    #[test]
    fn numbers_and_ranges() {
        assert_eq!(mask("1"), 0x01);
        assert_eq!(mask("1,3,5-8"), 0b1111_0101);
        assert_eq!(mask("8-5"), 0b1111_0000);
        assert_eq!(mask(" 2 , 2 "), 0x02);
        assert_eq!(mask("all"), 0xff);
        assert_eq!(mask("ALL,1"), 0xff);
    }

    #[test]
    fn bad_arguments() {
        let none = Config::default();
        for arg in ["", "1,,2", "0", "9", "1-9", "a", "-1", "1-", "+1", "300"] {
            assert!(parse(arg, &none).is_err(), "{arg}");
        }
    }

    #[test]
    fn names() {
        let c = config();
        let sel = parse("dut1-reset,1", &c).unwrap();
        assert_eq!(sel.mask, 0b0000_0101);
        assert_eq!(sel.serial.as_deref(), Some(SERIAL_A));
        assert_eq!(sel.pulse_ms, 300);

        let sel = parse("dut1-power", &c).unwrap();
        assert_eq!(sel.pulse_ms, DEFAULT_PULSE_MS);

        assert!(parse("dut1-reset,other", &c).is_err());
        assert!(parse("unknown", &c).is_err());
    }

    #[test]
    fn numbers_have_no_board() {
        let sel = parse("1-3", &config()).unwrap();
        assert_eq!(sel.serial, None);
        assert_eq!(sel.pulse_ms, DEFAULT_PULSE_MS);
    }

    #[test]
    fn scpi_lists() {
        assert_eq!(scpi_list(0x01), "(@1)");
        assert_eq!(scpi_list(0xff), "(@1:8)");
        assert_eq!(scpi_list(0b1111_0101), "(@1,3,5:8)");
        assert_eq!(scpi_list(0b1000_0001), "(@1,8)");
        assert_eq!(scpi_list(0b0000_0110), "(@2:3)");
    }

    #[test]
    fn descriptions() {
        assert_eq!(describe(0b0000_0101), "CH1, CH3");
    }
}
