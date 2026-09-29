//! Error handling.
//!
//! Errors are split in two levels:
//!
//! - **Driver errors**: problems detected on the host side (bus failures,
//!   timeouts, malformed frames, parameter validation).
//! - **NCI status errors** ([`Status`]): error codes the NFC controller
//!   reports in response and notification payloads, surfaced as
//!   [`Error::Nci`].

use {crate::frame::Status, derive_more::From};

/// Error type shared by the NCI core and the controller drivers built on it.
#[derive(Copy, Clone, Debug, Eq, From, PartialEq)]
pub enum Error {
    /// I2C bus error while talking to the controller.
    ///
    /// The original error is normalized to [`embedded_hal::i2c::ErrorKind`] so
    /// the driver stays generic over the concrete bus implementation.
    I2c(embedded_hal::i2c::ErrorKind),

    /// GPIO error on the IRQ, enable or reset pin.
    Gpio(embedded_hal::digital::ErrorKind),

    /// Timed out waiting for the IRQ pin to signal that the controller has
    /// data available for the host.
    ///
    /// In discovery this simply means no target showed up in the given window
    /// and is a normal, retryable condition.
    Timeout,

    /// The controller did not answer the start-up `CORE_RESET` exchange, or
    /// answered with something that is not a valid `CORE_RESET` response or
    /// notification. Usually a wiring problem (I2C address, IRQ, enable pin).
    Identification,

    /// The controller reset itself: a `CORE_RESET_NTF` arrived with this reset
    /// trigger where none was expected. Trigger `0x00` is an unrecoverable
    /// error inside the controller (NCI 2.0 Table 11); the host must reset it
    /// and start again.
    ControllerReset { trigger: u8 },

    /// The controller reported a non-OK NCI status code.
    #[from]
    Nci(Status),

    /// A frame (or an accumulated chained payload) does not fit in the
    /// driver's fixed-capacity buffers.
    BufferOverflow,

    /// Received an NCI packet that does not match what the current operation
    /// expects (wrong message type, GID or OID).
    UnexpectedFrame {
        /// Message type, GID and OID octets of the offending packet header.
        mt: u8,
        gid: u8,
        oid: u8,
    },

    /// A read returned only filler bytes (see
    /// [`Config::filler`](crate::Config::filler)): the controller had nothing
    /// to send.
    NoPacket,

    /// A response payload is shorter than the specification mandates.
    InvalidResponseLength { expected: usize, actual: usize },

    /// The remote endpoint (tag or reader) was deactivated / left the field
    /// while an operation was in progress.
    TargetLost,

    /// Too many unexpected notifications were received while waiting for a
    /// response; the exchange was aborted to avoid looping forever.
    NotificationFlood,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::I2c(kind) => write!(f, "I2C bus error: {kind}"),
            Error::Gpio(kind) => write!(f, "GPIO error: {kind}"),
            Error::Timeout => write!(f, "timed out waiting for the controller IRQ"),
            Error::Identification => write!(f, "NFC controller presence check failed"),
            Error::ControllerReset { trigger } => {
                write!(f, "NFC controller reset itself (trigger {trigger:#04x})")
            }
            Error::Nci(status) => write!(f, "NCI error status: {status:?}"),
            Error::BufferOverflow => write!(f, "frame does not fit in driver buffers"),
            Error::UnexpectedFrame { mt, gid, oid } => {
                write!(
                    f,
                    "unexpected NCI frame (mt={mt:#x} gid={gid:#x} oid={oid:#x})"
                )
            }
            Error::NoPacket => write!(f, "the controller had no packet to send"),
            Error::InvalidResponseLength { expected, actual } => {
                write!(
                    f,
                    "invalid response length (expected {expected}, got {actual})"
                )
            }
            Error::TargetLost => write!(f, "remote endpoint deactivated"),
            Error::NotificationFlood => write!(f, "too many unexpected notifications"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

impl Error {
    /// Normalize an I2C implementation error into [`Error::I2c`].
    pub fn i2c<E: embedded_hal::i2c::Error>(err: E) -> Self {
        Error::I2c(err.kind())
    }

    /// Normalize a GPIO implementation error into [`Error::Gpio`].
    pub fn gpio<E: embedded_hal::digital::Error>(err: E) -> Self {
        Error::Gpio(err.kind())
    }
}

/// Convenience alias used across the crate.
pub type Result<T> = core::result::Result<T, Error>;
