package io.github.petyok.sshub.platform

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** U13-6: an ordinary Android action (a new fingerprint) must not destroy the phone key. */
class KeySpecTest {
    @Test fun adding_a_fingerprint_does_not_invalidate_the_key() {
        val spec = AndroidKeyVault.keySpec("sshub.test.spec", strong = false, requireAuth = true)
        assertFalse("the key must survive a new fingerprint", spec.isInvalidatedByBiometricEnrollment)
        assertTrue("every use still needs the system lock", spec.isUserAuthenticationRequired)
        assertTrue("per-use authentication (timeout 0)", spec.userAuthenticationValidityDurationSeconds == 0)
    }
}
