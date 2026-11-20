package io.github.petyok.sshub.ui.screens.trust

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.petyok.sshub.ui.components.ButtonKind
import io.github.petyok.sshub.ui.components.SshubButton
import io.github.petyok.sshub.ui.theme.sshub

/**
 * Spec 7.13 sheet: only for the small reversible choice of revoking a certificate or declaring a code dead. It never shows a hash,
 * words, a share or a certificate body. A swipe down or a tap on the scrim is the same as the cancel button. Test tags: `sheet-confirm`, `sheet-cancel`.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TrustConfirmSheet(
    title: String,
    confirmLabel: String,
    cancelLabel: String,
    onConfirm: () -> Unit,
    onCancel: () -> Unit,
    enabled: Boolean = true,
    body: @Composable ColumnScope.() -> Unit,
) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = onCancel,
        sheetState = state,
        containerColor = MaterialTheme.colorScheme.surfaceContainer,
        scrimColor = MaterialTheme.colorScheme.scrim.copy(alpha = 0.6f),
    ) {
        Column(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(title, Modifier.semantics { heading() }, style = MaterialTheme.typography.titleLarge, color = MaterialTheme.sshub.dangerText)
            body()
            SshubButton(confirmLabel, onConfirm, Modifier.fillMaxWidth().testTag("sheet-confirm"), kind = ButtonKind.Danger, enabled = enabled)
            SshubButton(cancelLabel, onCancel, Modifier.fillMaxWidth().testTag("sheet-cancel"))
        }
    }
}
