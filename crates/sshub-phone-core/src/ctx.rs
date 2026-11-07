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
    /// Runs `f` as one transaction on the state: on any `Err` (a render or size failure after an op was appended, a
    /// recorded short code) the state is put back exactly as it was, so a failed approval leaves nothing for the next
    /// reply's courier tail to carry.
    pub fn atomically<T>(
        &mut self,
        f: impl FnOnce(&mut Ctx<'a>) -> crate::error::Result<T>,
    ) -> crate::error::Result<T> {
        let saved = self.state.clone();
        let id_saved = self.id.snapshot();
        let r = f(self);
        if r.is_err() {
            *self.state = saved;
            // a wrap key rotation switches the identity to the next generation: that goes back too
            self.id.rollback(&id_saved);
        }
        r
    }

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
