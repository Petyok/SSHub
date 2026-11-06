package uk.petruha.sshub.phone.platform

import android.content.pm.PackageManager
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.security.MessageDigest
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate

class SystemAndManifestTest {
    private val inst = InstrumentationRegistry.getInstrumentation()
    private val target = inst.targetContext

    /** The test APK is signed by the same committed debug key, so its signer is an independent source for the expected value. */
    private fun expectedFingerprint(): String {
        @Suppress("DEPRECATION")
        val info = inst.context.packageManager.getPackageInfo(inst.context.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
        val sig = info.signingInfo!!.signingCertificateHistory.last()
        val cert = CertificateFactory.getInstance("X.509").generateCertificate(sig.toByteArray().inputStream()) as X509Certificate
        return MessageDigest.getInstance("SHA-256").digest(cert.encoded).joinToString("") { "%02x".format(it) }
    }

    @Test fun fingerprint_equals_debug_signer() {
        val fp = AndroidSystem(target).apkCertSha256
        assertEquals(64, fp.length)
        assertTrue(fp.all { it in '0'..'9' || it in 'a'..'f' })
        assertEquals(expectedFingerprint(), fp)
    }

    @Test fun version_and_licence() {
        val sys = AndroidSystem(target)
        assertTrue(sys.versionName.isNotEmpty())
        assertTrue(sys.licenceText("Inter-LICENSE.txt").isNotEmpty())
        assertEquals("", sys.licenceText("../AndroidManifest.xml"))
        assertEquals("", sys.licenceText("missing.txt"))
    }

    @Test fun manifest_requests_only_allowed_permissions() {
        @Suppress("DEPRECATION")
        val perms = target.packageManager.getPackageInfo(target.packageName, PackageManager.GET_PERMISSIONS).requestedPermissions.orEmpty().toSet()
        assertFalse(perms.contains("android.permission.INTERNET"))
        assertFalse(perms.contains("android.permission.ACCESS_NETWORK_STATE"))
        assertFalse(perms.contains("android.permission.RECORD_AUDIO"))
        val allowed = setOf(
            "android.permission.CAMERA",
            "android.permission.BLUETOOTH_SCAN",
            "android.permission.BLUETOOTH_CONNECT",
            "android.permission.USE_BIOMETRIC",
            target.packageName + ".DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION",
        )
        assertTrue("unexpected: " + (perms - allowed), allowed.containsAll(perms))
    }

    @Test fun backup_is_off() {
        assertEquals(0, target.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_ALLOW_BACKUP)
    }
}
