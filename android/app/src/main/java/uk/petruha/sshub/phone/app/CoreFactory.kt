package uk.petruha.sshub.phone.app

import uk.petruha.sshub.phone.core.PhoneCoreApi

/** Where the app gets its core. Debug builds: `FakePhoneCore` (src/debug). Release: the uniffi adapter (src/release, added with crates/sshub-phone-core). */
interface CoreFactory { fun create(): PhoneCoreApi }
