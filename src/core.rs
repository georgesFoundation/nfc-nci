//! Payloads of the NCI core messages a start-up sequence reads.

use crate::{
    error::{Error, Result},
    rf::RfInterface,
};

/// Largest manufacturer-specific block kept from a `CORE_RESET_NTF`.
pub const MAX_MANUFACTURER_INFO: usize = 64;

/// Largest number of RF interfaces kept from a `CORE_INIT_RSP`.
pub const MAX_INTERFACES: usize = 16;

/// `CORE_RESET_NTF` (NCI 2.0 §4.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResetNotification {
    /// Why the controller reset: `0x00` an unrecoverable error, `0x01`
    /// power-on, `0x02` a `CORE_RESET_CMD`; `0xA0` and up are proprietary.
    pub trigger: u8,
    /// `0x00`: NCI RF configuration kept, `0x01`: reset.
    pub config_status: u8,
    /// NCI version, major in the high nibble (`0x20` for 2.0).
    pub nci_version: u8,
    /// IC manufacturer code (ISO/IEC 7816-6): `0x02` STMicroelectronics,
    /// `0x04` NXP.
    pub manufacturer_id: u8,
    /// Manufacturer-specific information, for the controller's driver.
    pub manufacturer_info: heapless::Vec<u8, MAX_MANUFACTURER_INFO>,
}

impl ResetNotification {
    /// Parse the payload of a `CORE_RESET_NTF`.
    pub fn parse(payload: &[u8]) -> Result<Self> {
        if payload.len() < 5 {
            return Err(Error::InvalidResponseLength {
                expected: 5,
                actual: payload.len(),
            });
        }
        let len = payload[4] as usize;
        let info = payload
            .get(5..5 + len)
            .ok_or(Error::InvalidResponseLength {
                expected: 5 + len,
                actual: payload.len(),
            })?;
        Ok(ResetNotification {
            trigger: payload[0],
            config_status: payload[1],
            nci_version: payload[2],
            manufacturer_id: payload[3],
            manufacturer_info: heapless::Vec::from_slice(info)
                .map_err(|_| Error::BufferOverflow)?,
        })
    }
}

/// `CORE_INIT_RSP`, NCI 2.0 form (§4.2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitInfo {
    /// NFCC Features, four octets as sent.
    pub features: [u8; 4],
    /// Dynamic logical connections the controller supports.
    pub max_logical_connections: u8,
    /// Size of the listen-mode routing table, in bytes.
    pub max_routing_table_size: u16,
    /// Largest control packet payload the controller accepts.
    pub max_control_payload_size: u8,
    /// Largest data packet payload on the static HCI connection.
    pub max_hci_data_payload_size: u8,
    /// Credits the controller granted on the static HCI connection.
    pub hci_credits: u8,
    /// Largest NFC-V RF frame, in bytes.
    pub max_nfcv_frame_size: u16,
    /// RF interfaces the controller supports (their extensions are skipped).
    pub interfaces: heapless::Vec<RfInterface, MAX_INTERFACES>,
}

impl InitInfo {
    /// Parse the payload of a `CORE_INIT_RSP`, status byte included.
    pub fn parse(payload: &[u8]) -> Result<Self> {
        const FIXED: usize = 14;
        if payload.len() < FIXED {
            return Err(Error::InvalidResponseLength {
                expected: FIXED,
                actual: payload.len(),
            });
        }
        let mut interfaces = heapless::Vec::new();
        let count = payload[13] as usize;
        let mut offset = FIXED;
        for _ in 0..count {
            // Interface, number of extensions, extensions.
            let (Some(&interface), Some(&extensions)) =
                (payload.get(offset), payload.get(offset + 1))
            else {
                return Err(Error::InvalidResponseLength {
                    expected: offset + 2,
                    actual: payload.len(),
                });
            };
            interfaces
                .push(RfInterface::from(interface))
                .map_err(|_| Error::BufferOverflow)?;
            offset += 2 + extensions as usize;
        }
        Ok(InitInfo {
            features: [payload[1], payload[2], payload[3], payload[4]],
            max_logical_connections: payload[5],
            max_routing_table_size: u16::from_le_bytes([payload[6], payload[7]]),
            max_control_payload_size: payload[8],
            max_hci_data_payload_size: payload[9],
            hci_credits: payload[10],
            max_nfcv_frame_size: u16::from_le_bytes([payload[11], payload[12]]),
            interfaces,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_notification() {
        let ntf = ResetNotification::parse(&[0x01, 0x01, 0x20, 0x04, 0x02, 0xAA, 0xBB]).unwrap();
        assert_eq!(ntf.trigger, 0x01);
        assert_eq!(ntf.nci_version, 0x20);
        assert_eq!(ntf.manufacturer_id, 0x04);
        assert_eq!(ntf.manufacturer_info.as_slice(), &[0xAA, 0xBB]);

        // Manufacturer info shorter than announced.
        assert!(ResetNotification::parse(&[0x01, 0x01, 0x20, 0x04, 0x03, 0xAA]).is_err());
        assert!(ResetNotification::parse(&[0x01, 0x01]).is_err());
    }

    #[test]
    fn init_info() {
        let rsp = [
            0x00, // status
            0x01, 0x02, 0x03, 0x04, // features
            0x01, // logical connections
            0x00, 0x02, // routing table size, little endian
            0xFF, 0xFF, 0x00, // control payload, HCI data payload, HCI credits
            0x00, 0x01, // NFC-V frame size, little endian
            0x03, // interfaces
            0x01, 0x00, // Frame
            0x02, 0x01, 0x99, // ISO-DEP, one extension
            0x80, 0x00, // proprietary
        ];
        let info = InitInfo::parse(&rsp).unwrap();
        assert_eq!(info.features, [0x01, 0x02, 0x03, 0x04]);
        assert_eq!(info.max_routing_table_size, 0x0200);
        assert_eq!(info.max_nfcv_frame_size, 0x0100);
        assert_eq!(
            info.interfaces.as_slice(),
            &[
                RfInterface::Frame,
                RfInterface::IsoDep,
                RfInterface::Other(0x80)
            ]
        );

        // Four interfaces announced, three present.
        let mut short = rsp;
        short[13] = 4;
        assert!(InitInfo::parse(&short).is_err());
    }
}
