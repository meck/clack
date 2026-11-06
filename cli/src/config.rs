//! The optional config file with names for boards and channels.
//!
//! Example:
//!
//! ```toml
//! [boards]
//! rack1 = "E66038B713554D2F"
//!
//! [channels]
//! dut1-reset = { board = "rack1", channel = 3, pulse_ms = 300 }
//! dut1-power = { board = "rack1", channel = 4 }
//! ```

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use relay_scpi::chanlist::{Channel, FIRST_CHANNEL, LAST_CHANNEL};
use serde::Deserialize;

/// A channel that has a name in the config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedChannel {
    pub serial: String,
    pub channel: Channel,
    pub pulse_ms: Option<u64>,
}

/// The names from the config file. Without a file, there are no names.
#[derive(Debug, Default)]
pub struct Config {
    /// Board name to serial number.
    boards: BTreeMap<String, String>,
    channels: BTreeMap<String, NamedChannel>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    boards: BTreeMap<String, String>,
    #[serde(default)]
    channels: BTreeMap<String, FileChannel>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileChannel {
    board: String,
    channel: u8,
    pulse_ms: Option<u64>,
}

impl Config {
    /// Reads the config file.
    ///
    /// With `explicit`, the file must exist. Without it, the function reads
    /// the default file. If the default file does not exist, the function
    /// gives an empty config.
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let path = match explicit {
            Some(path) => path.to_path_buf(),
            None => match default_path() {
                Some(path) => path,
                None => return Ok(Self::default()),
            },
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if explicit.is_none() && e.kind() == ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(e) => {
                return Err(e).with_context(|| format!("cannot read {}", path.display()));
            }
        };
        Self::parse(&text)
            .with_context(|| format!("the config file {} has an error", path.display()))
    }

    /// Parses and checks the text of a config file.
    pub fn parse(text: &str) -> Result<Self> {
        let file: File = toml::from_str(text)?;

        let mut boards = BTreeMap::new();
        for (name, serial) in file.boards {
            check_name(&name)?;
            let serial = check_serial(&serial).with_context(|| format!("board '{name}'"))?;
            boards.insert(name, serial);
        }

        let mut channels = BTreeMap::new();
        let mut relays: BTreeMap<(String, u8), String> = BTreeMap::new();
        for (name, entry) in file.channels {
            check_name(&name)?;
            let serial = match boards.get(&entry.board) {
                Some(serial) => String::clone(serial),
                None => check_serial(&entry.board).with_context(|| {
                    format!(
                        "channel '{name}': '{}' is not a board name or a serial number",
                        entry.board
                    )
                })?,
            };
            let Some(channel) = Channel::new(entry.channel) else {
                bail!(
                    "channel '{name}': the channel number must be {FIRST_CHANNEL} to {LAST_CHANNEL}"
                );
            };
            if entry.pulse_ms == Some(0) {
                bail!("channel '{name}': pulse_ms must be more than 0");
            }
            if let Some(other) = relays.insert((serial.clone(), entry.channel), name.clone()) {
                bail!("the channels '{other}' and '{name}' are the same relay");
            }
            channels.insert(
                name,
                NamedChannel {
                    serial,
                    channel,
                    pulse_ms: entry.pulse_ms,
                },
            );
        }
        Ok(Self { boards, channels })
    }

    /// Gives the channel with this name.
    pub fn channel(&self, name: &str) -> Option<&NamedChannel> {
        self.channels.get(name)
    }

    /// Gives the serial number of the board with this name.
    pub fn board(&self, name: &str) -> Option<&str> {
        self.boards.get(name).map(String::as_str)
    }

    /// Gives the name of the board with this serial number.
    pub fn board_name(&self, serial: &str) -> Option<&str> {
        self.boards
            .iter()
            .find(|(_, s)| s.as_str() == serial)
            .map(|(name, _)| name.as_str())
    }

    /// Gives the name of a channel on a board.
    pub fn channel_name(&self, serial: &str, channel: Channel) -> Option<&str> {
        self.channels
            .iter()
            .find(|(_, c)| c.serial == serial && c.channel == channel)
            .map(|(name, _)| name.as_str())
    }
}

/// `$XDG_CONFIG_HOME/clack/config.toml`, or `~/.config/clack/config.toml`.
fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("clack").join("config.toml"))
}

/// A name starts with a letter and has only letters, digits, `-` and `_`.
/// Thus a name is never a channel number or a range. `all` is not a name.
fn check_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !first_ok || !rest_ok {
        bail!(
            "'{name}' is not a correct name. Start with a letter, and use only letters, digits, '-' and '_'"
        );
    }
    if name.eq_ignore_ascii_case("all") {
        bail!("'all' is not a correct name, because it selects all channels");
    }
    Ok(())
}

/// A serial number has 16 hexadecimal digits. The function gives it in
/// upper case, the same as the board.
fn check_serial(serial: &str) -> Result<String> {
    if serial.len() != 16 || !serial.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("'{serial}' is not a serial number with 16 hexadecimal digits");
    }
    Ok(serial.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_empty_config() {
        let c = Config::parse("").unwrap();
        assert!(c.channel("x").is_none());
        assert!(c.board("x").is_none());
    }

    #[test]
    fn names_and_lookups() {
        let c = Config::parse(
            r#"
            [boards]
            rack1 = "e66038b713554d2f"
            [channels]
            dut1-reset = { board = "rack1", channel = 3, pulse_ms = 300 }
            direct = { board = "4E1F2A9C7B305D18", channel = 8 }
            "#,
        )
        .unwrap();
        assert_eq!(c.board("rack1"), Some("E66038B713554D2F"));
        assert_eq!(c.board_name("E66038B713554D2F"), Some("rack1"));
        let ch = c.channel("dut1-reset").unwrap();
        assert_eq!(ch.serial, "E66038B713554D2F");
        assert_eq!(ch.channel.number(), 3);
        assert_eq!(ch.pulse_ms, Some(300));
        assert_eq!(c.channel("direct").unwrap().serial, "4E1F2A9C7B305D18");
        assert_eq!(
            c.channel_name("E66038B713554D2F", Channel::new(3).unwrap()),
            Some("dut1-reset")
        );
        assert_eq!(
            c.channel_name("E66038B713554D2F", Channel::new(4).unwrap()),
            None
        );
    }

    #[test]
    fn bad_files() {
        let bad = [
            "x = 1",
            "[boards]\nrack1 = \"123\"",
            "[boards]\n1rack = \"E66038B713554D2F\"",
            "[boards]\nall = \"E66038B713554D2F\"",
            "[channels]\na = { board = \"nope\", channel = 1 }",
            "[channels]\na = { board = \"E66038B713554D2F\", channel = 9 }",
            "[channels]\na = { board = \"E66038B713554D2F\", channel = 1, pulse_ms = 0 }",
            "[channels]\na = { board = \"E66038B713554D2F\", channel = 1, extra = 1 }",
            "[channels]\na = { board = \"E66038B713554D2F\", channel = 1 }\nb = { board = \"E66038B713554D2F\", channel = 1 }",
        ];
        for text in bad {
            assert!(Config::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn missing_explicit_file_is_an_error() {
        assert!(Config::load(Some(Path::new("/nonexistent/clack.toml"))).is_err());
    }
}
