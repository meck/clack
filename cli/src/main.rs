//! `clack`: a command-line client for Pico-Relay-B boards with the clack
//! SCPI firmware.

mod board;
mod channels;
mod config;
mod discover;
mod output;
mod pulse;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use relay_scpi::error::QUEUE_LEN;

use crate::board::{Board, BoardError, SerialTransport};
use crate::channels::{Selection, all_channels};
use crate::config::Config;
use crate::discover::Candidate;
use crate::output::{BoardRow, ChannelState, Status};
use crate::pulse::Pulse;

/// The exit code after Ctrl-C or SIGTERM during a pulse.
const EXIT_STOPPED: u8 = 130;

/// Control the relays of Pico-Relay-B boards that have the clack firmware.
///
/// CHANNELS is a list such as 1,3,5-8 or all. It can also have channel
/// names from the config file, for example dut1-reset,4.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// The board: a board name from the config file, or the start of the
    /// serial number. Necessary when the computer has more than one board.
    #[arg(long, global = true)]
    board: Option<String>,

    /// The serial port of the board, for example /dev/ttyACM0. The client
    /// then does not search for boards.
    #[arg(long, global = true, conflicts_with = "board")]
    port: Option<String>,

    #[arg(
        long,
        global = true,
        help = "The config file. Default: $XDG_CONFIG_HOME/clack/config.toml, or \
                ~/.config/clack/config.toml. Without the file, there are no names"
    )]
    config: Option<PathBuf>,

    /// Print the output as JSON.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the connected boards.
    List,
    /// Show the state of each relay and of the beeper.
    Status,
    /// Close relays.
    Close {
        /// The channels, for example 1,3,5-8, all or dut1-reset.
        channels: String,
    },
    /// Open relays.
    Open {
        /// The channels, for example 1,3,5-8, all or dut1-reset.
        channels: String,
    },
    /// Change the state of relays: close the open relays and open the
    /// closed relays. All of them switch at the same time.
    Toggle {
        /// The channels, for example 1,3,5-8, all or dut1-reset.
        channels: String,
    },
    /// Close relays for a time, then open them, for example to push a
    /// reset button. If a relay is already closed, nothing changes.
    Pulse {
        /// The channels, for example 1,3,5-8, all or dut1-reset.
        channels: String,
        #[arg(
            long,
            value_parser = clap::value_parser!(u64).range(1..),
            help = "The time in milliseconds. Default: pulse_ms from the config file, or 200"
        )]
        ms: Option<u64>,
    },
    /// Open all relays and set the beeper to off (*RST).
    Rst,
    /// Set the beeper: on (beep at each relay change), off, or once (one
    /// beep now).
    Beep { action: BeepAction },
    /// Show and clear the error queue of the board.
    Errors,
}

#[derive(Clone, Copy, ValueEnum)]
enum BeepAction {
    On,
    Off,
    Once,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("clack: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode> {
    let config = Config::load(cli.config.as_deref())?;
    match &cli.command {
        Command::List => list(cli, &config)?,
        Command::Status => status(cli, &config)?,
        Command::Close { channels } => {
            let sel = channels::parse(channels, &config)?;
            ready(cli, &config, &sel)?.board.switch(sel.mask, 0)?;
        }
        Command::Open { channels } => {
            let sel = channels::parse(channels, &config)?;
            ready(cli, &config, &sel)?.board.switch(0, sel.mask)?;
        }
        Command::Toggle { channels } => {
            let sel = channels::parse(channels, &config)?;
            let mut board = ready(cli, &config, &sel)?.board;
            let closed = board.closed()?;
            board.switch(sel.mask & !closed, sel.mask & closed)?;
        }
        Command::Pulse { channels, ms } => {
            let sel = channels::parse(channels, &config)?;
            let time = Duration::from_millis(ms.unwrap_or(sel.pulse_ms));
            let mut board = ready(cli, &config, &sel)?.board;
            // From now on, Ctrl-C and SIGTERM end the pulse. The relays
            // then open before the program stops.
            let stop = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&stop);
            ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))
                .context("cannot install the signal handler")?;
            if pulse::pulse(&mut board, sel.mask, time, &stop)? == Pulse::Stopped {
                eprintln!("clack: stopped. The relays are open");
                return Ok(ExitCode::from(EXIT_STOPPED));
            }
        }
        Command::Rst => ready(cli, &config, &Selection::none())?.board.reset()?,
        Command::Beep { action } => {
            let mut board = ready(cli, &config, &Selection::none())?.board;
            match action {
                BeepAction::On => board.set_beeper(true)?,
                BeepAction::Off => board.set_beeper(false)?,
                BeepAction::Once => board.beep()?,
            }
        }
        Command::Errors => errors(cli, &config)?,
    }
    Ok(ExitCode::SUCCESS)
}

/// A board that is open, with its port and serial number.
struct Connected {
    board: Board<SerialTransport>,
    port: String,
    serial: String,
}

/// Opens the board for a command and clears its error queue.
fn ready(cli: &Cli, config: &Config, sel: &Selection) -> Result<Connected> {
    let mut c = connect(cli, config, sel.serial.as_deref())?;
    c.board.clear_errors()?;
    Ok(c)
}

/// Finds and opens the board for a command.
///
/// `wanted` is the serial number that the channel names select.
fn connect(cli: &Cli, config: &Config, wanted: Option<&str>) -> Result<Connected> {
    if let Some(port) = &cli.port {
        let (board, idn) = discover::open(port)?;
        if let Some(serial) = wanted
            && serial != idn.serial
        {
            bail!(
                "the channel names are on board {serial}, but {port} has board {}",
                idn.serial
            );
        }
        return Ok(Connected {
            board,
            port: port.clone(),
            serial: idn.serial,
        });
    }

    // --board is a board name from the config file, or the start of a
    // serial number.
    let by_option = cli.board.as_deref().map(|text| match config.board(text) {
        Some(serial) => (serial.to_owned(), true),
        None => (text.to_ascii_uppercase(), false),
    });

    let found = discover::candidates()?;
    let matches: Vec<&Candidate> = found
        .iter()
        .filter(|c| {
            let serial = c.serial.as_deref().unwrap_or("");
            let names_ok = wanted.is_none_or(|w| serial == w);
            let option_ok = by_option.as_ref().is_none_or(|(text, exact)| {
                if *exact {
                    serial == text
                } else {
                    serial.starts_with(text.as_str())
                }
            });
            names_ok && option_ok
        })
        .collect();

    let candidate = match matches.as_slice() {
        [one] => *one,
        [] if found.is_empty() => bail!("no board found"),
        [] => bail!(
            "no connected board matches. Connected boards:\n{}",
            describe_candidates(config, &found)
        ),
        _ => bail!(
            "{} boards found. Select one with --board:\n{}",
            matches.len(),
            describe_candidates(config, &found)
        ),
    };

    let (board, idn) = discover::open(&candidate.port)?;
    if candidate.serial.as_deref().is_some_and(|s| s != idn.serial) {
        bail!(
            "the USB serial number of {} is not the same as its *IDN? serial number {}",
            candidate.port,
            idn.serial
        );
    }
    Ok(Connected {
        board,
        port: candidate.port.clone(),
        serial: idn.serial,
    })
}

fn describe_candidates(config: &Config, found: &[Candidate]) -> String {
    found
        .iter()
        .map(|c| {
            let serial = c.serial.as_deref().unwrap_or("(no serial number)");
            match config.board_name(serial) {
                Some(name) => format!("  {serial} {name} on {}", c.port),
                None => format!("  {serial} on {}", c.port),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn list(cli: &Cli, config: &Config) -> Result<()> {
    let ports: Vec<String> = match &cli.port {
        Some(port) => vec![port.clone()],
        None => discover::candidates()?
            .into_iter()
            .map(|c| c.port)
            .collect(),
    };
    let mut rows = Vec::new();
    for port in ports {
        match discover::open(&port) {
            Ok((_, idn)) => rows.push(BoardRow {
                name: config.board_name(&idn.serial).map(str::to_owned),
                port,
                serial: idn.serial,
                version: idn.version,
            }),
            Err(e) => eprintln!("clack: {e:#}"),
        }
    }
    if cli.json {
        output::print_json(&rows)
    } else {
        if rows.is_empty() {
            eprintln!("clack: no board found");
        }
        output::print_list(&rows);
        Ok(())
    }
}

/// Shows the relays and the beeper. This command does not clear the error
/// queue, thus `errors` can show the errors after it.
fn status(cli: &Cli, config: &Config) -> Result<()> {
    let Connected {
        mut board,
        port,
        serial,
    } = connect(cli, config, None)?;
    let closed = board.closed()?;
    let beeper = board.beeper()?;
    let status = Status {
        name: config.board_name(&serial).map(str::to_owned),
        beeper,
        channels: all_channels()
            .map(|ch| ChannelState {
                channel: ch.number(),
                closed: closed & ch.bit() != 0,
                name: config.channel_name(&serial, ch).map(str::to_owned),
            })
            .collect(),
        port,
        serial,
    };
    if cli.json {
        output::print_json(&status)
    } else {
        output::print_status(&status);
        Ok(())
    }
}

/// Reads the error queue until it is empty. This command does not clear
/// the queue first.
fn errors(cli: &Cli, config: &Config) -> Result<()> {
    let mut board = connect(cli, config, None)?.board;
    let mut errors: Vec<BoardError> = Vec::new();
    // The queue has a maximum of QUEUE_LEN entries. The limit prevents an
    // endless loop if the board does not operate correctly.
    for _ in 0..=QUEUE_LEN {
        match board.next_error()? {
            Some(e) => errors.push(e),
            None => break,
        }
    }
    if cli.json {
        output::print_json(&errors)
    } else {
        output::print_errors(&errors);
        Ok(())
    }
}
