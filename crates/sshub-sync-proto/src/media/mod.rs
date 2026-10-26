//! The decoders and the link layer of the phone channels, with no device access: `fsk` (audio),
//! `qrscan` (reverse QR, feature `qr-decode`) and `ble_link` (the Bluetooth link layer, sans-IO).

pub mod ble_link;
pub mod fsk;
#[cfg(feature = "qr-decode")]
pub mod qrscan;
