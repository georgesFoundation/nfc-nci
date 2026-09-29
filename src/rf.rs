//! RF management types: technologies, protocols, interfaces, discovery
//! configuration and activated-target information.

use crate::error::{Error, Result};

/// RF interface used to exchange data with an activated remote endpoint
/// (NCI 2.0 Table 132). Proprietary interfaces (`0x80` and up) come through
/// as [`RfInterface::Other`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RfInterface {
    /// NFCEE direct: no RF interface.
    Undetermined,
    /// Raw frames, used for T1T/T2T/T3T/T5T tag commands.
    Frame,
    /// ISO-DEP (ISO/IEC 14443-4) block exchanges.
    IsoDep,
    /// NFC-DEP (peer-to-peer).
    NfcDep,
    /// NDEF: the controller reads or writes an NDEF message by itself.
    Ndef,
    Other(u8),
}

impl RfInterface {
    pub const fn value(self) -> u8 {
        match self {
            RfInterface::Undetermined => 0x00,
            RfInterface::Frame => 0x01,
            RfInterface::IsoDep => 0x02,
            RfInterface::NfcDep => 0x03,
            RfInterface::Ndef => 0x06,
            RfInterface::Other(value) => value,
        }
    }
}

impl From<u8> for RfInterface {
    fn from(value: u8) -> Self {
        match value {
            0x00 => RfInterface::Undetermined,
            0x01 => RfInterface::Frame,
            0x02 => RfInterface::IsoDep,
            0x03 => RfInterface::NfcDep,
            0x06 => RfInterface::Ndef,
            other => RfInterface::Other(other),
        }
    }
}

/// RF protocol of a remote endpoint (NCI 2.0 Table 131). Proprietary
/// protocols (`0x80` and up, such as MIFARE Classic) come through as
/// [`RfProtocol::Other`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RfProtocol {
    Undetermined,
    /// Type 1 Tag (Topaz / Jewel).
    T1t,
    /// Type 2 Tag (MIFARE Ultralight, NTAG, ...).
    T2t,
    /// Type 3 Tag (FeliCa).
    T3t,
    /// ISO-DEP (ISO/IEC 14443-4, Type 4 Tags).
    IsoDep,
    /// NFC-DEP (peer-to-peer).
    NfcDep,
    /// Type 5 Tag (ISO/IEC 15693 vicinity cards).
    T5t,
    /// NDEF, with the NDEF RF interface.
    Ndef,
    Other(u8),
}

impl RfProtocol {
    pub const fn value(self) -> u8 {
        match self {
            RfProtocol::Undetermined => 0x00,
            RfProtocol::T1t => 0x01,
            RfProtocol::T2t => 0x02,
            RfProtocol::T3t => 0x03,
            RfProtocol::IsoDep => 0x04,
            RfProtocol::NfcDep => 0x05,
            RfProtocol::T5t => 0x06,
            RfProtocol::Ndef => 0x07,
            RfProtocol::Other(value) => value,
        }
    }

    /// The standard RF interface for this protocol in reader mode: ISO-DEP
    /// and NFC-DEP get their own interface, everything else the Frame
    /// interface. Proprietary protocols need their controller's mapping.
    pub const fn default_interface(self) -> RfInterface {
        match self {
            RfProtocol::IsoDep => RfInterface::IsoDep,
            RfProtocol::NfcDep => RfInterface::NfcDep,
            _ => RfInterface::Frame,
        }
    }
}

impl From<u8> for RfProtocol {
    fn from(value: u8) -> Self {
        match value {
            0x00 => RfProtocol::Undetermined,
            0x01 => RfProtocol::T1t,
            0x02 => RfProtocol::T2t,
            0x03 => RfProtocol::T3t,
            0x04 => RfProtocol::IsoDep,
            0x05 => RfProtocol::NfcDep,
            0x06 => RfProtocol::T5t,
            0x07 => RfProtocol::Ndef,
            other => RfProtocol::Other(other),
        }
    }
}

/// RF technology and mode codes used in `RF_DISCOVER_CMD` and reported in
/// activation notifications (NCI 2.0 Table 129).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryTech {
    /// Poll for NFC-A (ISO/IEC 14443A) targets.
    PollA,
    /// Poll for NFC-B (ISO/IEC 14443B) targets.
    PollB,
    /// Poll for NFC-F (FeliCa) targets.
    PollF,
    /// Poll in Active Communication mode (NFC-DEP).
    PollActive,
    /// Poll for NFC-V (ISO/IEC 15693) targets.
    PollV,
    /// Listen for NFC-A readers (card emulation / P2P target).
    ListenA,
    /// Listen for NFC-B readers.
    ListenB,
    /// Listen for NFC-F readers.
    ListenF,
    /// Listen in Active Communication mode.
    ListenActive,
    /// A proprietary technology and mode code.
    Other(u8),
}

impl DiscoveryTech {
    pub const fn value(self) -> u8 {
        match self {
            DiscoveryTech::PollA => 0x00,
            DiscoveryTech::PollB => 0x01,
            DiscoveryTech::PollF => 0x02,
            DiscoveryTech::PollActive => 0x03,
            DiscoveryTech::PollV => 0x06,
            DiscoveryTech::ListenA => 0x80,
            DiscoveryTech::ListenB => 0x81,
            DiscoveryTech::ListenF => 0x82,
            DiscoveryTech::ListenActive => 0x83,
            DiscoveryTech::Other(value) => value,
        }
    }
}

/// RF technology and mode of an activated interface, as reported by
/// `RF_INTF_ACTIVATED_NTF`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TechAndMode(pub u8);

impl TechAndMode {
    /// `true` when the controller was activated in listen mode (i.e. a
    /// remote reader selected us: card emulation or P2P target).
    pub const fn is_listen(self) -> bool {
        self.0 & 0x80 != 0
    }

    /// `true` when the controller activated a remote target in poll mode
    /// (reader/initiator operation).
    pub const fn is_poll(self) -> bool {
        !self.is_listen()
    }
}

/// RF-technology-specific parameters of an activated target, extracted from
/// `RF_INTF_ACTIVATED_NTF` (NCI 2.0 §7.1).
#[derive(Clone, Debug, PartialEq)]
pub enum TargetInfo {
    /// NFC-A (ISO/IEC 14443A) target activated in poll mode.
    NfcA {
        /// SENS_RES (ATQA), as transmitted by the target.
        sens_res: [u8; 2],
        /// NFCID1 (UID), 4, 7 or 10 bytes.
        nfcid1: heapless::Vec<u8, 10>,
        /// SEL_RES (SAK), when the target sent one.
        sel_res: Option<u8>,
    },
    /// NFC-B (ISO/IEC 14443B) target activated in poll mode.
    NfcB {
        /// SENSB_RES (ATQB), NFCID0 starts at offset 0.
        sensb_res: heapless::Vec<u8, 12>,
    },
    /// NFC-F (FeliCa) target activated in poll mode.
    NfcF {
        /// 1 = 212 kbit/s, 2 = 424 kbit/s.
        bit_rate: u8,
        /// SENSF_RES; IDm at offset 0 (8 bytes), PMm at offset 8.
        sensf_res: heapless::Vec<u8, 18>,
    },
    /// NFC-V (ISO/IEC 15693) target activated in poll mode.
    NfcV {
        afi: u8,
        dsfid: u8,
        /// UID stored MSB first (byte-reversed from over-the-air order).
        uid: [u8; 8],
    },
    /// Technology without a dedicated parser (e.g. listen-mode activation);
    /// carries the raw RF-technology-specific parameter bytes.
    Raw(heapless::Vec<u8, 64>),
}

/// An activated remote endpoint, parsed from `RF_INTF_ACTIVATED_NTF`.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// RF Discovery ID assigned by the controller.
    pub discovery_id: u8,
    /// RF interface in use for data exchange.
    pub interface: RfInterface,
    /// RF protocol of the remote endpoint.
    pub protocol: RfProtocol,
    /// Activation RF technology and mode.
    pub tech_and_mode: TechAndMode,
    /// Maximum data packet payload size accepted by the controller on the
    /// static RF connection.
    pub max_payload_size: u8,
    /// Initial number of data credits granted on the static RF connection.
    pub initial_credits: u8,
    /// RF-technology-specific parameters.
    pub info: TargetInfo,
    /// Activation parameters that follow the technology-specific block
    /// (RATS response for ISO-DEP poll A, ATTRIB response for NFC-B, ...).
    pub activation: heapless::Vec<u8, 64>,
}

fn subslice(payload: &[u8], start: usize, len: usize) -> Result<&[u8]> {
    payload
        .get(start..start + len)
        .ok_or(Error::InvalidResponseLength {
            expected: start + len,
            actual: payload.len(),
        })
}

impl Target {
    /// Parse the payload of an `RF_INTF_ACTIVATED_NTF`.
    pub fn parse(payload: &[u8]) -> Result<Self> {
        if payload.len() < 7 {
            return Err(Error::InvalidResponseLength {
                expected: 7,
                actual: payload.len(),
            });
        }
        let tech_and_mode = TechAndMode(payload[3]);
        let params_len = payload[6] as usize;
        let params = subslice(payload, 7, params_len)?;

        let info = if tech_and_mode.is_poll() {
            match tech_and_mode.0 {
                // NFC-A passive poll
                0x00 => Self::parse_nfca(params)?,
                // NFC-B passive poll
                0x01 => Self::parse_nfcb(params)?,
                // NFC-F passive poll
                0x02 => Self::parse_nfcf(params)?,
                // NFC-V (ISO 15693) passive poll
                0x06 => Self::parse_nfcv(params)?,
                _ => Self::raw_info(params)?,
            }
        } else {
            Self::raw_info(params)?
        };

        // After the technology-specific block: Data Exchange RF technology
        // and mode (1), transmit bit rate (1), receive bit rate (1),
        // activation parameters length (1) + activation parameters.
        let mut activation = heapless::Vec::new();
        let trailer = 7 + params_len;
        if let Some(&activation_len) = payload.get(trailer + 3) {
            if let Ok(bytes) = subslice(payload, trailer + 4, activation_len as usize) {
                activation
                    .extend_from_slice(bytes)
                    .map_err(|_| Error::BufferOverflow)?;
            }
        }

        Ok(Target {
            discovery_id: payload[0],
            interface: RfInterface::from(payload[1]),
            protocol: RfProtocol::from(payload[2]),
            tech_and_mode,
            max_payload_size: payload[4],
            initial_credits: payload[5],
            info,
            activation,
        })
    }

    fn parse_nfca(params: &[u8]) -> Result<TargetInfo> {
        let sens_res = subslice(params, 0, 2)?;
        let nfcid1_len = *params.get(2).ok_or(Error::InvalidResponseLength {
            expected: 3,
            actual: params.len(),
        })? as usize;
        let mut nfcid1 = heapless::Vec::new();
        nfcid1
            .extend_from_slice(subslice(params, 3, nfcid1_len)?)
            .map_err(|_| Error::BufferOverflow)?;
        let sel_res = match params.get(3 + nfcid1_len) {
            Some(&1) => params.get(4 + nfcid1_len).copied(),
            _ => None,
        };
        Ok(TargetInfo::NfcA {
            sens_res: [sens_res[0], sens_res[1]],
            nfcid1,
            sel_res,
        })
    }

    fn parse_nfcb(params: &[u8]) -> Result<TargetInfo> {
        let len = *params.first().ok_or(Error::InvalidResponseLength {
            expected: 1,
            actual: 0,
        })? as usize;
        let mut sensb_res = heapless::Vec::new();
        sensb_res
            .extend_from_slice(subslice(params, 1, len)?)
            .map_err(|_| Error::BufferOverflow)?;
        Ok(TargetInfo::NfcB { sensb_res })
    }

    fn parse_nfcf(params: &[u8]) -> Result<TargetInfo> {
        if params.len() < 2 {
            return Err(Error::InvalidResponseLength {
                expected: 2,
                actual: params.len(),
            });
        }
        let len = params[1] as usize;
        let mut sensf_res = heapless::Vec::new();
        sensf_res
            .extend_from_slice(subslice(params, 2, len)?)
            .map_err(|_| Error::BufferOverflow)?;
        Ok(TargetInfo::NfcF {
            bit_rate: params[0],
            sensf_res,
        })
    }

    fn parse_nfcv(params: &[u8]) -> Result<TargetInfo> {
        let uid_bytes = subslice(params, 2, 8)?;
        let mut uid = [0u8; 8];
        for (i, byte) in uid_bytes.iter().enumerate() {
            // Byte-reverse so `uid[0]` is the MSB (0xE0 for ISO 15693).
            uid[7 - i] = *byte;
        }
        Ok(TargetInfo::NfcV {
            afi: params[0],
            dsfid: params[1],
            uid,
        })
    }

    fn raw_info(params: &[u8]) -> Result<TargetInfo> {
        let mut raw = heapless::Vec::new();
        raw.extend_from_slice(params)
            .map_err(|_| Error::BufferOverflow)?;
        Ok(TargetInfo::Raw(raw))
    }
}

/// A discovered (but not yet activated) endpoint reported by
/// `RF_DISCOVER_NTF` when more than one target is in the field.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredEndpoint {
    pub discovery_id: u8,
    pub protocol: RfProtocol,
    pub tech_and_mode: TechAndMode,
}

/// One `RF_DISCOVER_MAP_CMD` entry: which RF interface the controller
/// activates for a protocol, in poll mode, listen mode or both.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Mapping {
    pub protocol: RfProtocol,
    pub mode: MappingMode,
    pub interface: RfInterface,
}

/// The modes a [`Mapping`] applies to.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MappingMode {
    Poll = 0x01,
    Listen = 0x02,
    PollAndListen = 0x03,
}

/// `RF_DEACTIVATE_CMD` deactivation type (NCI 2.0 Table 63).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DeactivationType {
    /// Back to RFST_IDLE: RF off.
    Idle = 0x00,
    /// Put the target to sleep, to select another one.
    Sleep = 0x01,
    /// Sleep for an NFC-DEP target, via ATR_REQ attribute.
    SleepAf = 0x02,
    /// Back to the discovery loop.
    Discovery = 0x03,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nfca_t2t_target() {
        // RF_INTF_ACTIVATED_NTF payload: NTAG213 activated with the Frame
        // interface: id=1, intf=Frame, proto=T2T, tech=NFC-A poll,
        // max payload=251, credits=1, 12 tech param bytes.
        let payload = [
            0x01, 0x01, 0x02, 0x00, 0xFB, 0x01, 0x0C, // header part
            0x44, 0x00, // SENS_RES
            0x07, 0x04, 0x51, 0x8E, 0x1A, 0xC0, 0x28, 0x80, // NFCID1 len + 7 bytes
            0x01, 0x00, // SEL_RES len + SAK
        ];
        let target = Target::parse(&payload).unwrap();
        assert_eq!(target.discovery_id, 1);
        assert_eq!(target.interface, RfInterface::Frame);
        assert_eq!(target.protocol, RfProtocol::T2t);
        assert!(target.tech_and_mode.is_poll());
        assert_eq!(target.max_payload_size, 0xFB);
        assert_eq!(target.initial_credits, 1);
        match &target.info {
            TargetInfo::NfcA {
                sens_res,
                nfcid1,
                sel_res,
            } => {
                assert_eq!(sens_res, &[0x44, 0x00]);
                assert_eq!(
                    nfcid1.as_slice(),
                    &[0x04, 0x51, 0x8E, 0x1A, 0xC0, 0x28, 0x80]
                );
                assert_eq!(*sel_res, Some(0x00));
            }
            info => panic!("wrong info: {info:?}"),
        }
    }

    #[test]
    fn parse_iso_dep_target_with_rats() {
        // ISO-DEP card: 4-byte NFCID1, SAK 0x20, followed by data-exchange
        // parameters and the RATS response as activation parameters.
        let payload = [
            0x01, 0x02, 0x04, 0x00, 0xFB, 0x01, 0x09, // ISO-DEP on NFC-A
            0x04, 0x03, // SENS_RES
            0x04, 0xDE, 0xAD, 0xBE, 0xEF, // NFCID1
            0x01, 0x20, // SEL_RES
            0x00, 0x00, 0x00, // DE tech+mode, tx rate, rx rate
            0x03, 0x05, 0x78, 0x80, // activation params: RATS len + bytes
        ];
        let target = Target::parse(&payload).unwrap();
        assert_eq!(target.protocol, RfProtocol::IsoDep);
        assert_eq!(target.interface, RfInterface::IsoDep);
        assert_eq!(target.activation.as_slice(), &[0x05, 0x78, 0x80]);
        match &target.info {
            TargetInfo::NfcA { sel_res, .. } => assert_eq!(*sel_res, Some(0x20)),
            info => panic!("wrong info: {info:?}"),
        }
    }

    #[test]
    fn parse_nfcv_target() {
        let payload = [
            0x01, 0x01, 0x06, 0x06, 0xFB, 0x01, 0x0A, // T5T on NFC-V
            0x00, 0x25, // AFI, DSFID
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0xE0, // UID (air order)
        ];
        let target = Target::parse(&payload).unwrap();
        assert_eq!(target.protocol, RfProtocol::T5t);
        match &target.info {
            TargetInfo::NfcV { afi, dsfid, uid } => {
                assert_eq!(*afi, 0x00);
                assert_eq!(*dsfid, 0x25);
                // Byte-reversed: MSB (0xE0) first.
                assert_eq!(uid, &[0xE0, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
            }
            info => panic!("wrong info: {info:?}"),
        }
    }

    #[test]
    fn parse_listen_mode_activation() {
        // Activated as a card (listen NFC-A, ISO-DEP): raw params.
        let payload = [0x01, 0x02, 0x04, 0x80, 0xFB, 0x01, 0x02, 0xAA, 0xBB];
        let target = Target::parse(&payload).unwrap();
        assert!(target.tech_and_mode.is_listen());
        assert_eq!(
            target.info,
            TargetInfo::Raw(heapless::Vec::from_slice(&[0xAA, 0xBB]).unwrap())
        );
    }

    #[test]
    fn parse_too_short() {
        assert!(Target::parse(&[0x01, 0x02]).is_err());
        // params_len larger than the actual payload
        assert!(Target::parse(&[0x01, 0x01, 0x02, 0x00, 0xFB, 0x01, 0x30]).is_err());
    }

    #[test]
    fn proprietary_codes_pass_through() {
        assert_eq!(RfProtocol::from(0x80), RfProtocol::Other(0x80));
        assert_eq!(RfInterface::from(0x80), RfInterface::Other(0x80));
        assert_eq!(
            RfProtocol::Other(0x80).default_interface(),
            RfInterface::Frame
        );
        assert_eq!(RfProtocol::IsoDep.default_interface(), RfInterface::IsoDep);
    }
}
