#!/usr/bin/env python3
"""Hardware smoke test for the Pico-Relay-B SCPI firmware.

The test switches all eight relays. Disconnect loads that must not
switch before you start it.

Usage:
    nix develop -c python tools/smoke.py [--port /dev/ttyACM0] [--yes]
"""

import argparse
import sys
import time

import pyvisa

IDN_MARK = "Pico-Relay-B"
CHANNELS = range(1, 9)


class Failure(Exception):
    pass


def check(what, got, expected):
    if got != expected:
        raise Failure(f"{what}: got {got!r}, expected {expected!r}")
    print(f"  ok  {what}")


def open_board(rm, port):
    """Open the board. Without a port, find it with *IDN?."""
    if port:
        names = [f"ASRL{port}::INSTR"]
    else:
        names = [n for n in rm.list_resources() if "ttyACM" in n or "usbmodem" in n]
    for name in names:
        try:
            inst = rm.open_resource(name)
        except pyvisa.Error:
            continue
        inst.read_termination = "\n"
        inst.write_termination = "\n"
        inst.timeout = 2000
        try:
            idn = inst.query("*IDN?").strip()
        except pyvisa.Error:
            inst.close()
            continue
        if IDN_MARK in idn:
            print(f"found {idn} at {name}")
            return inst
        inst.close()
    raise Failure("no Pico-Relay-B found. Use --port to give the port.")


def run(inst):
    q = lambda cmd: inst.query(cmd).strip()  # noqa: E731

    print("reset")
    inst.write("*RST;*CLS")
    check("*OPC?", q("*OPC?"), "1")
    check("all open", q("ROUT:CLOS? (@1:8)"), "0,0,0,0,0,0,0,0")
    check("*TST?", q("*TST?"), "0")
    check("SYST:VERS?", q("SYST:VERS?"), "1999.0")

    print("one channel at a time")
    for ch in CHANNELS:
        inst.write(f"ROUT:CLOS (@{ch})")
        expected = ",".join("1" if c == ch else "0" for c in CHANNELS)
        check(f"CH{ch} closed", q("ROUT:CLOS? (@1:8)"), expected)
        time.sleep(0.2)
        inst.write(f"ROUT:OPEN (@{ch})")
        check(f"CH{ch} open", q(f"ROUT:OPEN? (@{ch})"), "1")

    print("channel list")
    inst.write("ROUT:CLOS (@1,3,5:8)")
    check("list closed", q("ROUT:CLOS? (@1:8)"), "1,0,1,0,1,1,1,1")
    check("relative header", q("ROUT:OPEN (@5:6);CLOS? (@4:7)"), "0,0,0,1")
    inst.write("ROUT:OPEN:ALL")
    check("all open", q("ROUT:CLOS? (@1:8)"), "0,0,0,0,0,0,0,0")

    print("errors")
    check("no error", q("SYST:ERR?"), '0,"No error"')
    inst.write("BOGUS:CMD")
    check("undefined header", q("SYST:ERR?"), '-113,"Undefined header"')
    inst.write("ROUT:CLOS (@9)")
    check("out of range", q("SYST:ERR?"), '-222,"Data out of range"')
    check("queue empty", q("SYST:ERR:NEXT?"), '0,"No error"')

    print("beeper")
    check("beeper off after *RST", q("SYST:BEEP:STAT?"), "0")
    inst.write("SYST:BEEP")
    inst.write("SYST:BEEP:STAT ON")
    check("beeper on", q("SYST:BEEP:STAT?"), "1")
    inst.write("SYST:BEEP:STAT OFF")

    print("final reset")
    inst.write("*RST")
    check("all open", q("ROUT:CLOS? (@1:8)"), "0,0,0,0,0,0,0,0")
    check("no error", q("SYST:ERR?"), '0,"No error"')


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", help="serial port, for example /dev/ttyACM0")
    parser.add_argument("--yes", action="store_true", help="do not ask before the relays switch")
    args = parser.parse_args()

    if not args.yes:
        answer = input("This test switches all eight relays. Continue? [y/N] ")
        if answer.strip().lower() != "y":
            return 1

    rm = pyvisa.ResourceManager("@py")
    try:
        inst = open_board(rm, args.port)
        try:
            run(inst)
        finally:
            inst.write("*RST")
            inst.close()
    except Failure as err:
        print(f"FAIL: {err}")
        return 1
    print("PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
