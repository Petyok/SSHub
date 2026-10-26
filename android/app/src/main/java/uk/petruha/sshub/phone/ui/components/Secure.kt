package uk.petruha.sshub.phone.ui.components

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.view.WindowManager
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.platform.LocalContext

private fun Context.activity(): Activity? = when (this) { is Activity -> this; is ContextWrapper -> baseContext.activity(); else -> null }

/**
 * D18: FLAG_SECURE for as long as this composable is in the composition. Put `SecureWindow()` at the top of every backup, release,
 * Red, certificate, words, shares and full-hash screen. Reference counted, so two overlapping secure screens do not clear each other.
 */
@Composable
fun SecureWindow() {
    val a = LocalContext.current.activity()
    DisposableEffect(a) {
        if (a != null) { if (secureCount++ == 0) a.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE) }
        onDispose { if (a != null && --secureCount == 0) a.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE) }
    }
}
private var secureCount = 0
