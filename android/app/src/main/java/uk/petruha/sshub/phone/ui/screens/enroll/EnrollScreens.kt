package uk.petruha.sshub.phone.ui.screens.enroll

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.core.BleDevice
import uk.petruha.sshub.phone.core.EnrollChannel
import uk.petruha.sshub.phone.ui.components.BarButtons
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.ChainRow
import uk.petruha.sshub.phone.ui.components.BigWords
import uk.petruha.sshub.phone.ui.components.ChoiceRow
import uk.petruha.sshub.phone.ui.components.Glyph
import uk.petruha.sshub.phone.ui.components.GlyphKind
import uk.petruha.sshub.phone.ui.components.HexGroups
import uk.petruha.sshub.phone.ui.components.KeyValue
import uk.petruha.sshub.phone.ui.components.MonoField
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SecureWindow
import uk.petruha.sshub.phone.ui.components.SectionHeading
import uk.petruha.sshub.phone.ui.components.SquareState
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.StopBox
import uk.petruha.sshub.phone.ui.components.TextArea
import uk.petruha.sshub.phone.ui.components.ViewfinderFrame
import uk.petruha.sshub.phone.ui.components.WarnBox
import uk.petruha.sshub.phone.ui.components.WaitingDots
import uk.petruha.sshub.phone.ui.theme.SshubText
import uk.petruha.sshub.phone.ui.theme.sshub

// ---- 1 Scan (enroll-1) -----------------------------------------------------------------------------------------------

private val Channels = listOf(
    Triple(EnrollChannel.Qr, GlyphKind.Scan, "Scan the terminal, show yours"),
    Triple(EnrollChannel.Bluetooth, GlyphKind.Bluetooth, "Tag shown on the terminal"),
    Triple(EnrollChannel.Paste, GlyphKind.Paste, "Terminal without a camera"),
    Triple(EnrollChannel.Sound, GlyphKind.Sound, "This phone plays, the terminal listens"),
)

private fun nextLines(c: EnrollChannel): List<String> = when (c) {
    EnrollChannel.Qr -> listOf("Step 2 shows this phone's code: hold it to the terminal camera.", "Steps 3 and 5 scan the terminal's codes.")
    EnrollChannel.Bluetooth -> listOf(
        "The terminal shows a 4-character tag. This phone finds that terminal in a Bluetooth list.",
        "The tag only points to the terminal. The four-word check still decides.",
    )
    EnrollChannel.Paste -> listOf("Each step gives you text to copy to the terminal, or a box for the terminal's text.", "Use it when the terminal has no camera.")
    EnrollChannel.Sound -> listOf(
        "This phone plays its replies as sound, about 8 s (fast) or 16 s (slow) each.",
        "The phone does not listen: it reads the terminal's codes with the camera. Audio is opt-in on the terminal.",
    )
}

@Composable
fun EnrollScanScreen(state: EnrollScanState, on: EnrollScanActions) {
    // The core checks the name rules (length, control characters) when the terminal's code is read; the screen claims nothing before that.
    val blank = state.name.isBlank()
    val ok = state.nameProblem == null && !blank
    val why = state.nameProblem ?: if (blank) "Give this phone a name." else null
    EnrollFrame(
        state.terminal, 1, on.onBack,
        bar = {
            if (state.scanning) SshubButton("Cancel", on.onCancelScan, Modifier.fillMaxWidth())
            else when (state.channel) {
                EnrollChannel.Qr, EnrollChannel.Sound -> SshubButton("Scan the terminal's code", on.onScan, Modifier.fillMaxWidth(), ButtonKind.Primary, enabled = ok, disabledReason = why)
                EnrollChannel.Bluetooth -> SshubButton("Find the terminal", on.onFindTerminal, Modifier.fillMaxWidth(), ButtonKind.Primary, enabled = ok, disabledReason = why)
                EnrollChannel.Paste -> SshubButton("Paste the text", on.onPasteText, Modifier.fillMaxWidth(), ButtonKind.Primary, enabled = ok, disabledReason = why)
            }
        },
    ) {
        Note("On the terminal start phone enrollment. It shows a short QR code.", Modifier.padding(bottom = 14.dp))
        Text("Name this phone will have in the ring", style = SshubText.fieldKey, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(bottom = 4.dp))
        MonoField(state.name, on.onName, label = "Name this phone will have in the ring", error = state.nameProblem)
        if (ok) ValidationLine(LineKind.Neutral, "This phone will be called ${state.name.trim()}. The phone checks the name again when the terminal's code is read.")
        SectionHeading("Channel", trailing = state.channel.label())
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Channels.forEach { (c, g, d) ->
                ChoiceRow(c.label(), c == state.channel, { on.onChannel(c) }, glyph = g, detail = d)
            }
        }
        Gap12()
        if (state.scanning && (state.channel == EnrollChannel.Qr || state.channel == EnrollChannel.Sound)) {
            ReceiveBlock(state.receive, on.receive)
        } else SshubCard {
            when (state.channel) {
                EnrollChannel.Qr -> Row(horizontalArrangement = Arrangement.spacedBy(14.dp), verticalAlignment = Alignment.CenterVertically) {
                    ViewfinderFrame(Modifier.size(104.dp).clearAndSetSemantics { })
                    Column(Modifier.weight(1f)) {
                        Text("Point at the terminal", style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
                        Note("Then hold this phone to its camera.")
                    }
                }
                EnrollChannel.Bluetooth -> PickerLine(GlyphKind.Bluetooth, "Pick from a list", "Pick it from the nearby SSHub devices by the tag the terminal shows.")
                EnrollChannel.Paste -> PickerLine(GlyphKind.Paste, "Text both ways", "Copy to the terminal, paste from it.")
                EnrollChannel.Sound -> PickerLine(GlyphKind.Sound, "Sound", "Fast 8 s, slow 16 s. This phone only plays.")
            }
            nextLines(state.channel).forEach { Note(it, Modifier.padding(top = 8.dp)) }
        }
    }
}

@Composable
private fun PickerLine(glyph: GlyphKind, title: String, body: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(14.dp), verticalAlignment = Alignment.CenterVertically) {
        Glyph(glyph, MaterialTheme.colorScheme.onSurface, Modifier.clearAndSetSemantics { }, size = 36.dp)
        Column(Modifier.weight(1f)) {
            Text(title, style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
            Note(body)
        }
    }
}

// ---- 2 Show (enroll-2) -------------------------------------------------------------------------------------------------

internal fun holdLine(c: EnrollChannel, terminal: String): String = when (c) {
    EnrollChannel.Qr -> "Hold this to the terminal camera until it confirms, then tap Next."
    EnrollChannel.Bluetooth -> "The terminal receives it over the Bluetooth link. Tap Next when it confirms."
    EnrollChannel.Paste -> "Copy the text and paste it into the terminal, then tap Next."
    EnrollChannel.Sound -> "Hold the phone near $terminal while it plays, then tap Next."
}

@Composable
fun EnrollShowScreen(state: EnrollShowState, on: EnrollShowActions) {
    EnrollFrame(state.terminal, 2, on.onBack, bar = { SshubButton("Next", on.onNext, Modifier.fillMaxWidth(), ButtonKind.Primary) }) {
        SectionHeading("Show this phone's commitment", first = true, trailing = state.send.channel.label())
        Note(holdLine(state.send.channel, state.terminal), Modifier.padding(bottom = 12.dp))
        SendBlock(state.send, on.send)
    }
}

// ---- 3 Keys (enroll-3) -------------------------------------------------------------------------------------------------

internal fun keysLine(c: EnrollChannel, terminal: String): String = when (c) {
    EnrollChannel.Qr, EnrollChannel.Sound -> "Scan the terminal's second QR code. It carries its keys."
    EnrollChannel.Bluetooth -> "The terminal sends its keys over the link."
    EnrollChannel.Paste -> "Paste the text the terminal shows for its keys."
}

@Composable
fun EnrollKeysScreen(state: EnrollKeysState, on: EnrollKeysActions) {
    val stopped = state.status == KeysStatus.Mismatch || state.status == KeysStatus.Stopped
    EnrollFrame(
        state.terminal, 3, on.onBack,
        bar = {
            when (state.status) {
                KeysStatus.Mismatch, KeysStatus.Stopped -> SshubButton("Start again", on.onStartAgain, Modifier.fillMaxWidth())
                KeysStatus.Ok -> SshubButton("Compare the words", on.onCompare, Modifier.fillMaxWidth(), ButtonKind.Primary)
                KeysStatus.Waiting -> SshubButton("Next", {}, Modifier.fillMaxWidth(), ButtonKind.Primary, subtitle = "waiting for the keys", enabled = false, disabledReason = "waiting for the keys")
            }
        },
    ) {
        SectionHeading("Terminal keys", first = true, trailing = state.receive.channel.label())
        if (stopped) {
            SecureWindow()
            // Mismatch is the one refusal that says the keys differ; any other stop is titled with the core's own sentence (M7).
            val mismatch = state.status == KeysStatus.Mismatch
            StopBox(if (mismatch) "The keys do not match the first code." else state.reason.ifEmpty { "Enrollment stopped." }, live = true) {
                if (mismatch) Text(
                    state.reason.ifEmpty { "Nothing was enrolled." } + " Do not retry on this channel; start again on the terminal.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onErrorContainer,
                ) else Text(
                    "Nothing was enrolled. Start again on the terminal.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onErrorContainer,
                )
                if (state.committed.isNotEmpty()) {
                    Text("first code committed to", style = SshubText.fieldKey, color = MaterialTheme.colorScheme.onErrorContainer)
                    HexGroups(state.committed, groups = null, label = "First code committed to")
                }
                if (state.shown.isNotEmpty()) {
                    Text("keys shown now open to", style = SshubText.fieldKey, color = MaterialTheme.colorScheme.onErrorContainer)
                    HexGroups(state.shown, groups = null, label = "Keys shown now open to")
                }
            }
        } else {
            Note(keysLine(state.receive.channel, state.terminal), Modifier.padding(bottom = 12.dp))
            ReceiveBlock(state.receive, on.receive)
            Gap12()
            SshubCard {
                if (state.status == KeysStatus.Ok) ValidationLine(LineKind.Ok, "The keys match the first code. Terminal calls itself ${state.terminal} [${state.terminalId}].")
                else ValidationLine(LineKind.Neutral, "Waiting for the keys. Then the phone checks them against the first code.")
            }
        }
    }
}

// ---- 4 Compare (enroll) ------------------------------------------------------------------------------------------------

@Composable
fun EnrollCompareScreen(state: EnrollCompareState, on: EnrollCompareActions) {
    val cs = MaterialTheme.colorScheme
    val t = state.terminal
    EnrollFrame(
        t, 4, on.onBack,
        bar = {
            when (state.phase) {
                // D15: neither button is pre-selected and neither is amber; both are always enabled.
                ComparePhase.Compare -> BarButtons {
                    SshubButton("They differ", on.onDiffer, kind = ButtonKind.Outline)
                    SshubButton("The words match", on.onMatch, kind = ButtonKind.Outline)
                }
                ComparePhase.Match -> SshubButton("Continue", on.onContinue, Modifier.fillMaxWidth(), ButtonKind.Primary, subtitle = "hold the confirmation")
                ComparePhase.Differ -> SshubButton("Start again", on.onStartAgain, Modifier.fillMaxWidth())
            }
        },
    ) {
        SecureWindow()
        if (state.phase == ComparePhase.Differ) {
            StopBox("Stop. The words differ.", live = true) {
                Text(
                    "Someone may sit between this phone and $t. Nothing was enrolled. Do not retry on this channel; check the terminal first.",
                    style = MaterialTheme.typography.bodyMedium, color = cs.onErrorContainer,
                )
            }
            Gap12()
            SendBlock(state.send, on.send, title = "Tell the terminal to stop")
            Note("Show this declined confirmation to $t so it stops too.", Modifier.padding(top = 8.dp))
        } else {
            SectionHeading("Compare the four words", first = true, trailing = "both screens")
            Column(
                Modifier.fillMaxWidth().border(2.dp, cs.onSurface, MaterialTheme.shapes.small).padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("words on $t must read the same, in this order", style = SshubText.fieldKey, color = cs.onSurfaceVariant)
                BigWords(state.words)
            }
            Gap12()
            SshubCard {
                if (state.phase == ComparePhase.Match) {
                    Text("You confirmed the words.", style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.Bold), color = MaterialTheme.sshub.ok)
                    Note("Next, hold this phone's confirmation to $t, then read its answer. The terminal adds this phone to the ring.")
                } else Note("Here both sides always have a screen, so the four words are a real check, unlike a machine reached over ssh. Different words mean someone sits between you: tap They differ.")
            }
            KeyValue("terminal", "$t [${state.terminalId}]")
            KeyValue("channel", "${state.channel.label()}, steps 1 to 3 done")
            SectionHeading("This phone's keys", trailing = "for $t")
            Note("$t needs them too. Keep showing them until it has read them.", Modifier.padding(bottom = 10.dp))
            SendBlock(state.send, on.send)
        }
    }
}

// ---- 5 Confirm (enroll-5) ----------------------------------------------------------------------------------------------

@Composable
fun EnrollConfirmScreen(state: EnrollConfirmState, on: EnrollConfirmActions) {
    val t = state.terminal
    EnrollFrame(
        t, 5, on.onBack,
        bar = {
            when (state.status) {
                ConfirmStatus.Bad -> SshubButton("Start again", on.onStartAgain, Modifier.fillMaxWidth())
                ConfirmStatus.Ok -> SshubButton("Finish", on.onFinish, Modifier.fillMaxWidth(), ButtonKind.Primary)
                ConfirmStatus.Waiting -> SshubButton("Waiting for $t", {}, Modifier.fillMaxWidth(), ButtonKind.Primary, subtitle = "show yours, then read its code", enabled = false, disabledReason = "waiting for the terminal confirmation")
            }
        },
    ) {
        when (state.status) {
            ConfirmStatus.Bad -> StopBox("The terminal confirmation has a bad signature.", live = true) {
                Text(
                    state.reason.ifEmpty { "Nothing was enrolled." } + " Do not retry on this channel; check the terminal first.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onErrorContainer,
                )
            }
            ConfirmStatus.Waiting -> {
                SectionHeading("Show this phone's confirmation", first = true, trailing = state.send.channel.label())
                Note("Hold it to $t, then read its answer below.", Modifier.padding(bottom = 12.dp))
                SendBlock(state.send, on.send)
                SectionHeading("$t confirmation", trailing = "waiting")
                ReceiveBlock(state.receive, on.receive)
            }
            ConfirmStatus.Ok -> {
                SectionHeading("$t confirmation", first = true, trailing = "received")
                SshubCard { ValidationLine(LineKind.Ok, "The terminal confirmation is received and its signature is good.") }
            }
        }
    }
}

// ---- done (enroll-done) ------------------------------------------------------------------------------------------------

@Composable
fun EnrolledScreen(state: EnrolledState, on: EnrolledActions) {
    val t = state.terminal
    ScreenFrame(
        title = "Enrolled", onBack = null,
        bar = { SshubButton("Done", on.onDone, Modifier.fillMaxWidth(), ButtonKind.Primary) },
        headerTrailing = { Text("step 5 of 5 done", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) },
    ) {
        EnrollSteps(6)
        Text("Enrolled", Modifier.padding(top = 12.dp, bottom = 6.dp), style = MaterialTheme.typography.headlineSmall, color = MaterialTheme.colorScheme.onSurface)
        Note("This phone is now known to $t [${state.terminalId}] as ${state.phoneName}.", Modifier.padding(bottom = 12.dp))
        SshubCard {
            ChainRow(SquareState.Fill, t, "${state.terminalId}, terminal holder", first = true, last = false, description = "$t, ${state.terminalId}, terminal holder")
            ChainRow(
                SquareState.Me, state.phoneName, "${state.phoneId}, known to $t", first = false, last = true, chip = "new",
                description = "${state.phoneName}, this phone, new, ${state.phoneId}, known to $t",
            )
        }
        Gap12()
        SshubCard { Note("Promotion to holder is a separate step on the terminal. Until then the terminal knows this phone but does not yet count it as a holder.") }
    }
}

// ---- Bluetooth chooser (enroll-bt) -------------------------------------------------------------------------------------

@Composable
fun EnrollBtScreen(state: EnrollBtState, on: EnrollBtActions) {
    val cs = MaterialTheme.colorScheme
    val t = state.terminal
    val tagOk = state.tag.length == 4
    val sel = state.devices.getOrNull(state.selected)
    val tagMatches = sel != null && tagOk && sel.tag.equals(state.tag, ignoreCase = true)
    // Two entries with one tag mean stop: either may be a stranger imitating the terminal (L8). Pair stays off until one is left.
    val sameTag = tagOk && state.devices.count { it.tag.equals(state.tag, ignoreCase = true) } > 1
    val canPair = state.phase == BtPhase.Found && tagMatches && !sameTag
    EnrollFrame(
        t, 1, on.onBack,
        bar = {
            when (state.phase) {
                BtPhase.Searching, BtPhase.Connecting -> SshubButton("Cancel", on.onBack, Modifier.fillMaxWidth())
                BtPhase.Found -> SshubButton(
                    if (canPair) "Pair with ${sel!!.name}" else "Pair", on.onPair, Modifier.fillMaxWidth(), ButtonKind.Primary,
                    subtitle = if (canPair) "tag ${state.tag}" else if (sameTag) "two devices show this tag" else "pick the row with your tag", enabled = canPair, disabledReason = if (sameTag) "two devices show this tag" else "pick the row with your tag",
                )
                BtPhase.Unsupported -> SshubButton("Other channel", on.onOtherChannel, Modifier.fillMaxWidth())
                else -> BarButtons {
                    SshubButton("Other channel", on.onOtherChannel)
                    if (state.phase == BtPhase.Denied) SshubButton("Allow Bluetooth", on.onAllow, kind = ButtonKind.Primary)
                    else SshubButton("Search again", on.onSearchAgain, kind = if (state.phase == BtPhase.Off) ButtonKind.Outline else ButtonKind.Primary)
                }
            }
        },
    ) {
        Text("Tag shown on $t", style = SshubText.fieldKey, color = cs.onSurfaceVariant, modifier = Modifier.padding(bottom = 4.dp))
        MonoField(state.tag, on.onTag, placeholder = "----", capitalize = true, label = "Tag shown on $t")
        ValidationLine(
            if (tagOk) LineKind.Ok else LineKind.Neutral,
            if (tagOk) "Tag ${state.tag} is complete." else if (state.tag.isEmpty()) "Type the 4-character tag shown on $t." else "The tag has 4 characters. ${4 - state.tag.length} to go.",
        )
        Gap12()
        SshubCard {
            Text("The tag only points to the right terminal.", style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.Bold), color = cs.onSurface)
            Note("It does not replace the four-word check in step 4. The link adds no security: every frame is signed or sealed end to end.")
        }
        Gap12()
        Column(
            Modifier.fillMaxWidth().background(cs.surfaceContainerLow, RoundedCornerShape(24.dp, 24.dp, 12.dp, 12.dp))
                .border(1.dp, cs.outlineVariant, RoundedCornerShape(24.dp, 24.dp, 12.dp, 12.dp)).padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(Modifier.align(Alignment.CenterHorizontally).width(36.dp).size(width = 36.dp, height = 4.dp).background(cs.onSurfaceVariant, RoundedCornerShape(2.dp)).clearAndSetSemantics { })
            Text("Nearby SSHub devices", style = MaterialTheme.typography.bodyLarge, color = cs.onSurface)
            state.problem?.let { p -> WarnBox("Bluetooth problem", live = true) { Note(p) } }
            when (state.phase) {
                BtPhase.Searching -> { Text("Looking for devices nearby", style = SshubText.monoBody, color = cs.onSurface); WaitingDots(word = "searching") }
                BtPhase.Found -> {
                    if (sameTag) WarnBox("Two devices show the tag ${state.tag}. Stop.", live = true) {
                        Note("Only one of them is the terminal. Do not pair with either: turn Bluetooth off on the other device or move away, then search again.")
                    }
                    state.devices.forEachIndexed { i, d -> BtRow(d, i == state.selected, tagOk && d.tag.equals(state.tag, ignoreCase = true)) { on.onSelect(i) } }
                    if (sel != null && !tagMatches) WarnBox(
                        if (tagOk) "Tag ${sel.tag} is not the tag you typed (${state.tag})." else "Type the tag shown on $t first.", live = true,
                    ) { Note("Pick the row with the same tag as the terminal screen.") }
                }
                BtPhase.None -> BtMessage(GlyphKind.Bluetooth, "No SSHub device with tag ${state.tag.ifEmpty { "----" }} found", "Check that Bluetooth is armed on $t and the tag matches, then search again.")
                BtPhase.Denied -> BtMessage(GlyphKind.Warning, "Bluetooth permission was not granted", "Without it this phone cannot look for $t. Allow it, or use another channel.")
                BtPhase.Off -> BtMessage(GlyphKind.Bluetooth, "Bluetooth is off", "Turn it on to look for $t, or use another channel.")
                BtPhase.Unsupported -> BtMessage(GlyphKind.Bluetooth, "This phone has no Bluetooth LE", "Use another channel.")
                BtPhase.Connecting -> { Text("Linking with SSHub-${state.tag}", style = SshubText.monoBody, color = cs.onSurface); Note("Waiting for the code of $t over the link."); WaitingDots(word = "link opening") }
                BtPhase.LinkFailed -> BtMessage(GlyphKind.Warning, "The link could not be opened", "Check that $t is waiting for the phone, then search again.")
            }
        }
    }
}

@Composable
private fun BtMessage(glyph: GlyphKind, title: String, body: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        Glyph(glyph, MaterialTheme.colorScheme.onSurface, Modifier.clearAndSetSemantics { }, size = 32.dp)
        Column(Modifier.weight(1f)) {
            Text(title, style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
            Note(body)
        }
    }
}

@Composable
private fun BtRow(d: BleDevice, selected: Boolean, hit: Boolean, onClick: () -> Unit) {
    val cs = MaterialTheme.colorScheme
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp)
            .background(if (selected) cs.secondaryContainer else androidx.compose.ui.graphics.Color.Transparent, MaterialTheme.shapes.medium)
            .border(if (hit) 2.dp else 1.dp, if (hit) cs.onSurface else MaterialTheme.sshub.boundary, MaterialTheme.shapes.medium)
            .selectable(selected, role = Role.RadioButton, onClick = onClick).padding(horizontal = 14.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Glyph(GlyphKind.Bluetooth, cs.onSurface, Modifier.clearAndSetSemantics { })
        Column(Modifier.weight(1f)) {
            Text(d.name, style = SshubText.monoBody, color = cs.onSurface)
            Text(d.address, style = MaterialTheme.typography.bodySmall, color = cs.onSurfaceVariant)
        }
        if (hit) Row(verticalAlignment = Alignment.CenterVertically) {
            Glyph(GlyphKind.Check, cs.onSurface, size = 16.dp)
            Text("matches tag", style = MaterialTheme.typography.labelMedium.copy(fontWeight = FontWeight.Bold), color = cs.onSurface)
        }
    }
}

// ---- Paste (enroll-paste) ----------------------------------------------------------------------------------------------

private fun pasteIntro(n: Int): String = when (n) {
    1 -> "Paste the short code the terminal shows as text."
    2 -> "Copy this phone's commitment and paste it into the terminal."
    3 -> "Paste the text the terminal shows for its keys."
    else -> "Copy this phone's confirmation into the terminal, then paste its answer."
}

@Composable
fun EnrollPasteScreen(state: EnrollPasteState, on: EnrollPasteActions) {
    val n = state.n
    val inA = n == 1 || n == 3 || n == 5
    val fromBlock: @Composable () -> Unit = {
        SshubCard {
            CardTitle("From the terminal", if (inA) "step $n frame" else "not needed here")
            if (inA) {
                TextArea(state.text, on.onText, label = "Text from the terminal", placeholder = "Paste ${pasteWhat(n)} here")
                Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SshubButton("Paste", on.onPaste, Modifier.weight(1f))
                    SshubButton("Clear", on.onClear, Modifier.weight(1f))
                }
                ValidationLine(
                    when (state.validation.verdict) { PasteVerdict.Ok -> LineKind.Ok; PasteVerdict.Bad -> LineKind.Bad; PasteVerdict.Empty -> LineKind.Neutral },
                    state.validation.message,
                )
            } else Note("This step sends nothing to this phone.")
        }
    }
    val toBlock: @Composable () -> Unit = {
        if (state.send != null) SendBlock(state.send, on.send, title = "To the terminal", right = "step $n frame")
        else SshubCard {
            CardTitle("To the terminal", "not needed here")
            Note("Nothing to copy at this step.")
        }
    }
    EnrollFrame(
        state.terminal, n, on.onBack,
        bar = {
            if (n == 2) SshubButton("Next", on.onNext, Modifier.fillMaxWidth(), ButtonKind.Primary)
            else SshubButton(
                when (n) { 1 -> "Use this text"; 3 -> "Check the keys"; else -> "Check the signature" }, on.onUse, Modifier.fillMaxWidth(), ButtonKind.Primary,
                enabled = state.validation.verdict == PasteVerdict.Ok, disabledReason = state.validation.message,
            )
        },
    ) {
        SectionHeading("Step $n of 5", first = true, trailing = "Paste text")
        Note(pasteIntro(n), Modifier.padding(bottom = 12.dp))
        if (n == 5) { toBlock(); Gap12(); fromBlock() } else { fromBlock(); Gap12(); toBlock() }
    }
}

// ---- Sound (enroll-sound) ----------------------------------------------------------------------------------------------

@Composable
fun EnrollSoundScreen(state: EnrollSoundScreenState, on: EnrollSoundActions) {
    EnrollFrame(state.terminal, state.step, on.onBack, bar = { SshubButton("Next", on.onContinue, Modifier.fillMaxWidth(), ButtonKind.Primary) }) {
        SectionHeading("Play this phone's reply", first = true, trailing = "Sound")
        Note("Hold the phone near ${state.terminal} while it plays, then tap Next. This phone never records: the terminal does the listening.", Modifier.padding(bottom = 12.dp))
        state.problem?.let { p -> WarnBox("The sound could not be played", Modifier.padding(bottom = 12.dp), live = true) { Note(p) } }
        if (state.volumeZero) WarnBox("The media volume is zero", Modifier.padding(bottom = 12.dp), live = true) {
            Note("Raise the media volume with the volume keys, or the terminal hears nothing.")
        }
        SendBlock(state.send, on.send, title = "To the terminal")
    }
}
