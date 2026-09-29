# nfc-nci

The host side of the NFC Forum NFC Controller Interface (NCI 2.x) for `no_std` drivers built on
`embedded-hal` 1.0: everything that does not depend on which NFC controller sits at the other end.

## Features

- **Packets**: `Frame` header encoding and parsing, message types, the standard group and opcode
  identifiers, status codes.
- **Core messages**: `CORE_RESET_NTF` (`ResetNotification`) and the NCI 2.0 `CORE_INIT_RSP`
  (`InitInfo`).
- **RF types**: technologies, protocols, interfaces, discovery mappings, deactivation types, and
  parsing of `RF_INTF_ACTIVATED_NTF` into a `Target` (NFC-A/B/F/V parameters, activation bytes).
  Proprietary codes pass through as `Other(u8)`.
- **The link** (`Nci`): packets over I2C, paced by the controller's IRQ line (either polarity),
  with the header and the payload read in separate transactions as the NCI I2C mapping requires.
  Command/response matching that skips unrelated notifications and reports a `CORE_RESET_NTF` in
  the middle of an operation as `Error::ControllerReset`. Standard commands: `CORE_RESET`,
  `CORE_INIT`, `CORE_SET_CONFIG`/`GET_CONFIG`, `RF_DISCOVER_MAP`, `RF_DISCOVER`,
  `RF_DISCOVER_SELECT`, `RF_DEACTIVATE`. Multi-target discovery. Data exchange on the static RF
  connection with segmentation, reassembly and credit-based flow control.
- **Controller quirks as configuration** (`Config`): IRQ polarity, a retry for writes the controller
  NACKs while it wakes up, and a filler byte to skip ahead of a header.
- `no_std`, `heapless` buffers; `alloc` and `std` features.

## Basic usage

```rust,ignore
use nfc_nci::{gid, Config, InitInfo, Nci, ResetNotification, ResetType};

// I2C bus + IRQ input + delay from your HAL.
let mut nci = Nci::new(i2c, irq, delay, Config::new(0x28));
let reset = nci.core_reset(ResetType::ResetConfig)?;
let init = InitInfo::parse(&nci.core_init()?.payload)?;
// A vendor command in the proprietary group:
let rsp = nci.command(gid::PROPRIETARY, 0x02, &[], 500)?;
```

## Hardware notes

- The controller raises its IRQ line while it has a packet for the host. `Nci` polls it once per
  millisecond; nothing is read without it.
- Driving the controller's enable or reset pins is the driver's job, as is anything its datasheet
  asks for outside NCI (the bus is reachable with `Nci::i2c_mut`).

## References

- NFC Forum, NFC Controller Interface (NCI) Technical Specification 2.0 and 2.1: packet format
  (§3.4), core messages (§4), RF management (§7), data flow control (§4.4), the I2C transport
  mapping (§10.2).

## License

This work is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
