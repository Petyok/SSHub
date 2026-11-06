//! What every flow gets: the unlocked identity, the state, the randomness and the clock, all passed in (Rust never
//! reads the platform clock; tests inject a seeded RNG).

use rand_core::RngCore;

use crate::identity::PhoneIdentity;
use crate::state::PhoneState;

pub struct Ctx<'a> {
    pub id: &'a mut PhoneIdentity,
    pub state: &'a mut PhoneState,
    pub rng: &'a mut dyn RngCore,
    /// Unix seconds from the caller.
    pub now: i64,
}

impl<'a> Ctx<'a> {
    /// Names of devices this phone knows: names the user gave plus the names of the terminals it enrolled with
    /// (the ring wins), by device id.
    pub fn aliases(&self) -> std::collections::BTreeMap<String, String> {
        let mut a = self.state.aliases.clone();
        for (id, d) in &self.state.ring {
            a.insert(id.clone(), d.name.clone());
        }
        a
    }
}
