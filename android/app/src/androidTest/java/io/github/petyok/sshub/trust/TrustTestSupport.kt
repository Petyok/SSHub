package io.github.petyok.sshub.trust

import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import org.junit.Assert.assertTrue
import io.github.petyok.sshub.ui.screens.trust.CodeIssuedActions
import io.github.petyok.sshub.ui.screens.trust.ShortCodesActions
import io.github.petyok.sshub.ui.screens.trust.TrustExtendActions
import io.github.petyok.sshub.ui.screens.trust.TrustHandoffActions
import io.github.petyok.sshub.ui.screens.trust.TrustListActions
import io.github.petyok.sshub.ui.screens.trust.TrustRequestActions

/** D18: the window of the activity carries FLAG_SECURE while the screen is composed. */
internal fun assertFlagSecure(rule: AndroidComposeTestRule<*, ComponentActivity>) {
    rule.runOnIdle {
        assertTrue("FLAG_SECURE not set", rule.activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_SECURE != 0)
    }
}

internal fun SemanticsNodeInteraction.description(): String =
    fetchSemanticsNode().config[SemanticsProperties.ContentDescription].joinToString(" ")

internal fun requestActions(
    onTyped: (String) -> Unit = {}, onApprove: () -> Unit = {}, onDecline: () -> Unit = {}, onBack: () -> Unit = {}, onScanId: () -> Unit = {},
) = TrustRequestActions(onTyped, onApprove, onDecline, onBack, onScanId)

internal fun extendActions(
    onApprove: () -> Unit = {}, onDecline: () -> Unit = {}, onCopy: () -> Unit = {}, onDone: () -> Unit = {}, onBack: () -> Unit = {},
) = TrustExtendActions(onApprove, onDecline, onCopy, onDone, onBack)

internal fun handoffActions(onCopy: () -> Unit = {}, onDone: () -> Unit = {}, onBack: () -> Unit = {}) = TrustHandoffActions(onCopy, onDone, onBack)

internal fun listActions(
    onToggle: (String) -> Unit = {}, onAskRevoke: (String) -> Unit = {}, onKeep: () -> Unit = {}, onRevoke: () -> Unit = {}, onScan: () -> Unit = {}, onBack: () -> Unit = {},
) = TrustListActions(onToggle, onAskRevoke, onKeep, onRevoke, onScan, onBack)

internal fun codesActions(
    onAskDead: (String) -> Unit = {}, onKeep: () -> Unit = {}, onDeclareDead: () -> Unit = {}, onBack: () -> Unit = {},
) = ShortCodesActions(onAskDead, onKeep, onDeclareDead, onBack)

internal fun issuedActions(onDone: () -> Unit = {}, onBack: () -> Unit = {}) = CodeIssuedActions(onDone, onBack)
