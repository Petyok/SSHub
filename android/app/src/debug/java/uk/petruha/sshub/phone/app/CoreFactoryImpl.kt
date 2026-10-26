package uk.petruha.sshub.phone.app

import uk.petruha.sshub.phone.core.PhoneCoreApi
import uk.petruha.sshub.phone.fake.FakePhoneCore

object CoreFactoryImpl : CoreFactory { override fun create(): PhoneCoreApi = FakePhoneCore() }
