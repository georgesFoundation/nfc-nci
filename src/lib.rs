#![cfg_attr(not(any(test, feature = "std")), no_std)]

//! # NFC Forum NCI, host side
//!
//! The parts of the NFC Controller Interface (NCI 2.x) that do not depend on
//! which controller sits at the other end, for `no_std` drivers built on
//! `embedded-hal` 1.0:
//!
//! - **Packets** ([`Frame`]): header encoding and parsing, message types,
//!   the standard group and opcode identifiers and status codes.
//! - **RF types**: technologies, protocols, interfaces, discovery mappings,
//!   and parsing of `RF_INTF_ACTIVATED_NTF` into a [`Target`].
//! - **The link** ([`Nci`]): packets over I2C paced by the controller's IRQ
//!   line, command/response matching, the standard core and RF management
//!   commands, multi-target discovery, and data exchange with segmentation,
//!   reassembly and credit-based flow control.
//!
//! What stays in each controller's driver: its enable and reset pins and
//! start-up sequence, its proprietary commands (GID `0xF`), status codes,
//! configuration parameters and RF protocols, and the type-state API
//! applications use.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use nfc_nci::{gid, core_oid, Config, Nci, ResetType};
//!
//! let mut nci = Nci::new(i2c, irq, delay, Config::new(0x28));
//! let reset = nci.core_reset(ResetType::ResetConfig)?;
//! let init = nci.core_init()?;
//! // A vendor command in the proprietary group:
//! let rsp = nci.command(gid::PROPRIETARY, 0x02, &[], 500)?;
//! ```
//!
//! ## References
//!
//! - NFC Forum, NFC Controller Interface (NCI) Technical Specification 2.0
//!   and 2.1: packet format (§3.4), core and RF management messages (§4,
//!   §7), data flow control (§4.4), the I2C transport mapping (§10.2).

#[cfg(feature = "alloc")]
extern crate alloc;

mod core;
mod error;
mod frame;
mod host;
mod rf;

pub use {
    core::{InitInfo, ResetNotification, MAX_INTERFACES, MAX_MANUFACTURER_INFO},
    error::{Error, Result},
    frame::{
        core_oid, gid, nfcee_oid, rf_oid, Frame, MessageType, ResetType, Status, HEADER_SIZE,
        MAX_FRAME_SIZE, MAX_PAYLOAD_SIZE,
    },
    host::{
        Activation, Config, Nci, RfConnection, MAX_DISCOVERED, NTF_TIMEOUT_MS, RESPONSE_TIMEOUT_MS,
    },
    rf::{
        DeactivationType, DiscoveredEndpoint, DiscoveryTech, Mapping, MappingMode, RfInterface,
        RfProtocol, Target, TargetInfo, TechAndMode,
    },
};
