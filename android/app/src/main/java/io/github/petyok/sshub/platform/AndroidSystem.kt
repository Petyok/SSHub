package io.github.petyok.sshub.platform

import android.content.Context
import android.content.pm.PackageManager
import android.provider.Settings
import io.github.petyok.sshub.core.SystemPort
import java.io.IOException
import java.security.MessageDigest

class AndroidSystem(private val context: Context) : SystemPort {
    /** SHA-256 of the signing certificate: `apkContentsSigners` (first) when several signers, else the newest of the signing history. */
    override val apkCertSha256: String by lazy {
        @Suppress("DEPRECATION")
        val info = context.packageManager.getPackageInfo(context.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
        val signing = info.signingInfo ?: return@lazy ""
        val sig = if (signing.hasMultipleSigners()) signing.apkContentsSigners.first() else signing.signingCertificateHistory.last()
        MessageDigest.getInstance("SHA-256").digest(sig.toByteArray()).joinToString("") { "%02x".format(it) }
    }

    override val versionName: String by lazy {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: ""
    }

    override fun licenceText(name: String): String {
        if (name.isEmpty() || name.contains('/') || name.contains("..")) return ""
        return try {
            context.assets.open("licenses/$name").bufferedReader().use { it.readText() }
        } catch (_: IOException) {
            ""
        }
    }

    override val reducedMotion: Boolean
        get() = Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f
}
