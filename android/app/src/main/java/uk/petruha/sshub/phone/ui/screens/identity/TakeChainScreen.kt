package uk.petruha.sshub.phone.ui.screens.identity

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.ChainAdopted
import uk.petruha.sshub.phone.core.ChainCheck
import uk.petruha.sshub.phone.ui.components.BannerKind
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.ChainSquare
import uk.petruha.sshub.phone.ui.components.CheckLine
import uk.petruha.sshub.phone.ui.components.DiffBox
import uk.petruha.sshub.phone.ui.components.DiffEntry
import uk.petruha.sshub.phone.ui.components.Gap
import uk.petruha.sshub.phone.ui.components.Glyph
import uk.petruha.sshub.phone.ui.components.GlyphKind
import uk.petruha.sshub.phone.ui.components.MonoField
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.RedBanner
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SquareState
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.StopBox
import uk.petruha.sshub.phone.ui.components.TextArea
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.theme.SshubText
import uk.petruha.sshub.phone.ui.theme.sshub

/** `Read`: scan or paste lists. `Paste`: the paste box is open. `Taken`: head adopted or "never signed" confirmed. `Refused`: the last list did not verify. */
enum class ChainHeadPhase { Read, Paste, Taken, Refused }

data class ChainHeadState(
    val phase: ChainHeadPhase = ChainHeadPhase.Read,
    /** Lists read so far: 0, 1, or 2 when two lists end at the same newest op. */
    val lists: Int = 0,
    val chain: ChainAdopted? = null,
    /** The terminal id each list read so far names, oldest first (null: the list did not say). Only a claim. */
    val senders: List<String?> = emptyList(),
    val checks: List<ChainCheck> = emptyList(),
    /** Core's literal reason for a refused list. */
    val refusal: String? = null,
    /** A frame that was not an ops list, shown as a line, nothing was changed. */
    val notice: String? = null,
    val pasteText: String = "",
    val typedId: String = "",
    /** This phone's device id as the core prints it, the text the user must type for "never signed". */
    val deviceId: String = "",
    val neverSigned: Boolean = false,
    val takenSeq: Long? = null,
    val busy: Boolean = false,
) {
    val canNeverSigned: Boolean get() = deviceId.isNotEmpty() && typedId.trim().uppercase() == deviceId.uppercase() && !busy
    val canCheckPaste: Boolean get() = pasteText.trim().length >= 16 && !busy
}

class ChainHeadActions(
    val onScan: () -> Unit = {},
    val onOpenPaste: () -> Unit = {},
    val onClosePaste: () -> Unit = {},
    val onPasteText: (String) -> Unit = {},
    val onCheckPaste: () -> Unit = {},
    val onAdopt: () -> Unit = {},
    val onTypedId: (String) -> Unit = {},
    val onNeverSigned: () -> Unit = {},
    val onTryAnother: () -> Unit = {},
    val onHome: () -> Unit = {},
    val onBack: () -> Unit = {},
)

private const val HEAD_MESSAGE = "This phone was restored from its 24 words, so it does not know how far its own chain went and must not sign yet."

/** Take my chain from a terminal (spec 12 `chain-head`): two lists must agree, then the user accepts the head with `decide`. */
@Composable
fun TakeChainScreen(state: ChainHeadState, on: ChainHeadActions) {
    val taken = state.phase == ChainHeadPhase.Taken
    val seq = state.chain?.headSeq
    val trailing = if (state.neverSigned) "no ops" else "seq ${state.takenSeq ?: seq ?: 0}"
    val bannerContent: @Composable () -> Unit = { RedBanner("CHAIN TAKEN", kind = BannerKind.Green, trailing = trailing) }
    val takenBanner: (@Composable () -> Unit)? = if (taken) bannerContent else null
    ScreenFrame(
        title = "Take my chain from a terminal",
        onBack = on.onBack,
        banner = takenBanner,
        bar = { ChainBar(state, on) },
    ) {
        Steps(state)
        Gap(14.dp)
        when (state.phase) {
            ChainHeadPhase.Taken -> TakenBody(state)
            ChainHeadPhase.Refused -> {
                StopBox("This list was refused", live = true) {
                    Text(state.refusal ?: "An op in it does not verify.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
                    Text("Nothing was adopted and the phone still does not sign. Take the list again from a terminal.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
                }
                Checks(state.checks)
            }
            else -> ReadBody(state, on)
        }
    }
}

@Composable
private fun Steps(state: ChainHeadState) {
    val taken = state.phase == ChainHeadPhase.Taken
    val refused = state.phase == ChainHeadPhase.Refused
    val names = listOf("list 1", "list 2", "adopt")
    val c = if (taken) 3 else state.lists
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        names.forEachIndexed { i, name ->
            val square = when {
                taken -> SquareState.Ok
                i < c -> SquareState.Fill
                i == c && refused -> SquareState.No
                i == c -> SquareState.Current
                else -> SquareState.Default
            }
            val word = when { taken || i < c -> "done"; i == c && refused -> "refused"; i == c -> "next"; else -> "waiting" }
            val seqText = if ((taken || i < minOf(c, 2)) && state.chain != null && i < 2) "seq ${state.chain.headSeq}" else ""
            Column(
                Modifier.weight(1f).semantics(mergeDescendants = true) { contentDescription = "Step ${i + 1} of 3, $name, $word" + if (seqText.isNotEmpty()) ", $seqText" else "" },
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                ChainSquare(square, Modifier.clearAndSetSemantics { })
                Text(name, style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurface, textAlign = TextAlign.Center)
                Text(word, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
                if (seqText.isNotEmpty()) Text(seqText, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
            }
        }
    }
}

@Composable
private fun TakenBody(state: ChainHeadState) {
    SshubCard {
        Text(
            if (state.neverSigned) "Confirmed. This key never signed anything." else "Chain taken. Continuing after seq ${state.takenSeq ?: state.chain?.headSeq ?: 0}",
            style = MaterialTheme.typography.titleLarge, color = MaterialTheme.sshub.ok,
        )
        Gap(6.dp)
        Note(
            if (state.neverSigned) "This phone may sign from the first sequence number."
            else "The phone key generation is read from the adopted chain. The red card on Home is gone.",
        )
    }
    Gap(12.dp)
    WarnBox("One copy only") {
        Text(
            "Do not use the companion web page with these words again. A signature from the old copy now looks like a fork of this chain.",
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
        )
    }
}

@Composable
private fun ReadBody(state: ChainHeadState, on: ChainHeadActions) {
    val chain = state.chain
    Text("the red card on Home says", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    Gap(6.dp)
    WarnBox("This phone does not sign yet.") {
        Text(HEAD_MESSAGE, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
    }
    Gap(12.dp)
    if (state.notice != null) {
        Row(Modifier.fillMaxWidth().padding(bottom = 8.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.Top) {
            Glyph(GlyphKind.Info, MaterialTheme.colorScheme.onSurface, Modifier.clearAndSetSemantics { }, size = 20.dp)
            Text(state.notice, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
        }
    }
    if (state.lists == 0) {
        Note("Ask a terminal for this phone's own newest ops. It shows them as a QR code or as text.")
        Gap(8.dp)
        Text("One list is not enough.", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurface)
        Note("A terminal can be behind, and a chain continued from an old list forks it. Take a second list from a different terminal. The same terminal twice does not count. The phone continues only when both end at the same newest op.")
    }
    if (state.senders.isNotEmpty() && state.lists > 0) {
        state.senders.forEach { Note(senderLine(it)) }
        Note("The terminal id is what the list says about itself, not proof of who sent it.")
    }
    if (chain != null && state.lists == 1) {
        SshubCard {
            Text("List read, one more needed.", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.sshub.ok)
            if (chain.summary.isNotEmpty()) Note(chain.summary)
            if (chain.note.isNotEmpty()) Note(chain.note)
        }
        Checks(state.checks)
    }
    if (chain != null && state.lists >= 2) {
        SectionHeading("Two lists agree", trailing = "both end at seq ${chain.headSeq}")
        if (chain.summary.isNotEmpty()) Note(chain.summary)
        if (chain.note.isNotEmpty()) {
            Gap(12.dp)
            WarnBox("Compare with a terminal first.") {
                Text(chain.note, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
            }
        }
        Checks(state.checks)
    }
    if (state.phase == ChainHeadPhase.Paste) {
        SectionHeading("Paste the list", trailing = "text from a terminal")
        TextArea(state.pasteText, on.onPasteText, label = "Pasted ops list", placeholder = "Paste the terminal's ops list here")
    }
    if (state.lists == 0 && state.phase == ChainHeadPhase.Read) {
        SectionHeading("Only if this key never signed anything")
        WarnBox("A brand new key restored from its words.") {
            Text(
                "If this key ever signed, a new op could reuse a seq your devices already hold.",
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
            )
            Text("type ${state.deviceId} to confirm", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            MonoField(state.typedId, on.onTypedId, placeholder = "device id", capitalize = true, label = "Type the device id to confirm")
            SshubButton("This key never signed anything", on.onNeverSigned, Modifier.fillMaxWidth(), enabled = state.canNeverSigned, disabledReason = "type the device id first")
        }
    }
}

private fun senderLine(id: String?): String =
    if (id == null) "This list does not say which terminal made it."
    else "List from terminal " + id.filter { it.isLetterOrDigit() }.chunked(4).joinToString(" ")

@Composable
private fun Checks(checks: List<ChainCheck>) {
    if (checks.isEmpty()) return
    val pass = checks.count { it.ok == true }
    SectionHeading("Checks on the last list", trailing = "$pass of ${checks.size} pass")
    SshubCard {
        checks.forEach { c ->
            if (c.ok == null) {
                Row(Modifier.fillMaxWidth().heightIn(min = 32.dp).padding(vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text("${c.text}: not checked", Modifier.padding(start = 30.dp), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            } else CheckLine(c.text, c.ok)
        }
    }
}

@Composable
private fun ChainBar(state: ChainHeadState, on: ChainHeadActions) {
    when {
        state.phase == ChainHeadPhase.Taken -> SshubButton("Go to Home", on.onHome, Modifier.fillMaxWidth(), kind = ButtonKind.Primary)
        state.phase == ChainHeadPhase.Refused -> BarButtons {
            SshubButton("Home", on.onHome)
            SshubButton("Try another list", on.onTryAnother, kind = ButtonKind.Primary)
        }
        state.phase == ChainHeadPhase.Paste -> BarButtons {
            SshubButton("Back", on.onClosePaste)
            SshubButton("Check the list", on.onCheckPaste, kind = ButtonKind.Primary, enabled = state.canCheckPaste, disabledReason = "paste the list first")
        }
        state.lists >= 2 -> BarButtons {
            SshubButton("Not yet", on.onHome)
            SshubButton("Adopt: continue after seq ${state.chain?.headSeq ?: 0}", on.onAdopt, kind = ButtonKind.Primary, enabled = !state.busy, disabledReason = "adopting")
        }
        else -> BarButtons {
            SshubButton(if (state.lists == 0) "Scan the list" else "Take the next list", on.onScan, kind = ButtonKind.Primary)
            SshubButton("Paste text", on.onOpenPaste)
        }
    }
}
