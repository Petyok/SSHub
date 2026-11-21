package io.github.petyok.sshub.ui.screens.identity

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import io.github.petyok.sshub.core.WordState
import io.github.petyok.sshub.ui.components.ButtonKind
import io.github.petyok.sshub.ui.components.BarButtons
import io.github.petyok.sshub.ui.components.CheckLine
import io.github.petyok.sshub.ui.components.CheckboxRow
import io.github.petyok.sshub.ui.components.ChoiceRow
import io.github.petyok.sshub.ui.components.Gap
import io.github.petyok.sshub.ui.components.Glyph
import io.github.petyok.sshub.ui.components.GlyphKind
import io.github.petyok.sshub.ui.components.Note
import io.github.petyok.sshub.ui.components.ScreenFrame
import io.github.petyok.sshub.ui.components.SecureWindow
import io.github.petyok.sshub.ui.components.SectionHeading
import io.github.petyok.sshub.ui.components.SshubButton
import io.github.petyok.sshub.ui.components.TextArea
import io.github.petyok.sshub.ui.components.WaitingDots
import io.github.petyok.sshub.ui.components.WarnBox
import io.github.petyok.sshub.ui.theme.SshubShape
import io.github.petyok.sshub.ui.theme.SshubText
import io.github.petyok.sshub.ui.theme.sshub

enum class RestorePhase { Editing, Restoring, NoSecureLock, Failed }

data class RestoreState(
    val words: List<String> = List(WORDS) { "" },
    val wordStates: List<WordState> = List(WORDS) { WordState.Empty },
    val paste: String = "",
    /** Restoring over a stored key: the old enrollments and short codes are erased. */
    val replaces: Boolean = false,
    /** Answer to "Did you use the companion page in a browser on this phone before?"; null until the user answers (M4). */
    val usedPage: Boolean? = null,
    /** Only asked when `usedPage` is true: "I cleared the site data of the companion page" (the app may not sign while the page still can, C21). */
    val siteDataCleared: Boolean = false,
    val phase: RestorePhase = RestorePhase.Editing,
    val error: String? = null,
) {
    val accepted: Int get() = wordStates.count { it == WordState.Ok }
    val bad: Int get() = wordStates.count { it == WordState.Bad }
    val canRestore: Boolean get() = accepted == WORDS && usedPage != null && (usedPage == false || siteDataCleared) && phase != RestorePhase.Restoring

    companion object { const val WORDS = 24 }
}

class RestoreActions(
    val onWord: (index: Int, text: String) -> Unit = { _, _ -> },
    val onPaste: (String) -> Unit = {},
    val onFill: () -> Unit = {},
    val onUsedPage: (Boolean) -> Unit = {},
    val onSiteDataCleared: (Boolean) -> Unit = {},
    val onRestore: () -> Unit = {},
    val onBack: () -> Unit = {},
    val onOpenSecuritySettings: () -> Unit = {},
)

/** Restore from the 24 words (spec 12 `firstrun-restore`). The words are typed, so the window is secure (D18). */
@Composable
fun RestoreScreen(state: RestoreState, on: RestoreActions) {
    SecureWindow()
    val restoring = state.phase == RestorePhase.Restoring
    val reason = when {
        state.accepted < RestoreState.WORDS -> "${state.accepted} of ${RestoreState.WORDS} words accepted"
        state.usedPage == null -> "answer whether you used the companion page"
        !state.siteDataCleared && state.usedPage == true -> "tick that you cleared the site data of the companion page"
        else -> "the phone is being restored"
    }
    ScreenFrame(
        title = if (state.replaces) "Replace the key" else "Restore from 24 words",
        onBack = on.onBack,
        bar = {
            BarButtons {
                SshubButton("Back", on.onBack)
                SshubButton(
                    if (restoring) "Restoring..." else if (state.replaces) "Replace and restore" else "Restore this phone", on.onRestore,
                    kind = ButtonKind.Primary, enabled = state.canRestore, disabledReason = reason,
                )
            }
        },
    ) {
        if (state.replaces) {
            WarnBox("Replaces the key stored on this phone") {
                Text("Enrollments and the short-code list of the old key are erased.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
            }
            Gap(8.dp)
        }
        SectionHeading("Recovery words", first = !state.replaces, trailing = "in order, one per box")
        WordEntryGrid(state, on)
        Gap(8.dp)
        if (state.accepted == RestoreState.WORDS) {
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                Glyph(GlyphKind.Check, MaterialTheme.sshub.ok, Modifier.clearAndSetSemantics { }, size = 20.dp)
                Text("${state.accepted} of ${RestoreState.WORDS} words accepted", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.sshub.ok)
            }
        } else {
            Text("${state.accepted} of ${RestoreState.WORDS} words accepted", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
        }
        if (state.bad > 0) CheckLine("${state.bad} not in the word list: fix the marked ones.", ok = false)
        SectionHeading("Or paste all 24")
        TextArea(state.paste, on.onPaste, label = "Pasted words", placeholder = "24 backup words, separated by spaces", secret = true)
        Gap(8.dp)
        SshubButton("Fill the grid", on.onFill, enabled = state.paste.isNotBlank(), disabledReason = "paste the words first")
        SectionHeading("The companion page")
        Text("Did you use the companion page in a browser on this phone before?", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
        ChoiceRow("Yes, I used it", state.usedPage == true, { on.onUsedPage(true) }, Modifier.padding(top = 8.dp))
        ChoiceRow("No, never", state.usedPage == false, { on.onUsedPage(false) }, Modifier.padding(top = 6.dp))
        if (state.usedPage == true) {
            Note("Open the companion page menu, choose Show backup words, then clear the site data of that page in the browser. Two copies that sign with one key fork the chain.", Modifier.padding(top = 8.dp))
            CheckboxRow("I cleared the site data of the companion page", state.siteDataCleared, on.onSiteDataCleared)
        }
        Gap(12.dp)
        WarnBox("A restored phone signs nothing yet.") {
            Text(
                "It has lost the record of its own chain. Home shows a red card until the phone has taken its chain from a terminal. " +
                    "Enrollments, aliases and short codes of an old copy are not carried over: terminals you met before are met again.",
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface,
            )
        }
        when (state.phase) {
            RestorePhase.NoSecureLock -> {
                Gap(12.dp)
                WarnBox("No screen lock on this phone", live = true) {
                    Text("Set a screen lock in Android settings, then restore. The app never stores the key unprotected.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
                    SshubButton("Open security settings", on.onOpenSecuritySettings, Modifier.fillMaxWidth())
                }
            }
            RestorePhase.Failed -> {
                Gap(12.dp)
                WarnBox("The phone was not restored", live = true) {
                    Text(state.error ?: "Try again.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
                }
            }
            RestorePhase.Restoring -> { Gap(12.dp); WaitingDots(word = "restoring") }
            RestorePhase.Editing -> Unit
        }
    }
}

@Composable
private fun WordEntryGrid(state: RestoreState, on: RestoreActions) {
    val scale = textScale()
    val cols = if (scale >= 2.0f) 1 else if (scale >= 1.3f) 2 else 3
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        state.words.indices.toList().chunked(cols).forEach { row ->
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                row.forEach { i -> WordCell(i, state.words[i], state.wordStates[i], { on.onWord(i, it) }, Modifier.weight(1f)) }
                repeat(cols - row.size) { Box(Modifier.weight(1f)) }
            }
        }
    }
}

/** One box of the grid: index, the typed word, a check or a cross. A space or a pasted list moves on to the next box. */
@Composable
private fun WordCell(index: Int, word: String, wordState: WordState, onChange: (String) -> Unit, modifier: Modifier = Modifier) {
    val cs = MaterialTheme.colorScheme
    val sc = MaterialTheme.sshub
    val focus = LocalFocusManager.current
    val shape = MaterialTheme.shapes.small
    val ok = wordState == WordState.Ok
    val bad = wordState == WordState.Bad
    val fill = when { bad -> cs.errorContainer; ok -> cs.secondaryContainer; else -> cs.background }
    val edge = when { bad -> sc.danger; ok -> cs.onSurface; else -> sc.boundary }
    Row(
        modifier.heightIn(min = 48.dp).background(fill, shape).border(if (bad) 2.dp else 1.dp, edge, shape).padding(start = 6.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text("${index + 1}", Modifier.widthIn(min = 18.dp).clearAndSetSemantics { }, style = MaterialTheme.typography.labelMedium, color = cs.onSurfaceVariant)
        BasicTextField(
            word,
            { v ->
                onChange(v)
                if (v.any { it.isWhitespace() }) focus.moveFocus(FocusDirection.Next)
            },
            Modifier.weight(1f).semantics {
                contentDescription = "Word ${index + 1}"
                stateDescription = when (wordState) {
                    WordState.Ok -> "accepted"
                    WordState.Bad -> "not in the word list"
                    WordState.Partial -> "not a whole word yet"
                    WordState.Empty -> "empty"
                }
            }.heightIn(min = 48.dp),
            textStyle = SshubText.monoBody.merge(TextStyle(color = cs.onSurface, fontWeight = FontWeight.Bold)),
            cursorBrush = SolidColor(cs.onSurface),
            keyboardOptions = KeyboardOptions(
                // The password variation keeps the keyboard from learning, suggesting or syncing the phrase (platform M12).
                capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, keyboardType = KeyboardType.Password, imeAction = ImeAction.Next,
            ),
            // the padding lives inside the field so the touch target (the semantics node) keeps the whole 48dp
            decorationBox = { inner -> Box(Modifier.heightIn(min = 48.dp).padding(vertical = 12.dp), contentAlignment = Alignment.CenterStart) { inner() } },
        )
        if (ok) Glyph(GlyphKind.Check, sc.ok, Modifier.clearAndSetSemantics { }, size = 16.dp)
        if (bad) Glyph(GlyphKind.Cross, sc.dangerText, Modifier.clearAndSetSemantics { }, size = 16.dp)
    }
}
