//! SCPI error codes and the error queue.

use heapless::Deque;

/// The number of entries in the error queue.
pub const QUEUE_LEN: usize = 10;

/// An SCPI error. The value is the standard SCPI error code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ScpiError {
    CommandError = -100,
    InvalidCharacter = -101,
    SyntaxError = -102,
    DataTypeError = -104,
    ParameterNotAllowed = -108,
    MissingParameter = -109,
    UndefinedHeader = -113,
    DataOutOfRange = -222,
    TooMuchData = -223,
    QueueOverflow = -350,
    InputBufferOverrun = -363,
}

impl ScpiError {
    /// Gives the standard SCPI error code.
    pub fn code(self) -> i16 {
        self as i16
    }

    /// Gives the standard SCPI error text.
    pub fn text(self) -> &'static str {
        match self {
            Self::CommandError => "Command error",
            Self::InvalidCharacter => "Invalid character",
            Self::SyntaxError => "Syntax error",
            Self::DataTypeError => "Data type error",
            Self::ParameterNotAllowed => "Parameter not allowed",
            Self::MissingParameter => "Missing parameter",
            Self::UndefinedHeader => "Undefined header",
            Self::DataOutOfRange => "Data out of range",
            Self::TooMuchData => "Too much data",
            Self::QueueOverflow => "Queue overflow",
            Self::InputBufferOverrun => "Input buffer overrun",
        }
    }
}

/// The SCPI error queue. The oldest error comes out first.
///
/// If the queue is full, the last entry changes to `QueueOverflow`.
/// The new error is then lost. This is the SCPI rule.
#[derive(Default)]
pub struct ErrorQueue {
    queue: Deque<ScpiError, QUEUE_LEN>,
}

impl ErrorQueue {
    pub const fn new() -> Self {
        Self {
            queue: Deque::new(),
        }
    }

    /// Adds an error to the end of the queue.
    pub fn push(&mut self, err: ScpiError) {
        if self.queue.is_full() {
            if let Some(last) = self.queue.back_mut() {
                *last = ScpiError::QueueOverflow;
            }
        } else {
            // The queue is not full, thus this push cannot fail.
            let _ = self.queue.push_back(err);
        }
    }

    /// Removes the oldest error from the queue and gives it.
    pub fn pop(&mut self) -> Option<ScpiError> {
        self.queue.pop_front()
    }

    /// Gives the oldest error. The error stays in the queue.
    pub fn peek(&self) -> Option<ScpiError> {
        self.queue.front().copied()
    }

    /// Removes all errors.
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_order() {
        let mut q = ErrorQueue::new();
        q.push(ScpiError::SyntaxError);
        q.push(ScpiError::UndefinedHeader);
        assert_eq!(q.pop(), Some(ScpiError::SyntaxError));
        assert_eq!(q.pop(), Some(ScpiError::UndefinedHeader));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn overflow_replaces_last_entry() {
        let mut q = ErrorQueue::new();
        for _ in 0..QUEUE_LEN + 3 {
            q.push(ScpiError::SyntaxError);
        }
        assert_eq!(q.len(), QUEUE_LEN);
        for _ in 0..QUEUE_LEN - 1 {
            assert_eq!(q.pop(), Some(ScpiError::SyntaxError));
        }
        assert_eq!(q.pop(), Some(ScpiError::QueueOverflow));
        assert!(q.is_empty());
    }

    #[test]
    fn codes() {
        assert_eq!(ScpiError::UndefinedHeader.code(), -113);
        assert_eq!(ScpiError::DataOutOfRange.code(), -222);
        assert_eq!(ScpiError::InputBufferOverrun.code(), -363);
    }
}
