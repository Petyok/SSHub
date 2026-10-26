package uk.petruha.sshub.phone.ui.screens.home

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.LocalPlatform
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Glyph
import uk.petruha.sshub.phone.ui.components.GlyphKind
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.TextArea
import uk.petruha.sshub.phone.ui.components.ViewfinderFrame
import uk.petruha.sshub.phone.ui.theme.SshubText
import uk.petruha.sshub.phone.ui.theme.sshub

const val SCAN_FOREIGN_TEXT = "Ignored a QR code that is not what this step needs."
const val SCAN_BAD_TEXT = "That is not base64 text. Copy the whole text the terminal shows."

/** `scan` of spec 12: the one screen every "scan a ..." step uses. Needs `LocalPlatform` for the camera preview. */
@Composable
fun ScanScreen(state: ScanState, on: ScanActions) {
    val live = state.camera == ScanCamera.Live
    ScreenFrame(
        title = state.title, onBack = on.onBack,
        bar = { ScanBar(state, on) },
    ) {
        when {
            state.done -> {
                Note(state.hint)
                Gap()
                StatusRow(state)
            }
            !live -> {
                CameraBlock(state.camera)
                if (state.paste || state.camera == ScanCamera.None) { Gap(); PastePanel(state, on) }
                else { Gap(); Note("No camera access? The terminal can still show the code as text: use Paste text instead.") }
            }
            state.paste -> { Note(state.hint); Gap(); PastePanel(state, on) }
            else -> {
                Note(state.hint)
                Gap()
                val camera = LocalPlatform.current.camera
                ViewfinderFrame { Box(Modifier.padding(20.dp)) { camera.Preview(Modifier, on.onLuma) } }
                Gap()
                StatusRow(state)
                Gap(8.dp)
                ProgressSquares(state)
            }
        }
    }
}

@Composable
private fun ScanBar(state: ScanState, on: ScanActions) {
    val pasteLabel = when {
        state.paste -> if (state.camera == ScanCamera.Live) "Use the camera" else "Hide paste box"
        state.camera == ScanCamera.Live -> "Paste text instead"
        else -> "Paste text"
    }
    when {
        state.done -> SshubButton("Continue", on.onContinue, Modifier.fillMaxWidth(), kind = ButtonKind.Primary)
        state.camera == ScanCamera.None -> SshubButton("Back", on.onBack, Modifier.fillMaxWidth())
        state.camera == ScanCamera.Rationale -> BarButtons {
            SshubButton("Allow the camera", on.onGrant, kind = ButtonKind.Primary)
            SshubButton(pasteLabel, on.onTogglePaste)
        }
        state.camera == ScanCamera.Denied -> BarButtons {
            SshubButton("Try again", on.onGrant, kind = ButtonKind.Primary, subtitle = "I allowed the camera")
            SshubButton(pasteLabel, on.onTogglePaste)
        }
        state.camera == ScanCamera.DeniedForever -> BarButtons {
            SshubButton("Open settings", on.onOpenSettings)
            SshubButton(pasteLabel, on.onTogglePaste)
        }
        else -> SshubButton(pasteLabel, on.onTogglePaste, Modifier.fillMaxWidth())
    }
}

@Composable
private fun CameraBlock(camera: ScanCamera) {
    val (title, body) = when (camera) {
        ScanCamera.Rationale -> "The camera reads the QR code" to "Nothing is recorded or stored. Android asks for the camera next."
        ScanCamera.Denied -> "Camera permission is off" to "SSHub reads a QR code with the camera. Tap Try again to ask Android again, or paste the text instead."
        ScanCamera.DeniedForever -> "Camera permission is off" to
            "SSHub reads a QR code with the camera. To turn it on, open Android Settings, then Apps, then SSHub, then Permissions. Choose Camera and allow it while the app is in use. Come back here. If Android says the app has to restart, close SSHub from the recent apps and open it again."
        else -> "This phone has no camera" to "The terminal can show the same bytes as text. Copy that text and paste it here."
    }
    SshubCard {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Glyph(GlyphKind.Scan, MaterialTheme.colorScheme.onSurface, Modifier.clearAndSetSemantics { }, size = 32.dp)
            Text(title, Modifier.weight(1f), style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurface)
        }
        Note(body, Modifier.padding(top = 8.dp))
    }
}

private class Status(val glyph: GlyphKind, val text: String, val announce: String)

private fun statusOf(s: ScanState): Status = when {
    s.done -> Status(
        GlyphKind.Check,
        if (s.need > 1) "Read ${s.need} of ${s.need} parts. Checking the document." else "Code read. Checking the document.",
        "Code read",
    )
    s.ignored != null -> Status(GlyphKind.Cross, s.ignored, "Code not recognised. ${s.ignored}")
    s.have > 0 -> Status(GlyphKind.Check, "Reading: ${s.have} of ${s.need} parts", "Reading ${s.have} of ${s.need} parts")
    else -> Status(GlyphKind.Scan, "Point the camera at the terminal QR code.", "Camera ready, point it at the QR code")
}

@Composable
private fun StatusRow(state: ScanState) {
    val st = statusOf(state)
    Row(
        Modifier.fillMaxWidth().padding(vertical = 4.dp).semantics(mergeDescendants = true) {
            liveRegion = LiveRegionMode.Polite
            contentDescription = st.announce
        },
        horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically,
    ) {
        Glyph(st.glyph, MaterialTheme.colorScheme.onSurface, size = 20.dp)
        Text(st.text, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
    }
}

@Composable
private fun ProgressSquares(state: ScanState) {
    val cs = MaterialTheme.colorScheme
    val sc = MaterialTheme.sshub
    if (state.need in 2..16) {
        Row(Modifier.fillMaxWidth().clearAndSetSemantics { }, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            for (i in 0 until state.need) {
                val read = i < state.have
                val current = i == state.have && state.ignored == null
                Box(
                    Modifier.weight(1f).height(20.dp)
                        .background(if (read) cs.onSurface else cs.background, MaterialTheme.shapes.extraSmall)
                        .border(if (current) 2.dp else 1.dp, if (current) sc.focusText else cs.onSurfaceVariant, MaterialTheme.shapes.extraSmall),
                )
            }
        }
    }
    Text(
        if (state.have > 0) "frame ${state.have} of ${state.need}" else "waiting for the first frame",
        Modifier.padding(top = 8.dp), style = SshubText.fieldKey, color = cs.onSurfaceVariant,
    )
}

@Composable
private fun PastePanel(state: ScanState, on: ScanActions) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        TextArea(
            state.pasteText, on.onPasteText, label = "Pasted text",
            placeholder = "base64 of the document (no camera? paste it here)",
        )
        if (state.pasteError != null) {
            Row(
                Modifier.semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
                horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically,
            ) {
                Glyph(GlyphKind.Cross, MaterialTheme.sshub.dangerText, size = 18.dp)
                Text(state.pasteError, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.sshub.dangerText)
            }
        }
        SshubButton("Paste from clipboard", on.onPasteFromClipboard, Modifier.fillMaxWidth())
        SshubButton(
            "Use pasted text", on.onUsePasted, Modifier.fillMaxWidth(),
            enabled = state.pasteText.isNotBlank(), disabledReason = "nothing pasted yet",
        )
    }
}
