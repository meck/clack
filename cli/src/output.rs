//! Text and JSON output.

use anyhow::Result;
use serde::Serialize;

use crate::board::BoardError;

/// One board in the output of `list`.
#[derive(Debug, Serialize)]
pub struct BoardRow {
    pub port: String,
    pub serial: String,
    pub name: Option<String>,
    pub version: String,
}

/// The output of `status`.
#[derive(Debug, Serialize)]
pub struct Status {
    pub port: String,
    pub serial: String,
    pub name: Option<String>,
    pub beeper: bool,
    pub channels: Vec<ChannelState>,
}

#[derive(Debug, Serialize)]
pub struct ChannelState {
    pub channel: u8,
    pub closed: bool,
    pub name: Option<String>,
}

pub fn print_json<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub fn print_list(rows: &[BoardRow]) {
    for row in rows {
        let name = row.name.as_deref().unwrap_or("");
        println!("{}  {}  {}  {}", row.port, row.serial, row.version, name);
    }
}

pub fn print_status(status: &Status) {
    match &status.name {
        Some(name) => println!("{} {} on {}", status.serial, name, status.port),
        None => println!("{} on {}", status.serial, status.port),
    }
    for ch in &status.channels {
        let state = if ch.closed { "closed" } else { "open  " };
        match &ch.name {
            Some(name) => println!("CH{} {state}  {name}", ch.channel),
            None => println!("CH{} {}", ch.channel, state.trim_end()),
        }
    }
    println!("beeper {}", if status.beeper { "on" } else { "off" });
}

pub fn print_errors(errors: &[BoardError]) {
    for e in errors {
        println!("{e}");
    }
}
