//! Splits a line into SCPI program message units and finds the command
//! for each unit.

use heapless::Vec;

use crate::error::ScpiError;

/// The maximum number of mnemonics in one header.
pub const MAX_DEPTH: usize = 6;

/// A command that the instrument knows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Command {
    /// `*IDN?`
    Identify,
    /// `*RST`
    Reset,
    /// `*CLS`
    ClearStatus,
    /// `*OPC?`
    OperationComplete,
    /// `*TST?`
    SelfTest,
    /// `ROUTe:CLOSe <list>`
    RouteClose,
    /// `ROUTe:OPEN <list>`
    RouteOpen,
    /// `ROUTe:CLOSe? <list>`
    RouteCloseQuery,
    /// `ROUTe:OPEN? <list>`
    RouteOpenQuery,
    /// `ROUTe:OPEN:ALL`
    RouteOpenAll,
    /// `SYSTem:ERRor[:NEXT]?`
    ErrorQuery,
    /// `SYSTem:VERSion?`
    VersionQuery,
    /// `SYSTem:BEEPer[:IMMediate]`
    Beep,
    /// `SYSTem:BEEPer:STATe <bool>`
    BeepState,
    /// `SYSTem:BEEPer:STATe?`
    BeepStateQuery,
}

/// One program message unit: a command and its parameter text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Unit<'a> {
    pub command: Command,
    /// The parameter text without spaces at the two ends. It is empty if
    /// the unit has no parameters.
    pub params: &'a str,
}

/// One node in a command header pattern.
struct Node {
    /// The long form. The upper case letters are the short form.
    name: &'static str,
    optional: bool,
}

const fn req(name: &'static str) -> Node {
    Node {
        name,
        optional: false,
    }
}

const fn opt(name: &'static str) -> Node {
    Node {
        name,
        optional: true,
    }
}

struct Entry {
    path: &'static [Node],
    query: bool,
    command: Command,
}

const TREE: &[Entry] = &[
    Entry {
        path: &[req("ROUTe"), req("CLOSe")],
        query: false,
        command: Command::RouteClose,
    },
    Entry {
        path: &[req("ROUTe"), req("CLOSe")],
        query: true,
        command: Command::RouteCloseQuery,
    },
    Entry {
        path: &[req("ROUTe"), req("OPEN")],
        query: false,
        command: Command::RouteOpen,
    },
    Entry {
        path: &[req("ROUTe"), req("OPEN")],
        query: true,
        command: Command::RouteOpenQuery,
    },
    Entry {
        path: &[req("ROUTe"), req("OPEN"), req("ALL")],
        query: false,
        command: Command::RouteOpenAll,
    },
    Entry {
        path: &[req("SYSTem"), req("ERRor"), opt("NEXT")],
        query: true,
        command: Command::ErrorQuery,
    },
    Entry {
        path: &[req("SYSTem"), req("VERSion")],
        query: true,
        command: Command::VersionQuery,
    },
    Entry {
        path: &[req("SYSTem"), req("BEEPer"), opt("IMMediate")],
        query: false,
        command: Command::Beep,
    },
    Entry {
        path: &[req("SYSTem"), req("BEEPer"), req("STATe")],
        query: false,
        command: Command::BeepState,
    },
    Entry {
        path: &[req("SYSTem"), req("BEEPer"), req("STATe")],
        query: true,
        command: Command::BeepStateQuery,
    },
];

/// The IEEE 488.2 common commands: name without `*`, query, command.
const COMMON: &[(&str, bool, Command)] = &[
    ("IDN", true, Command::Identify),
    ("RST", false, Command::Reset),
    ("CLS", false, Command::ClearStatus),
    ("OPC", true, Command::OperationComplete),
    ("TST", true, Command::SelfTest),
];

/// Gives the units of one line, one at a time.
///
/// The parser keeps the current header path. After a `;`, a header that
/// does not start with `:` or `*` starts at the parent node of the last
/// header. Example: `ROUT:CLOS (@1);OPEN (@2)` is `ROUT:CLOS (@1)` and
/// then `ROUT:OPEN (@2)`.
pub struct Parser<'a> {
    rest: Option<&'a str>,
    path: Vec<&'a str, MAX_DEPTH>,
}

impl<'a> Parser<'a> {
    pub fn new(line: &'a str) -> Self {
        Self {
            rest: Some(line),
            path: Vec::new(),
        }
    }

    fn unit(&mut self, text: &'a str) -> Result<Unit<'a>, ScpiError> {
        let text = text.trim();
        let (header, params) = text
            .split_once(|c: char| c.is_ascii_whitespace())
            .map_or((text, ""), |(h, p)| (h, p.trim()));
        let (header, query) = match header.strip_suffix('?') {
            Some(h) => (h, true),
            None => (header, false),
        };

        if let Some(name) = header.strip_prefix('*') {
            check_mnemonic(name)?;
            return COMMON
                .iter()
                .find(|(n, q, _)| *q == query && name.eq_ignore_ascii_case(n))
                .map(|&(_, _, command)| Unit { command, params })
                .ok_or(ScpiError::UndefinedHeader);
        }

        let (header, absolute) = match header.strip_prefix(':') {
            Some(h) => (h, true),
            None => (header, false),
        };
        let mut full: Vec<&'a str, MAX_DEPTH> = if absolute {
            Vec::new()
        } else {
            self.path.clone()
        };
        for mnemonic in header.split(':') {
            check_mnemonic(mnemonic)?;
            full.push(mnemonic)
                .map_err(|_| ScpiError::UndefinedHeader)?;
        }

        self.path = full.clone();
        self.path.pop();

        TREE.iter()
            .find(|e| e.query == query && matches_path(e.path, &full))
            .map(|e| Unit {
                command: e.command,
                params,
            })
            .ok_or(ScpiError::UndefinedHeader)
    }
}

impl<'a> Iterator for Parser<'a> {
    type Item = Result<Unit<'a>, ScpiError>;

    fn next(&mut self) -> Option<Self::Item> {
        let rest = self.rest?;
        let (text, next) = match rest.split_once(';') {
            Some((a, b)) => (a, Some(b)),
            None => (rest, None),
        };
        self.rest = next;

        if text.trim().is_empty() {
            // An empty line, or a `;` at the end of the line, is not an
            // error. An empty unit between two `;` is an error.
            next?;
            self.rest = None;
            return Some(Err(ScpiError::SyntaxError));
        }
        Some(self.unit(text))
    }
}

/// Checks the characters of one mnemonic.
fn check_mnemonic(m: &str) -> Result<(), ScpiError> {
    let mut bytes = m.bytes();
    match bytes.next() {
        None => Err(ScpiError::SyntaxError),
        Some(b) if !b.is_ascii_alphabetic() => Err(ScpiError::InvalidCharacter),
        Some(_) if bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_') => Ok(()),
        Some(_) => Err(ScpiError::InvalidCharacter),
    }
}

/// Compares input mnemonics with a header pattern. A node can be optional.
fn matches_path(nodes: &[Node], input: &[&str]) -> bool {
    match nodes.split_first() {
        None => input.is_empty(),
        Some((node, rest)) => {
            let here = input.split_first().is_some_and(|(m, tail)| {
                matches_mnemonic(node.name, m) && matches_path(rest, tail)
            });
            here || (node.optional && matches_path(rest, input))
        }
    }
}

/// Compares one input mnemonic with a long form such as `CLOSe`.
/// The input can be the short form (`CLOS`) or the long form (`CLOSE`).
/// Upper and lower case are the same.
fn matches_mnemonic(long: &str, input: &str) -> bool {
    let short_len = long
        .bytes()
        .position(|b| b.is_ascii_lowercase())
        .unwrap_or(long.len());
    input.eq_ignore_ascii_case(long)
        || long
            .get(..short_len)
            .is_some_and(|short| input.eq_ignore_ascii_case(short))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use Command::*;
    use std::vec::Vec as StdVec;

    fn parse(line: &str) -> StdVec<Result<(Command, &str), ScpiError>> {
        Parser::new(line)
            .map(|r| r.map(|u| (u.command, u.params)))
            .collect()
    }

    fn one(line: &str) -> Result<(Command, &str), ScpiError> {
        let v = parse(line);
        assert_eq!(v.len(), 1, "{line}");
        v[0]
    }

    #[test]
    fn short_and_long_forms() {
        for h in [
            "ROUT:CLOS",
            "ROUTE:CLOSE",
            "rout:clos",
            "Route:Close",
            ":ROUT:CLOS",
        ] {
            assert_eq!(
                one(&std::format!("{h} (@1)")),
                Ok((RouteClose, "(@1)")),
                "{h}"
            );
        }
        assert_eq!(one("ROUTE:CLOS? (@1)"), Ok((RouteCloseQuery, "(@1)")));
        assert_eq!(one("ROUT:OPEN:ALL"), Ok((RouteOpenAll, "")));
    }

    #[test]
    fn partial_forms_are_rejected() {
        assert_eq!(one("ROU:CLOS (@1)"), Err(ScpiError::UndefinedHeader));
        assert_eq!(one("ROUTE:CLO (@1)"), Err(ScpiError::UndefinedHeader));
        assert_eq!(one("ROUTX:CLOS (@1)"), Err(ScpiError::UndefinedHeader));
    }

    #[test]
    fn optional_nodes() {
        assert_eq!(one("SYST:ERR?"), Ok((ErrorQuery, "")));
        assert_eq!(one("SYST:ERR:NEXT?"), Ok((ErrorQuery, "")));
        assert_eq!(one("syst:beep"), Ok((Beep, "")));
        assert_eq!(one("SYST:BEEP:IMM"), Ok((Beep, "")));
        assert_eq!(one("SYST:BEEP:STAT ON"), Ok((BeepState, "ON")));
        assert_eq!(one("SYST:BEEP:STAT?"), Ok((BeepStateQuery, "")));
    }

    #[test]
    fn common_commands() {
        assert_eq!(one("*IDN?"), Ok((Identify, "")));
        assert_eq!(one("*idn?"), Ok((Identify, "")));
        assert_eq!(one("*RST"), Ok((Reset, "")));
        assert_eq!(one("*RST?"), Err(ScpiError::UndefinedHeader));
        assert_eq!(one("*IDN"), Err(ScpiError::UndefinedHeader));
        assert_eq!(one("*FOO"), Err(ScpiError::UndefinedHeader));
    }

    #[test]
    fn query_form_must_exist() {
        assert_eq!(one("ROUT:OPEN:ALL?"), Err(ScpiError::UndefinedHeader));
        assert_eq!(one("SYST:VERS"), Err(ScpiError::UndefinedHeader));
    }

    #[test]
    fn relative_headers() {
        assert_eq!(
            parse("ROUT:CLOS (@1);OPEN (@2)"),
            [Ok((RouteClose, "(@1)")), Ok((RouteOpen, "(@2)"))]
        );
        assert_eq!(
            parse("SYST:BEEP:STAT ON;STAT?"),
            [Ok((BeepState, "ON")), Ok((BeepStateQuery, ""))]
        );
        // A common command does not change the path.
        assert_eq!(
            parse("ROUT:CLOS (@1);*OPC?;OPEN (@1)"),
            [
                Ok((RouteClose, "(@1)")),
                Ok((OperationComplete, "")),
                Ok((RouteOpen, "(@1)"))
            ]
        );
        // A leading colon starts at the root.
        assert_eq!(
            parse("ROUT:CLOS (@1);:SYST:ERR?"),
            [Ok((RouteClose, "(@1)")), Ok((ErrorQuery, ""))]
        );
        // Without a colon, SYST is under ROUT and is not known.
        assert_eq!(
            parse("ROUT:CLOS (@1);SYST:ERR?"),
            [Ok((RouteClose, "(@1)")), Err(ScpiError::UndefinedHeader)]
        );
    }

    #[test]
    fn separators() {
        assert_eq!(parse("*RST;"), [Ok((Reset, ""))]);
        assert_eq!(parse("   "), []);
        assert_eq!(
            parse("*RST;;*CLS"),
            [Ok((Reset, "")), Err(ScpiError::SyntaxError)]
        );
    }

    #[test]
    fn bad_characters() {
        assert_eq!(one("ROUT::CLOS (@1)"), Err(ScpiError::SyntaxError));
        assert_eq!(one("ROUT:CL$S (@1)"), Err(ScpiError::InvalidCharacter));
        assert_eq!(one("1ROUT"), Err(ScpiError::InvalidCharacter));
    }

    #[test]
    fn whitespace_around_params() {
        assert_eq!(one("  ROUT:CLOS\t (@1, 2)  "), Ok((RouteClose, "(@1, 2)")));
    }
}
