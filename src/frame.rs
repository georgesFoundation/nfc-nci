//! NCI packet format (NCI 2.x §3.4).
//!
//! Every packet is a 3-byte header followed by 0-255 payload bytes:
//!
//! ```text
//! Control packet:            Data packet:
//! [7:5] MT  [4] PBF          [7:5] MT=000b  [4] PBF
//! [3:0] GID                  [3:0] Conn ID
//! [5:0] OID                  RFU (0x00)
//! [7:0] Payload length       [7:0] Payload length
//! ```

use crate::error::{Error, Result};

/// NCI header size in bytes.
pub const HEADER_SIZE: usize = 3;

/// Maximum NCI payload carried by a single packet (single length byte).
pub const MAX_PAYLOAD_SIZE: usize = 255;

/// Maximum size of a complete NCI packet (header + payload).
pub const MAX_FRAME_SIZE: usize = HEADER_SIZE + MAX_PAYLOAD_SIZE;

/// NCI Message Type (MT field of the packet header).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MessageType {
    /// Data packet exchanged with the remote endpoint over a logical
    /// connection.
    Data,
    /// Command from the host to the NFC controller.
    Command,
    /// Response from the NFC controller to a command.
    Response,
    /// Notification sent asynchronously by the NFC controller.
    Notification,
}

impl MessageType {
    pub const fn bits(self) -> u8 {
        match self {
            MessageType::Data => 0b000,
            MessageType::Command => 0b001,
            MessageType::Response => 0b010,
            MessageType::Notification => 0b011,
        }
    }

    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits & 0b111 {
            0b000 => Some(MessageType::Data),
            0b001 => Some(MessageType::Command),
            0b010 => Some(MessageType::Response),
            0b011 => Some(MessageType::Notification),
            _ => None,
        }
    }
}

/// NCI Group Identifiers.
pub mod gid {
    /// NCI Core group.
    pub const CORE: u8 = 0x0;
    /// RF Management group.
    pub const RF: u8 = 0x1;
    /// NFCEE Management group.
    pub const NFCEE: u8 = 0x2;
    /// Proprietary group (`2F`/`4F`/`6F` on the wire). Its opcodes belong to
    /// each controller vendor.
    pub const PROPRIETARY: u8 = 0xF;
}

/// NCI Core group opcodes (GID = [`gid::CORE`]).
pub mod core_oid {
    pub const RESET: u8 = 0x00;
    pub const INIT: u8 = 0x01;
    pub const SET_CONFIG: u8 = 0x02;
    pub const GET_CONFIG: u8 = 0x03;
    pub const CONN_CREATE: u8 = 0x04;
    pub const CONN_CLOSE: u8 = 0x05;
    /// Notification only.
    pub const CONN_CREDITS: u8 = 0x06;
    /// Notification only.
    pub const GENERIC_ERROR: u8 = 0x07;
    /// Notification only.
    pub const INTERFACE_ERROR: u8 = 0x08;
    pub const SET_POWER_SUB_STATE: u8 = 0x09;
}

/// RF Management group opcodes (GID = [`gid::RF`]).
pub mod rf_oid {
    pub const DISCOVER_MAP: u8 = 0x00;
    pub const SET_LISTEN_MODE_ROUTING: u8 = 0x01;
    pub const GET_LISTEN_MODE_ROUTING: u8 = 0x02;
    pub const DISCOVER: u8 = 0x03;
    pub const DISCOVER_SELECT: u8 = 0x04;
    /// Notification only.
    pub const INTF_ACTIVATED: u8 = 0x05;
    pub const DEACTIVATE: u8 = 0x06;
    /// Notification only.
    pub const FIELD_INFO: u8 = 0x07;
    pub const T3T_POLLING: u8 = 0x08;
    /// Notification only.
    pub const NFCEE_ACTION: u8 = 0x09;
    /// Notification only.
    pub const NFCEE_DISCOVERY_REQ: u8 = 0x0A;
}

/// NFCEE Management group opcodes (GID = [`gid::NFCEE`]).
pub mod nfcee_oid {
    pub const DISCOVER: u8 = 0x00;
    pub const MODE_SET: u8 = 0x01;
    /// Notification only.
    pub const STATUS: u8 = 0x02;
    pub const POWER_AND_LINK_CNTRL: u8 = 0x03;
}

/// NCI status codes (NCI 2.0 Table 128). Vendor codes come through as
/// [`Status::Other`], for the controller's own driver to interpret.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Ok,
    Rejected,
    RfFrameCorrupted,
    Failed,
    NotInitialized,
    SyntaxError,
    SemanticError,
    InvalidParam,
    MessageSizeExceeded,
    // RF Discovery group
    DiscoveryAlreadyStarted,
    DiscoveryTargetActivationFailed,
    DiscoveryTearDown,
    // RF Interface group
    RfTransmissionError,
    RfProtocolError,
    RfTimeoutError,
    // NFCEE Interface group
    NfceeInterfaceActivationFailed,
    NfceeTransmissionError,
    NfceeProtocolError,
    NfceeTimeoutError,
    /// Any status code not covered above, including proprietary ones.
    Other(u8),
}

impl From<u8> for Status {
    fn from(value: u8) -> Self {
        match value {
            0x00 => Status::Ok,
            0x01 => Status::Rejected,
            0x02 => Status::RfFrameCorrupted,
            0x03 => Status::Failed,
            0x04 => Status::NotInitialized,
            0x05 => Status::SyntaxError,
            0x06 => Status::SemanticError,
            0x09 => Status::InvalidParam,
            0x0A => Status::MessageSizeExceeded,
            0xA0 => Status::DiscoveryAlreadyStarted,
            0xA1 => Status::DiscoveryTargetActivationFailed,
            0xA2 => Status::DiscoveryTearDown,
            0xB0 => Status::RfTransmissionError,
            0xB1 => Status::RfProtocolError,
            0xB2 => Status::RfTimeoutError,
            0xC0 => Status::NfceeInterfaceActivationFailed,
            0xC1 => Status::NfceeTransmissionError,
            0xC2 => Status::NfceeProtocolError,
            0xC3 => Status::NfceeTimeoutError,
            other => Status::Other(other),
        }
    }
}

impl Status {
    /// Convert a status byte to a `Result`, treating anything that is not
    /// `STATUS_OK` as an error.
    pub fn ensure_ok(value: u8) -> Result<()> {
        match Status::from(value) {
            Status::Ok => Ok(()),
            status => Err(Error::Nci(status)),
        }
    }
}

/// `CORE_RESET_CMD` Reset Type (NCI 2.0 §4.1).
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ResetType {
    /// Keep the current NCI RF configuration.
    KeepConfig = 0x00,
    /// Reset the NCI RF configuration to its defaults.
    #[default]
    ResetConfig = 0x01,
}

/// A single NCI packet (control or data).
///
/// For control packets `id` is the GID and `oid` the OID. For data packets
/// `id` is the logical connection ID and `oid` is unused (RFU octet, 0).
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub mt: MessageType,
    /// Packet Boundary Flag: `true` when more segments of the same message
    /// follow.
    pub pbf: bool,
    /// GID (control packets) or Conn ID (data packets).
    pub id: u8,
    /// OID (control packets), 0 for data packets.
    pub oid: u8,
    pub payload: heapless::Vec<u8, MAX_PAYLOAD_SIZE>,
}

impl Frame {
    /// Build a command frame.
    pub fn command(gid: u8, oid: u8, payload: &[u8]) -> Result<Self> {
        let mut frame = Frame {
            mt: MessageType::Command,
            pbf: false,
            id: gid & 0x0F,
            oid: oid & 0x3F,
            payload: heapless::Vec::new(),
        };
        frame
            .payload
            .extend_from_slice(payload)
            .map_err(|_| Error::BufferOverflow)?;
        Ok(frame)
    }

    /// Build a data frame on a logical connection.
    pub fn data(conn_id: u8, pbf: bool, payload: &[u8]) -> Result<Self> {
        let mut frame = Frame {
            mt: MessageType::Data,
            pbf,
            id: conn_id & 0x0F,
            oid: 0,
            payload: heapless::Vec::new(),
        };
        frame
            .payload
            .extend_from_slice(payload)
            .map_err(|_| Error::BufferOverflow)?;
        Ok(frame)
    }

    /// Encode the 3-byte NCI header of this frame.
    pub fn header(&self) -> [u8; HEADER_SIZE] {
        [
            (self.mt.bits() << 5) | ((self.pbf as u8) << 4) | self.id,
            self.oid,
            self.payload.len() as u8,
        ]
    }

    /// Parse the packet fields from a raw 3-byte header.
    ///
    /// Returns `(mt, pbf, id, oid, payload_len)`.
    pub fn parse_header(header: [u8; HEADER_SIZE]) -> Result<(MessageType, bool, u8, u8, usize)> {
        let mt = MessageType::from_bits(header[0] >> 5).ok_or(Error::UnexpectedFrame {
            mt: header[0] >> 5,
            gid: header[0] & 0x0F,
            oid: header[1] & 0x3F,
        })?;
        Ok((
            mt,
            header[0] & 0x10 != 0,
            header[0] & 0x0F,
            header[1] & 0x3F,
            header[2] as usize,
        ))
    }

    /// `true` when this frame is the response to command `gid`/`oid`.
    pub fn is_response(&self, gid: u8, oid: u8) -> bool {
        self.mt == MessageType::Response && self.id == gid && self.oid == oid
    }

    /// `true` when this frame is the notification `gid`/`oid`.
    pub fn is_notification(&self, gid: u8, oid: u8) -> bool {
        self.mt == MessageType::Notification && self.id == gid && self.oid == oid
    }

    /// `true` when this frame is a data packet on the given connection.
    pub fn is_data(&self, conn_id: u8) -> bool {
        self.mt == MessageType::Data && self.id == conn_id
    }

    /// Return an [`Error::UnexpectedFrame`] describing this frame.
    pub fn unexpected(&self) -> Error {
        Error::UnexpectedFrame {
            mt: self.mt.bits(),
            gid: self.id,
            oid: self.oid,
        }
    }

    /// Validate that the first payload byte is `STATUS_OK`.
    ///
    /// Most NCI responses put a status code in the first payload octet.
    pub fn ensure_status_ok(&self) -> Result<()> {
        match self.payload.first() {
            Some(&status) => Status::ensure_ok(status),
            None => Err(Error::InvalidResponseLength {
                expected: 1,
                actual: 0,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_header_encoding() {
        let frame = Frame::command(gid::CORE, core_oid::RESET, &[0x01]).unwrap();
        assert_eq!(frame.header(), [0x20, 0x00, 0x01]);

        let frame = Frame::command(gid::RF, rf_oid::DISCOVER, &[0x01, 0x00, 0x01]).unwrap();
        assert_eq!(frame.header(), [0x21, 0x03, 0x03]);

        let frame = Frame::command(gid::PROPRIETARY, 0x02, &[]).unwrap();
        assert_eq!(frame.header(), [0x2F, 0x02, 0x00]);
    }

    #[test]
    fn data_header_encoding() {
        let frame = Frame::data(0, false, &[0x30, 0x00]).unwrap();
        assert_eq!(frame.header(), [0x00, 0x00, 0x02]);

        let chained = Frame::data(0, true, &[0xAA]).unwrap();
        assert_eq!(chained.header(), [0x10, 0x00, 0x01]);
    }

    #[test]
    fn header_parsing() {
        // CORE_RESET_RSP
        let (mt, pbf, gid, oid, len) = Frame::parse_header([0x40, 0x00, 0x01]).unwrap();
        assert_eq!(mt, MessageType::Response);
        assert!(!pbf);
        assert_eq!((gid, oid, len), (0x00, 0x00, 1));

        // RF_INTF_ACTIVATED_NTF
        let (mt, _, gid, oid, _) = Frame::parse_header([0x61, 0x05, 0x10]).unwrap();
        assert_eq!(mt, MessageType::Notification);
        assert_eq!((gid, oid), (gid::RF, rf_oid::INTF_ACTIVATED));

        // Chained data packet
        let (mt, pbf, conn, _, len) = Frame::parse_header([0x10, 0x00, 0xFF]).unwrap();
        assert_eq!(mt, MessageType::Data);
        assert!(pbf);
        assert_eq!((conn, len), (0, 255));

        // MT 0b111 is not a message type.
        assert!(Frame::parse_header([0xE0, 0x00, 0x00]).is_err());
    }

    #[test]
    fn status_conversion() {
        assert_eq!(Status::from(0x00), Status::Ok);
        assert_eq!(Status::from(0x03), Status::Failed);
        assert_eq!(Status::from(0x05), Status::SyntaxError);
        assert_eq!(Status::from(0xB2), Status::RfTimeoutError);
        assert_eq!(Status::from(0xE6), Status::Other(0xE6));
        assert!(Status::ensure_ok(0x00).is_ok());
        assert_eq!(
            Status::ensure_ok(0x06),
            Err(Error::Nci(Status::SemanticError))
        );
    }
}
