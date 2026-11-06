# clack

This firmware turns the Waveshare Pico-Relay-B into an SCPI instrument. The board
must have a Raspberry Pi Pico 2 (RP2350). The host sends commands over USB
CDC-ACM serial. pyvisa opens the board as `ASRL/dev/ttyACM0::INSTR`.

The `clack` command-line client finds the connected boards and switches
their relays. See [The clack client](#the-clack-client).

## Hardware

| Function | GPIO |
|----------|------|
| Relay CH1 to CH8 | 21, 20, 19, 18, 17, 16, 15, 14 (high closes the relay) |
| WS2812 RGB LED | 13 |
| Buzzer | 6 (PWM slice 3, channel A) |

The board also has these parts. The Pico connects to them through its test
pads on the lower side.

| Part | Pico connection |
|------|-----------------|
| BOOT button (Key0) | BOOTSEL test pad |
| RUN button (Key1) | RUN, pin 30. The button resets the chip. |
| Micro USB connector | USB D+ and D- test pads |

To start the boot ROM (BOOTSEL mode), hold BOOT, push and release RUN, then
release BOOT. The buttons operate without help from the firmware. The micro
USB connector on the board and the USB connector on the Pico use the same
USB lines. Connect only one of them at a time.

The firmware opens all relays at power-on, after `*RST` and after a watchdog
reset. The relays keep their state when the host closes the port.

With eight closed relays, the board takes approximately 0.9 A at 5 V. A USB
2.0 port supplies 500 mA. If more than a few relays stay closed at the same
time, connect a 5 V supply of 1 A or more to P17. The board connects P17
directly to the 5 V line of the micro USB connector on the board, thus use
only one of these two.

## Commands

The board accepts the short form (`ROUT:CLOS`) and the long form
(`ROUTE:CLOSE`) in upper or lower case. A line ends with `\n`, `\r\n` or
`\r`. A `;` separates commands on one line. The board does not echo input. A
reply ends with `\n`. In a terminal program, turn on the local echo, for
example `picocom --echo /dev/ttyACM0`.

| Command | Action |
|---------|--------|
| `*IDN?` | Gives `Waveshare,Pico-Relay-B,<chip ID>,<version>`. |
| `*RST` | Opens all relays and sets the beeper state to OFF. |
| `*CLS` | Clears the error queue. |
| `*OPC?` | Gives `1`. |
| `*TST?` | Gives `0`. |
| `ROUTe:CLOSe <list>` | Closes the relays in the list. |
| `ROUTe:OPEN <list>` | Opens the relays in the list. |
| `ROUTe:CLOSe? <list>` | Gives `1` for each closed relay and `0` for each open relay. |
| `ROUTe:OPEN? <list>` | Gives `1` for each open relay and `0` for each closed relay. |
| `ROUTe:OPEN:ALL` | Opens all relays. |
| `SYSTem:ERRor[:NEXT]?` | Gives the oldest error, for example `-113,"Undefined header"`. |
| `SYSTem:VERSion?` | Gives `1999.0`. |
| `SYSTem:BEEPer[:IMMediate]` | Makes one beep. |
| `SYSTem:BEEPer:STATe ON\|OFF` | Turns the beep on each relay change on or off. |
| `SYSTem:BEEPer:STATe?` | Gives `1` or `0`. |

A channel list has the form `(@1,3,5:8)`. The channel numbers are 1 to 8, the
same as the CH1 to CH8 labels on the board. Relays in one command switch at
the same time.

When a command has an error, the board puts the error in the queue. It then
ignores the remaining commands on that line. The error queue holds 10
errors. An input line can have a maximum of 256 bytes.

Example session:

```
*IDN?
Waveshare,Pico-Relay-B,E66038B713554D2F,0.1.0
ROUT:CLOS (@1,3);CLOS? (@1:4)
1,0,1,0
SYST:ERR?
0,"No error"
```

## LED

| Colour | Status |
|--------|--------|
| Dim green | No host has the port open. |
| Blue | A host has the port open and sets DTR. |
| Red | The error queue is not empty. Read it with `SYST:ERR?` or clear it with `*CLS`. |

## The clack client

`clack` finds the boards by their USB ID and product string, and makes sure
of each board with `*IDN?`. If the computer has only one board, `clack` uses it.
With more boards, give `--board` with a board name from the config file or
the start of the serial number. `--port` selects a serial port directly.

| Command | Action |
|---------|--------|
| `clack list` | Shows the connected boards. |
| `clack status` | Shows the state of each relay and of the beeper. |
| `clack close 1,3,5-8` | Closes relays. `all` selects all relays. |
| `clack open all` | Opens relays. |
| `clack toggle 1-4` | Closes the open relays and opens the closed relays in the list. |
| `clack pulse 3 --ms 300` | Closes relays, waits, then opens them. The default time is 200 ms. |
| `clack rst` | Opens all relays and sets the beeper to off (`*RST`). |
| `clack beep on\|off\|once` | Sets the beeper, or makes one beep. |
| `clack errors` | Shows and clears the error queue of the board. |

Give `--json` to get the output of `list`, `status` and `errors` as JSON.

All relays in one command switch at the same time. Before a command that
changes the board, `clack` clears the error queue of the board. After the
command, it reads the queue. If the board reports an error, `clack` shows it
and stops with exit code 1. `status` and `errors` do not clear the queue.

When `clack` opens the port, it discards old replies that an earlier
program did not read.

`pulse` is for a relay across a button, for example the reset button of a
different board. If a relay in the list is already closed, `pulse` changes
nothing and stops with an error. Ctrl-C or SIGTERM during the pulse opens
the relays, and `clack` stops with exit code 130.

### Config file

The config file is optional. It gives names to boards and channels. `clack`
reads `$XDG_CONFIG_HOME/clack/config.toml`, or `~/.config/clack/config.toml`.
`--config` gives a different file.

```toml
[boards]
bench = "4E1F2A9C7B305D18"

[channels]
dut1-reset = { board = "bench", channel = 2, pulse_ms = 300 }
dut1-power = { board = "bench", channel = 4 }
```

A channel name selects its board, thus `clack pulse dut1-reset` needs no
`--board`. A list can have names and numbers, for example
`clack close dut1-power,5`. All names in one list must be on the same
board. `pulse_ms` sets the pulse time of the channel. `--ms` overrides it.
`status` and `list` show the names.

## Build and flash

All tools come from the Nix flake.

```
nix build                  # result/clack.uf2 and result/bin/clack.elf
nix build .#clack          # the client: result/bin/clack
nix flake check            # tests, clippy, rustfmt, the firmware and the client
nix run .#flash            # flash with a debug probe (probe-rs)
nix run .#flash-uf2        # flash in BOOTSEL mode (picotool)
nix run . -- status        # run the client (the default app)
```

For development, use the dev shell. It also has the `clack` client.

```
nix develop
cd firmware && cargo run --release      # flash and show the defmt log
cd relay-scpi && cargo test             # parser tests on the host
cd cli && cargo test                    # client tests on the host
python tools/smoke.py                   # hardware test, switches all relays
```

The USB ID is `2E8A:0009`, the same as the Waveshare test firmware for the
Pico 2. The USB serial number is the chip ID. Thus
`/dev/serial/by-id/` has a stable name for each board.

## NixOS module

The flake has a NixOS module. It installs `clack` and a udev rule for the
boards. The rule matches the USB ID and the product string
`Pico-Relay-B SCPI`, thus other Pico SDK devices with the same USB ID keep
their usual permissions.

```nix
{
  inputs.clack.url = "github:meck/clack";

  outputs = { nixpkgs, clack, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      modules = [
        clack.nixosModules.default
        {
          programs.clack.enable = true;
          # programs.clack.group = "dialout";
          # programs.clack.flashing.enable = true;
        }
      ];
    };
  };
}
```

| Option | Default | Effect |
|--------|---------|--------|
| `programs.clack.enable` | `false` | Installs `clack` and the udev rule for the boards. |
| `programs.clack.group` | `"dialout"` | The group of the board ports. The module makes the group if it does not exist. |
| `programs.clack.flashing.enable` | `false` | Adds udev rules for the boot ROM in BOOTSEL mode (`2E8A:000F`) and the Raspberry Pi Debug Probe (`2E8A:000C`). Also installs picotool and probe-rs. |
| `programs.clack.package` | the flake package | The `clack` package. |

The user at the seat gets access through the `uaccess` tag. For access over
SSH or from a service, add the user to the group.

## Layout

- `relay-scpi/` is the SCPI parser and the instrument state. It has no
  hardware code, and its tests run on the host.
- `firmware/` is the Embassy firmware for the RP2350.
- `cli/` is the `clack` client. Its tests use the parser of `relay-scpi` as
  a board in memory.
- `nix/` has the NixOS module and its VM test.
- `.github/workflows/check.yml` runs `nix flake check` on GitHub for each
  push and pull request.
- `tools/smoke.py` is a pyvisa test for the real board.
