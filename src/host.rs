//! The host side of an NCI link over I2C: packet I/O paced by the
//! controller's IRQ line, command/response matching, the standard core and
//! RF management commands, and data exchange on the static RF connection.

use {
    crate::{
        error::{Error, Result},
        frame::{
            core_oid, gid, rf_oid, Frame, MessageType, ResetType, HEADER_SIZE, MAX_FRAME_SIZE,
            MAX_PAYLOAD_SIZE,
        },
        rf::{
            DeactivationType, DiscoveredEndpoint, DiscoveryTech, Mapping, RfInterface, RfProtocol,
            Target, TechAndMode,
        },
    },
    embedded_hal::{delay::DelayNs, digital::InputPin, i2c::I2c},
};

/// Default timeout for the response to an NCI command.
pub const RESPONSE_TIMEOUT_MS: u32 = 500;

/// Default timeout for a notification that closely follows a response
/// (`CORE_RESET_NTF`, `RF_DEACTIVATE_NTF`, ...).
pub const NTF_TIMEOUT_MS: u32 = 100;

/// Endpoints kept from a multi-target discovery round, besides the one
/// activated.
pub const MAX_DISCOVERED: usize = 3;

/// Abort threshold for unexpected notifications while waiting for a
/// specific packet.
const MAX_UNEXPECTED_FRAMES: u32 = 32;

/// Filler bytes skipped before giving up on a header.
const MAX_FILLER_BYTES: usize = 8;

/// How the controller behaves on the bus.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Config {
    /// 7-bit I2C address.
    pub address: u8,
    /// IRQ level meaning "a packet is waiting": `true` for active high.
    pub irq_active_high: bool,
    /// When set, a NACKed write is sent once more after this many
    /// milliseconds: some controllers NACK the first access while they wake
    /// up from standby.
    pub write_retry_delay_ms: Option<u32>,
    /// A byte the controller may send ahead of a packet header, and sends in
    /// place of one when it has nothing to say. It must never be a valid
    /// first header byte (a first byte whose message type is RFU is safe).
    pub filler: Option<u8>,
}

impl Config {
    /// Active-high IRQ, no write retry, no filler byte.
    pub const fn new(address: u8) -> Self {
        Config {
            address,
            irq_active_high: true,
            write_retry_delay_ms: None,
            filler: None,
        }
    }
}

/// Flow-control state of the static RF connection (Conn ID 0) while an
/// endpoint is activated.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RfConnection {
    /// Largest data packet payload the controller accepts.
    pub max_payload_size: u8,
    /// Data packets the host may still send before the controller grants
    /// more with `CORE_CONN_CREDITS_NTF`.
    pub credits: u8,
}

impl RfConnection {
    /// The connection an activation notification opened.
    pub fn new(target: &Target) -> Self {
        RfConnection {
            max_payload_size: target.max_payload_size,
            credits: target.initial_credits,
        }
    }

    /// Credit the static RF connection from a `CORE_CONN_CREDITS_NTF`.
    fn add_credits(&mut self, frame: &Frame) {
        // Payload: number of entries, then {Conn ID, Credits} pairs.
        let Some(&entries) = frame.payload.first() else {
            return;
        };
        for entry in 0..entries as usize {
            if let (Some(&conn_id), Some(&credits)) = (
                frame.payload.get(1 + entry * 2),
                frame.payload.get(2 + entry * 2),
            ) {
                if conn_id == 0 {
                    self.credits = self.credits.saturating_add(credits);
                }
            }
        }
    }
}

/// Outcome of [`Nci::wait_for_activation`].
// Boxing the target to balance the variants is not an option without
// `alloc`.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Activation {
    /// An endpoint was activated. `remaining` holds the other endpoints of a
    /// multi-target discovery round, which were not activated.
    Activated {
        target: Target,
        remaining: heapless::Vec<DiscoveredEndpoint, MAX_DISCOVERED>,
    },
    /// Nothing was activated within the timeout; discovery keeps running.
    Timeout,
}

/// The host side of an NCI link to a controller on I2C, with its IRQ line.
///
/// This is the building block for controller drivers: it knows NCI, not the
/// controller. Driving the enable or reset pins, the start-up sequence and
/// proprietary commands are the driver's job.
pub struct Nci<I2C, IRQ, D> {
    i2c: I2C,
    irq: IRQ,
    delay: D,
    config: Config,
}

impl<I2C, IRQ, D> Nci<I2C, IRQ, D>
where
    I2C: I2c,
    IRQ: InputPin,
    D: DelayNs,
{
    pub fn new(i2c: I2C, irq: IRQ, delay: D, config: Config) -> Self {
        Nci {
            i2c,
            irq,
            delay,
            config,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Release the bus, the IRQ pin and the delay.
    pub fn release(self) -> (I2C, IRQ, D) {
        (self.i2c, self.irq, self.delay)
    }

    /// The bus, for controller-specific accesses outside NCI.
    pub fn i2c_mut(&mut self) -> &mut I2C {
        &mut self.i2c
    }

    pub fn delay_mut(&mut self) -> &mut D {
        &mut self.delay
    }

    /// `true` when the IRQ line says a packet is waiting.
    pub fn irq_asserted(&mut self) -> Result<bool> {
        let high = self.irq.is_high().map_err(Error::gpio)?;
        Ok(high == self.config.irq_active_high)
    }

    /// Poll the IRQ line, once per millisecond, until a packet is waiting.
    pub fn wait_irq(&mut self, timeout_ms: u32) -> Result<()> {
        let mut waited = 0;
        loop {
            if self.irq_asserted()? {
                return Ok(());
            }
            if waited >= timeout_ms {
                return Err(Error::Timeout);
            }
            self.delay.delay_ms(1);
            waited += 1;
        }
    }

    /// Write one packet in a single I2C transaction.
    pub fn write_frame(&mut self, frame: &Frame) -> Result<()> {
        let mut buffer = [0u8; MAX_FRAME_SIZE];
        let len = HEADER_SIZE + frame.payload.len();
        buffer[..HEADER_SIZE].copy_from_slice(&frame.header());
        buffer[HEADER_SIZE..len].copy_from_slice(&frame.payload);
        let address = self.config.address;
        match (
            self.i2c.write(address, &buffer[..len]),
            self.config.write_retry_delay_ms,
        ) {
            (Ok(()), _) => Ok(()),
            (Err(_), Some(delay_ms)) => {
                self.delay.delay_ms(delay_ms);
                self.i2c.write(address, &buffer[..len]).map_err(Error::i2c)
            }
            (Err(err), None) => Err(Error::i2c(err)),
        }
    }

    /// Read one packet: the 3-byte header in one I2C transaction, then the
    /// payload in another (NCI 2.x §10.2, I2C transport mapping).
    pub fn read_frame(&mut self) -> Result<Frame> {
        let address = self.config.address;
        let mut header = [0u8; HEADER_SIZE];
        self.i2c.read(address, &mut header).map_err(Error::i2c)?;
        if let Some(filler) = self.config.filler {
            let mut skipped = 0;
            while header[0] == filler {
                skipped += 1;
                if skipped > MAX_FILLER_BYTES {
                    return Err(Error::NoPacket);
                }
                header.rotate_left(1);
                self.i2c
                    .read(address, &mut header[HEADER_SIZE - 1..])
                    .map_err(Error::i2c)?;
            }
        }
        let (mt, pbf, id, oid, len) = Frame::parse_header(header)?;
        let mut payload = heapless::Vec::new();
        if len > 0 {
            payload.resize(len, 0).map_err(|_| Error::BufferOverflow)?;
            self.i2c.read(address, &mut payload).map_err(Error::i2c)?;
        }
        Ok(Frame {
            mt,
            pbf,
            id,
            oid,
            payload,
        })
    }

    /// Wait for the IRQ, then read one packet.
    pub fn receive_frame(&mut self, timeout_ms: u32) -> Result<Frame> {
        self.wait_irq(timeout_ms)?;
        self.read_frame()
    }

    /// Read and discard up to `max_frames` packets left waiting, for example
    /// from before a reset.
    pub fn drain(&mut self, max_frames: usize) -> Result<()> {
        for _ in 0..max_frames {
            if !self.irq_asserted()? {
                break;
            }
            let _ = self.read_frame();
        }
        Ok(())
    }

    /// Send a command and wait for its response, skipping unrelated
    /// notifications. The response's status is not checked.
    ///
    /// A `CORE_RESET_NTF` in the meantime means the controller reset itself,
    /// and ends the wait with [`Error::ControllerReset`].
    pub fn command(&mut self, gid: u8, oid: u8, payload: &[u8], timeout_ms: u32) -> Result<Frame> {
        self.write_frame(&Frame::command(gid, oid, payload)?)?;
        let mut unexpected = 0;
        loop {
            let frame = self.receive_frame(timeout_ms)?;
            if frame.is_response(gid, oid) {
                return Ok(frame);
            }
            check_not_reset(&frame)?;
            if frame.mt != MessageType::Notification {
                return Err(frame.unexpected());
            }
            unexpected += 1;
            if unexpected >= MAX_UNEXPECTED_FRAMES {
                return Err(Error::NotificationFlood);
            }
        }
    }

    /// Wait for a specific notification, skipping others.
    pub fn wait_notification(&mut self, gid: u8, oid: u8, timeout_ms: u32) -> Result<Frame> {
        let mut unexpected = 0;
        loop {
            let frame = self.receive_frame(timeout_ms)?;
            if frame.is_notification(gid, oid) {
                return Ok(frame);
            }
            check_not_reset(&frame)?;
            unexpected += 1;
            if unexpected >= MAX_UNEXPECTED_FRAMES {
                return Err(Error::NotificationFlood);
            }
        }
    }

    /// `CORE_RESET_CMD`, then the `CORE_RESET_NTF` that NCI 2.x sends after
    /// the response, returned when it arrived within [`NTF_TIMEOUT_MS`].
    pub fn core_reset(&mut self, reset_type: ResetType) -> Result<Option<Frame>> {
        let rsp = self.command(
            gid::CORE,
            core_oid::RESET,
            &[reset_type as u8],
            RESPONSE_TIMEOUT_MS,
        )?;
        rsp.ensure_status_ok()?;
        match self.wait_notification(gid::CORE, core_oid::RESET, NTF_TIMEOUT_MS) {
            Ok(frame) => Ok(Some(frame)),
            Err(Error::Timeout) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// `CORE_INIT_CMD`, NCI 2.x form (two feature octets, both 0). Returns
    /// the response, status checked.
    pub fn core_init(&mut self) -> Result<Frame> {
        let rsp = self.command(
            gid::CORE,
            core_oid::INIT,
            &[0x00, 0x00],
            RESPONSE_TIMEOUT_MS,
        )?;
        rsp.ensure_status_ok()?;
        Ok(rsp)
    }

    /// `CORE_SET_CONFIG_CMD` with pre-encoded TLVs: `num_params` entries in
    /// `tlvs`.
    pub fn set_config(&mut self, num_params: u8, tlvs: &[u8]) -> Result<()> {
        let mut payload: heapless::Vec<u8, MAX_PAYLOAD_SIZE> = heapless::Vec::new();
        payload.push(num_params).ok();
        payload
            .extend_from_slice(tlvs)
            .map_err(|_| Error::BufferOverflow)?;
        self.command(
            gid::CORE,
            core_oid::SET_CONFIG,
            &payload,
            RESPONSE_TIMEOUT_MS,
        )?
        .ensure_status_ok()
    }

    /// `CORE_GET_CONFIG_CMD` for `num_params` identifiers concatenated in
    /// `ids`. Returns the raw response payload: status, number of
    /// parameters, TLVs.
    pub fn get_config(
        &mut self,
        num_params: u8,
        ids: &[u8],
    ) -> Result<heapless::Vec<u8, MAX_PAYLOAD_SIZE>> {
        let mut payload: heapless::Vec<u8, MAX_PAYLOAD_SIZE> = heapless::Vec::new();
        payload.push(num_params).ok();
        payload
            .extend_from_slice(ids)
            .map_err(|_| Error::BufferOverflow)?;
        let rsp = self.command(
            gid::CORE,
            core_oid::GET_CONFIG,
            &payload,
            RESPONSE_TIMEOUT_MS,
        )?;
        rsp.ensure_status_ok()?;
        Ok(rsp.payload)
    }

    /// `RF_DISCOVER_MAP_CMD`: which RF interface to activate for each
    /// protocol.
    pub fn discover_map(&mut self, mappings: &[Mapping]) -> Result<()> {
        let mut payload: heapless::Vec<u8, MAX_PAYLOAD_SIZE> = heapless::Vec::new();
        payload.push(mappings.len() as u8).ok();
        for mapping in mappings {
            payload
                .extend_from_slice(&[
                    mapping.protocol.value(),
                    mapping.mode as u8,
                    mapping.interface.value(),
                ])
                .map_err(|_| Error::BufferOverflow)?;
        }
        self.command(gid::RF, rf_oid::DISCOVER_MAP, &payload, RESPONSE_TIMEOUT_MS)?
            .ensure_status_ok()
    }

    /// `RF_DISCOVER_CMD`: start the discovery loop on `techs`, each at
    /// discovery frequency 1 (every loop).
    pub fn discover(&mut self, techs: &[DiscoveryTech]) -> Result<()> {
        let mut payload: heapless::Vec<u8, MAX_PAYLOAD_SIZE> = heapless::Vec::new();
        payload.push(techs.len() as u8).ok();
        for tech in techs {
            payload
                .extend_from_slice(&[tech.value(), 0x01])
                .map_err(|_| Error::BufferOverflow)?;
        }
        self.command(gid::RF, rf_oid::DISCOVER, &payload, RESPONSE_TIMEOUT_MS)?
            .ensure_status_ok()
    }

    /// `RF_DISCOVER_SELECT_CMD`: activate one endpoint of a multi-target
    /// discovery round.
    pub fn discover_select(
        &mut self,
        discovery_id: u8,
        protocol: RfProtocol,
        interface: RfInterface,
    ) -> Result<()> {
        self.command(
            gid::RF,
            rf_oid::DISCOVER_SELECT,
            &[discovery_id, protocol.value(), interface.value()],
            RESPONSE_TIMEOUT_MS,
        )?
        .ensure_status_ok()
    }

    /// `RF_DEACTIVATE_CMD(Idle)` from the discovery loop, which the
    /// controller answers with a response only.
    pub fn stop_discovery(&mut self) -> Result<()> {
        self.command(
            gid::RF,
            rf_oid::DEACTIVATE,
            &[DeactivationType::Idle as u8],
            RESPONSE_TIMEOUT_MS,
        )?
        .ensure_status_ok()
    }

    /// `RF_DEACTIVATE_CMD` from an activated state, then the
    /// `RF_DEACTIVATE_NTF` that follows (tolerating its absence).
    pub fn deactivate(&mut self, kind: DeactivationType) -> Result<()> {
        self.command(
            gid::RF,
            rf_oid::DEACTIVATE,
            &[kind as u8],
            RESPONSE_TIMEOUT_MS,
        )?
        .ensure_status_ok()?;
        match self.wait_notification(gid::RF, rf_oid::DEACTIVATE, NTF_TIMEOUT_MS) {
            Ok(_) | Err(Error::Timeout) => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Wait in the discovery loop until an endpoint is activated.
    ///
    /// When several endpoints are in the field the controller lists them
    /// with `RF_DISCOVER_NTF`; the first one is selected, with the RF
    /// interface `interface_for` picks for its protocol, and the others are
    /// returned in [`Activation::Activated::remaining`].
    pub fn wait_for_activation(
        &mut self,
        timeout_ms: u32,
        interface_for: impl Fn(RfProtocol) -> RfInterface,
    ) -> Result<Activation> {
        let mut remaining: heapless::Vec<DiscoveredEndpoint, MAX_DISCOVERED> = heapless::Vec::new();
        let mut first: Option<DiscoveredEndpoint> = None;
        loop {
            let frame = match self.receive_frame(timeout_ms) {
                Ok(frame) => frame,
                Err(Error::Timeout) => return Ok(Activation::Timeout),
                Err(err) => return Err(err),
            };
            check_not_reset(&frame)?;
            if frame.is_notification(gid::RF, rf_oid::INTF_ACTIVATED) {
                return Ok(Activation::Activated {
                    target: Target::parse(&frame.payload)?,
                    remaining,
                });
            }
            if frame.is_notification(gid::RF, rf_oid::DISCOVER) && frame.payload.len() >= 4 {
                let endpoint = DiscoveredEndpoint {
                    discovery_id: frame.payload[0],
                    protocol: RfProtocol::from(frame.payload[1]),
                    tech_and_mode: TechAndMode(frame.payload[2]),
                };
                match first {
                    None => first = Some(endpoint),
                    Some(_) => {
                        remaining.push(endpoint).ok();
                    }
                }
                // Notification type 0x02: more endpoints follow.
                let more = frame.payload.last() == Some(&0x02);
                if let (false, Some(endpoint)) = (more, first) {
                    self.discover_select(
                        endpoint.discovery_id,
                        endpoint.protocol,
                        interface_for(endpoint.protocol),
                    )?;
                    // The activation arrives on a later iteration.
                }
            }
            // Other notifications (RF_FIELD_INFO, CORE_GENERIC_ERROR, ...)
            // do not matter here.
        }
    }

    /// Send one (possibly segmented) data message on the static RF
    /// connection, honouring its credit-based flow control.
    pub fn send_data(
        &mut self,
        connection: &mut RfConnection,
        data: &[u8],
        timeout_ms: u32,
    ) -> Result<()> {
        let max = connection.max_payload_size.max(1) as usize;
        let mut chunks = data.chunks(max).peekable();
        loop {
            let chunk = chunks.next().unwrap_or(&[]);
            let more = chunks.peek().is_some();
            while connection.credits == 0 {
                let frame = self.receive_frame(timeout_ms)?;
                check_not_reset(&frame)?;
                if frame.is_notification(gid::CORE, core_oid::CONN_CREDITS) {
                    connection.add_credits(&frame);
                } else if frame.is_notification(gid::RF, rf_oid::DEACTIVATE) {
                    return Err(Error::TargetLost);
                }
            }
            self.write_frame(&Frame::data(0, more, chunk)?)?;
            connection.credits = connection.credits.saturating_sub(1);
            if !more {
                return Ok(());
            }
        }
    }

    /// Receive one complete (reassembled) data message from the static RF
    /// connection, feeding the payload segments to `push`.
    pub fn receive_data(
        &mut self,
        connection: &mut RfConnection,
        timeout_ms: u32,
        push: &mut impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        let mut unexpected = 0;
        loop {
            let frame = self.receive_frame(timeout_ms)?;
            check_not_reset(&frame)?;
            if frame.is_data(0) {
                push(&frame.payload)?;
                if !frame.pbf {
                    return Ok(());
                }
            } else if frame.is_notification(gid::CORE, core_oid::CONN_CREDITS) {
                connection.add_credits(&frame);
            } else if frame.is_notification(gid::RF, rf_oid::DEACTIVATE) {
                return Err(Error::TargetLost);
            } else if frame.is_notification(gid::CORE, core_oid::INTERFACE_ERROR) {
                return Err(Error::Nci(
                    frame.payload.first().copied().unwrap_or(0x03).into(),
                ));
            } else {
                unexpected += 1;
                if unexpected >= MAX_UNEXPECTED_FRAMES {
                    return Err(Error::NotificationFlood);
                }
            }
        }
    }
}

/// A `CORE_RESET_NTF` nobody asked for means the controller reset itself.
fn check_not_reset(frame: &Frame) -> Result<()> {
    if frame.is_notification(gid::CORE, core_oid::RESET) {
        return Err(Error::ControllerReset {
            trigger: frame.payload.first().copied().unwrap_or(0x00),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::rf::{MappingMode, TargetInfo},
        embedded_hal::i2c::ErrorKind,
        embedded_hal_mock::eh1::{
            delay::NoopDelay,
            digital::{Mock as PinMock, State as PinState, Transaction as PinTransaction},
            i2c::{Mock as I2cMock, Transaction as I2cTransaction},
        },
    };

    const ADDR: u8 = 0x28;

    fn irq_high() -> PinTransaction {
        PinTransaction::get(PinState::High)
    }

    fn link(i2c: I2cMock, irq: PinMock, config: Config) -> Nci<I2cMock, PinMock, NoopDelay> {
        Nci::new(i2c, irq, NoopDelay, config)
    }

    fn finish(nci: Nci<I2cMock, PinMock, NoopDelay>) {
        let (mut i2c, mut irq, _) = nci.release();
        i2c.done();
        irq.done();
    }

    fn connection(max_payload_size: u8, credits: u8) -> RfConnection {
        RfConnection {
            max_payload_size,
            credits,
        }
    }

    #[test]
    fn command_skips_notifications() {
        let i2c = I2cMock::new(&[
            I2cTransaction::write(ADDR, vec![0x20, 0x01, 0x02, 0x00, 0x00]),
            // RF_FIELD_INFO_NTF in the way
            I2cTransaction::read(ADDR, vec![0x61, 0x07, 0x01]),
            I2cTransaction::read(ADDR, vec![0x01]),
            // CORE_INIT_RSP
            I2cTransaction::read(ADDR, vec![0x40, 0x01, 0x01]),
            I2cTransaction::read(ADDR, vec![0x00]),
        ]);
        let irq = PinMock::new(&[irq_high(), irq_high()]);
        let mut nci = link(i2c, irq, Config::new(ADDR));
        let rsp = nci.core_init().unwrap();
        assert!(rsp.is_response(gid::CORE, core_oid::INIT));
        finish(nci);
    }

    #[test]
    fn unsolicited_core_reset_is_an_error() {
        let i2c = I2cMock::new(&[
            I2cTransaction::write(ADDR, vec![0x20, 0x01, 0x02, 0x00, 0x00]),
            // CORE_RESET_NTF, trigger 0x00: unrecoverable error
            I2cTransaction::read(ADDR, vec![0x60, 0x00, 0x01]),
            I2cTransaction::read(ADDR, vec![0x00]),
        ]);
        let irq = PinMock::new(&[irq_high()]);
        let mut nci = link(i2c, irq, Config::new(ADDR));
        assert_eq!(
            nci.core_init(),
            Err(Error::ControllerReset { trigger: 0x00 })
        );
        finish(nci);
    }

    #[test]
    fn core_reset_returns_the_notification() {
        let i2c = I2cMock::new(&[
            I2cTransaction::write(ADDR, vec![0x20, 0x00, 0x01, 0x01]),
            I2cTransaction::read(ADDR, vec![0x40, 0x00, 0x01]),
            I2cTransaction::read(ADDR, vec![0x00]),
            I2cTransaction::read(ADDR, vec![0x60, 0x00, 0x02]),
            I2cTransaction::read(ADDR, vec![0x02, 0x01]),
        ]);
        let irq = PinMock::new(&[irq_high(), irq_high()]);
        let mut nci = link(i2c, irq, Config::new(ADDR));
        let ntf = nci.core_reset(ResetType::ResetConfig).unwrap().unwrap();
        assert_eq!(ntf.payload.as_slice(), &[0x02, 0x01]);
        finish(nci);
    }

    #[test]
    fn filler_bytes_before_a_header_are_skipped() {
        let i2c = I2cMock::new(&[
            // Two filler bytes, then CORE_INIT_RSP's header, byte by byte.
            I2cTransaction::read(ADDR, vec![0x7E, 0x7E, 0x40]),
            I2cTransaction::read(ADDR, vec![0x01]),
            I2cTransaction::read(ADDR, vec![0x01]),
            I2cTransaction::read(ADDR, vec![0x00]),
        ]);
        let irq = PinMock::new(&[]);
        let config = Config {
            filler: Some(0x7E),
            ..Config::new(ADDR)
        };
        let mut nci = link(i2c, irq, config);
        let frame = nci.read_frame().unwrap();
        assert!(frame.is_response(gid::CORE, core_oid::INIT));
        assert_eq!(frame.payload.as_slice(), &[0x00]);
        finish(nci);
    }

    #[test]
    fn filler_only_means_no_packet() {
        let mut transactions = vec![I2cTransaction::read(ADDR, vec![0x7E, 0x7E, 0x7E])];
        transactions.extend((0..MAX_FILLER_BYTES).map(|_| I2cTransaction::read(ADDR, vec![0x7E])));
        let i2c = I2cMock::new(&transactions);
        let config = Config {
            filler: Some(0x7E),
            ..Config::new(ADDR)
        };
        let mut nci = link(i2c, PinMock::new(&[]), config);
        assert_eq!(nci.read_frame(), Err(Error::NoPacket));
        finish(nci);
    }

    #[test]
    fn nacked_write_is_retried_when_configured() {
        let i2c = I2cMock::new(&[
            I2cTransaction::write(ADDR, vec![0x21, 0x06, 0x01, 0x00]).with_error(ErrorKind::Other),
            I2cTransaction::write(ADDR, vec![0x21, 0x06, 0x01, 0x00]),
        ]);
        let config = Config {
            write_retry_delay_ms: Some(10),
            ..Config::new(ADDR)
        };
        let mut nci = link(i2c, PinMock::new(&[]), config);
        let frame = Frame::command(gid::RF, rf_oid::DEACTIVATE, &[0x00]).unwrap();
        nci.write_frame(&frame).unwrap();
        finish(nci);
    }

    #[test]
    fn nacked_write_fails_without_retry() {
        let i2c =
            I2cMock::new(&[I2cTransaction::write(ADDR, vec![0x21, 0x06, 0x01, 0x00])
                .with_error(ErrorKind::Other)]);
        let mut nci = link(i2c, PinMock::new(&[]), Config::new(ADDR));
        let frame = Frame::command(gid::RF, rf_oid::DEACTIVATE, &[0x00]).unwrap();
        assert_eq!(nci.write_frame(&frame), Err(Error::I2c(ErrorKind::Other)));
        finish(nci);
    }

    #[test]
    fn active_low_irq() {
        let irq = PinMock::new(&[
            PinTransaction::get(PinState::High),
            PinTransaction::get(PinState::Low),
        ]);
        let config = Config {
            irq_active_high: false,
            ..Config::new(ADDR)
        };
        let mut nci = link(I2cMock::new(&[]), irq, config);
        nci.wait_irq(5).unwrap();
        finish(nci);
    }

    #[test]
    fn irq_timeout() {
        let irq = PinMock::new(&[
            PinTransaction::get(PinState::Low),
            PinTransaction::get(PinState::Low),
            PinTransaction::get(PinState::Low),
        ]);
        let mut nci = link(I2cMock::new(&[]), irq, Config::new(ADDR));
        assert_eq!(nci.wait_irq(2), Err(Error::Timeout));
        finish(nci);
    }

    #[test]
    fn discover_map_encoding() {
        let i2c = I2cMock::new(&[
            I2cTransaction::write(
                ADDR,
                vec![0x21, 0x00, 0x07, 0x02, 0x02, 0x01, 0x01, 0x04, 0x03, 0x02],
            ),
            I2cTransaction::read(ADDR, vec![0x41, 0x00, 0x01]),
            I2cTransaction::read(ADDR, vec![0x00]),
        ]);
        let mut nci = link(i2c, PinMock::new(&[irq_high()]), Config::new(ADDR));
        nci.discover_map(&[
            Mapping {
                protocol: RfProtocol::T2t,
                mode: MappingMode::Poll,
                interface: RfInterface::Frame,
            },
            Mapping {
                protocol: RfProtocol::IsoDep,
                mode: MappingMode::PollAndListen,
                interface: RfInterface::IsoDep,
            },
        ])
        .unwrap();
        finish(nci);
    }

    #[test]
    fn segmented_send_waits_for_credits() {
        let i2c = I2cMock::new(&[
            // First segment, PBF set, uses the only credit.
            I2cTransaction::write(ADDR, vec![0x10, 0x00, 0x02, 0x01, 0x02]),
            // CORE_CONN_CREDITS_NTF: one credit for Conn ID 0
            I2cTransaction::read(ADDR, vec![0x60, 0x06, 0x03]),
            I2cTransaction::read(ADDR, vec![0x01, 0x00, 0x01]),
            // Last segment.
            I2cTransaction::write(ADDR, vec![0x00, 0x00, 0x01, 0x03]),
        ]);
        let mut nci = link(i2c, PinMock::new(&[irq_high()]), Config::new(ADDR));
        let mut conn = connection(2, 1);
        nci.send_data(&mut conn, &[0x01, 0x02, 0x03], 100).unwrap();
        assert_eq!(conn.credits, 0);
        finish(nci);
    }

    #[test]
    fn chained_receive_is_reassembled() {
        let i2c = I2cMock::new(&[
            I2cTransaction::read(ADDR, vec![0x10, 0x00, 0x02]),
            I2cTransaction::read(ADDR, vec![0x90, 0x00]),
            I2cTransaction::read(ADDR, vec![0x60, 0x06, 0x03]),
            I2cTransaction::read(ADDR, vec![0x01, 0x00, 0x01]),
            I2cTransaction::read(ADDR, vec![0x00, 0x00, 0x01]),
            I2cTransaction::read(ADDR, vec![0xAB]),
        ]);
        let irq = PinMock::new(&[irq_high(), irq_high(), irq_high()]);
        let mut nci = link(i2c, irq, Config::new(ADDR));
        let mut conn = connection(0xFB, 0);
        let mut message = Vec::new();
        nci.receive_data(&mut conn, 100, &mut |chunk| {
            message.extend_from_slice(chunk);
            Ok(())
        })
        .unwrap();
        assert_eq!(message, [0x90, 0x00, 0xAB]);
        assert_eq!(conn.credits, 1);
        finish(nci);
    }

    #[test]
    fn deactivation_during_receive_means_target_lost() {
        let i2c = I2cMock::new(&[
            I2cTransaction::read(ADDR, vec![0x61, 0x06, 0x02]),
            I2cTransaction::read(ADDR, vec![0x03, 0x02]),
        ]);
        let mut nci = link(i2c, PinMock::new(&[irq_high()]), Config::new(ADDR));
        let mut conn = connection(0xFB, 1);
        assert_eq!(
            nci.receive_data(&mut conn, 100, &mut |_| Ok(())),
            Err(Error::TargetLost)
        );
        finish(nci);
    }

    #[test]
    fn multi_target_discovery_selects_the_first() {
        let i2c = I2cMock::new(&[
            // RF_DISCOVER_NTF: ID 1, T2T, NFC-A poll, no params, more follow
            I2cTransaction::read(ADDR, vec![0x61, 0x03, 0x05]),
            I2cTransaction::read(ADDR, vec![0x01, 0x02, 0x00, 0x00, 0x02]),
            // RF_DISCOVER_NTF: ID 2, ISO-DEP, NFC-A poll, last one
            I2cTransaction::read(ADDR, vec![0x61, 0x03, 0x05]),
            I2cTransaction::read(ADDR, vec![0x02, 0x04, 0x00, 0x00, 0x00]),
            // RF_DISCOVER_SELECT_CMD for ID 1, T2T over the Frame interface
            I2cTransaction::write(ADDR, vec![0x21, 0x04, 0x03, 0x01, 0x02, 0x01]),
            I2cTransaction::read(ADDR, vec![0x41, 0x04, 0x01]),
            I2cTransaction::read(ADDR, vec![0x00]),
            // RF_INTF_ACTIVATED_NTF
            I2cTransaction::read(ADDR, vec![0x61, 0x05, 0x0A]),
            I2cTransaction::read(
                ADDR,
                vec![0x01, 0x01, 0x02, 0x00, 0xFB, 0x01, 0x03, 0x44, 0x00, 0x00],
            ),
        ]);
        let irq = PinMock::new(&[irq_high(), irq_high(), irq_high(), irq_high()]);
        let mut nci = link(i2c, irq, Config::new(ADDR));
        match nci
            .wait_for_activation(100, RfProtocol::default_interface)
            .unwrap()
        {
            Activation::Activated { target, remaining } => {
                assert_eq!(target.protocol, RfProtocol::T2t);
                assert!(matches!(target.info, TargetInfo::NfcA { .. }));
                assert_eq!(remaining.len(), 1);
                assert_eq!(remaining[0].discovery_id, 2);
                assert_eq!(remaining[0].protocol, RfProtocol::IsoDep);
            }
            Activation::Timeout => panic!("expected an activation"),
        }
        finish(nci);
    }

    #[test]
    fn discovery_timeout() {
        let irq = PinMock::new(&[
            PinTransaction::get(PinState::Low),
            PinTransaction::get(PinState::Low),
        ]);
        let mut nci = link(I2cMock::new(&[]), irq, Config::new(ADDR));
        assert_eq!(
            nci.wait_for_activation(1, RfProtocol::default_interface),
            Ok(Activation::Timeout)
        );
        finish(nci);
    }
}
