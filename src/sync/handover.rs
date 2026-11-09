//! Phone handover (design 5, contract 4j): the words both phones and the terminal show, and the request the terminal
//! builds. The rules of the ceremony live in `ceremony.rs` (`handover_effect`); the row effects in the oplog.

use anyhow::{ensure, Context, Result};

use super::crypto::DevicePublic;
use super::oplog::DeviceId;
use super::wire::PhoneHandoverRequestWire;

/// The `HANDOVER_WORDS` words the old phone and the terminal show for a handover to `new_phone` named `name`, from the
/// proto crate's one function (the phone core calls the same one, so the two sides cannot differ). Empty for a malformed
/// `old_phone` id.
pub fn handover_words(new_phone: &DevicePublic, name: &str, old_phone: &DeviceId) -> Vec<String> {
    match old_phone_raw(old_phone) {
        Some(raw) => super::crypto::handover_words(new_phone, name, &raw),
        None => Vec::new(),
    }
}

fn old_phone_raw(id: &DeviceId) -> Option<[u8; 8]> {
    hex::decode(id).ok()?.try_into().ok()
}

/// What a terminal shows the live phone to hand the ring over (kind 12, request stage): the new phone's public keys, the
/// name the user gave it and this terminal's id.
pub fn handover_request(
    new_phone: DevicePublic,
    name: &str,
    terminal: &DeviceId,
) -> Result<PhoneHandoverRequestWire> {
    ensure!(
        !name.is_empty() && name.len() <= 64 && name.chars().all(|c| !c.is_control()),
        "the new phone's name is one plain line of at most 64 bytes"
    );
    new_phone.validate().context("the new phone's keys")?;
    let terminal = old_phone_raw(terminal).context("this terminal's device id is malformed")?;
    Ok(PhoneHandoverRequestWire {
        new_phone,
        name: name.to_string(),
        terminal,
        op: None,
    })
}

#[cfg(test)]
mod tests;
