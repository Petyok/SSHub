# PoC E: a hardware key as a share holder

Verdict: **NOT RUN.**

The maintainer has no hardware security key, so the one property this PoC exists to prove could not be
measured: whether user presence is enforced per operation (each credential unwrap needs its own touch) or
cached per session (one touch lets a compromised machine drain every credential while the key stays inserted).
Reasoning cannot settle it, it depends on the key, its firmware and the browser or OS stack in between, and a
simulated key would only prove that the code agrees with itself.

## Consequence

An unproven factor does not enter Red operations. This version therefore ships:

- no hardware-key holder (the share unwrapping secret living in a key),
- no hardware-key third factor,
- no `ThirdFactor::HardwareKey`, no `UnlockPath::HardwareKey`, no holder kind for keys.

Paper is the default third factor and the emailed code the option. The recommended topology no longer
leans on a key: two or more machines plus the phone, with the paper phrase.

## To reopen

Run the rig described in `.claude/host-sync/poc-e-hwkey-holder.md` with at least one physical key and record,
per operation, whether a second unwrap without a touch succeeds. If presence is enforced per operation the
holder kind and the third factor can be added as a self-contained change (the contract enums above and the two
rows in the design tables); if it is cached per session they stay out.
