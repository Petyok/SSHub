package uk.petruha.sshub.phone.ui.screens.enroll

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import uk.petruha.sshub.phone.core.CameraPermission
import uk.petruha.sshub.phone.core.EnrollChannel
import uk.petruha.sshub.phone.core.LocalPlatform
import uk.petruha.sshub.phone.ui.components.ButtonKind
import uk.petruha.sshub.phone.ui.components.ChainSquare
import uk.petruha.sshub.phone.ui.components.ChoiceRow
import uk.petruha.sshub.phone.ui.components.Glyph
import uk.petruha.sshub.phone.ui.components.GlyphKind
import uk.petruha.sshub.phone.ui.components.Note
import uk.petruha.sshub.phone.ui.components.QrView
import uk.petruha.sshub.phone.ui.components.ScreenFrame
import uk.petruha.sshub.phone.ui.components.SquareState
import uk.petruha.sshub.phone.ui.components.SshubButton
import uk.petruha.sshub.phone.ui.components.SshubCard
import uk.petruha.sshub.phone.ui.components.TextArea
import uk.petruha.sshub.phone.ui.components.TickScale
import uk.petruha.sshub.phone.ui.components.ViewfinderFrame
import uk.petruha.sshub.phone.ui.components.WaitingDots
import uk.petruha.sshub.phone.ui.theme.LocalReducedMotion
import uk.petruha.sshub.phone.ui.theme.SshubText
import uk.petruha.sshub.phone.ui.theme.sshub

internal fun EnrollChannel.label(): String = when (this) {
    EnrollChannel.Qr -> "Camera QR"
    EnrollChannel.Bluetooth -> "Bluetooth"
    EnrollChannel.Paste -> "Paste text"
    EnrollChannel.Sound -> "Sound"
}

internal const val QR_DESCRIPTION = "QR code with the enrolment offer. Use the text code below or ask the other device to read it"

/** The glossary's names of the five steps; the terminal and the page use the same ones. */
private val StepNames = listOf("Terminal code", "Phone code", "Terminal keys", "Compare the words", "Finish")

/** The five steps as chain squares joined by a line: done filled with a check, current amber, later empty. `n` 6 means all done. */
@Composable
internal fun EnrollSteps(n: Int, modifier: Modifier = Modifier) {
    val cs = MaterialTheme.colorScheme
    val line = cs.onSurfaceVariant
    val stack = LocalDensity.current.fontScale >= 1.3f
    val shown = n.coerceIn(1, 5)
    val spoken = if (n > 5) "Steps 1 to 5 done" else "Step $shown of 5, ${StepNames[shown - 1]}"
    Column(modifier.fillMaxWidth().semantics(mergeDescendants = true) { contentDescription = spoken }) {
        Row(
            Modifier.fillMaxWidth().drawBehind {
                val col = size.width / 5
                drawLine(line, Offset(col / 2, 16.dp.toPx()), Offset(size.width - col / 2, 16.dp.toPx()), 2.dp.toPx())
            },
        ) {
            StepNames.forEachIndexed { i, name ->
                val k = i + 1
                Column(Modifier.weight(1f).clearAndSetSemantics { }, horizontalAlignment = Alignment.CenterHorizontally) {
                    Box(Modifier.background(cs.background)) {
                        ChainSquare(if (k < n) SquareState.Fill else if (k == n) SquareState.Current else SquareState.Default)
                    }
                    if (!stack) Text(
                        name, Modifier.padding(top = 4.dp),
                        style = MaterialTheme.typography.labelMedium.copy(fontWeight = if (k > n) FontWeight.Normal else FontWeight.Bold),
                        color = if (k > n) cs.onSurfaceVariant else cs.onSurface,
                    )
                }
            }
        }
        if (stack && n <= 5) Text(
            "Step $shown of 5: ${StepNames[shown - 1]}", Modifier.padding(top = 6.dp),
            style = MaterialTheme.typography.labelMedium, color = cs.onSurface,
        )
    }
}

@Composable
internal fun EnrollFrame(
    terminal: String,
    step: Int,
    onBack: (() -> Unit)?,
    bar: (@Composable () -> Unit)?,
    title: String = "Connect to $terminal",
    trailing: String = "Step $step of 5",
    content: @Composable ColumnScope.() -> Unit,
) = ScreenFrame(
    title = title, onBack = onBack, bar = bar,
    headerTrailing = { Text(trailing, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) },
    content = { EnrollSteps(step); Gap12(); content() },
)

@Composable
internal fun Gap12() = androidx.compose.foundation.layout.Spacer(Modifier.size(12.dp))

internal enum class LineKind { Neutral, Ok, Bad }

/** One validation line: glyph and words, never colour alone. Polite live region so a screen reader hears the change. */
@Composable
internal fun ValidationLine(kind: LineKind, text: String, modifier: Modifier = Modifier) {
    val sc = MaterialTheme.sshub
    val cs = MaterialTheme.colorScheme
    Row(
        modifier.fillMaxWidth().heightIn(min = 28.dp).padding(top = 6.dp).semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Glyph(
            when (kind) { LineKind.Ok -> GlyphKind.Check; LineKind.Bad -> GlyphKind.Cross; LineKind.Neutral -> GlyphKind.Info },
            when (kind) { LineKind.Ok -> sc.ok; LineKind.Bad -> sc.dangerText; LineKind.Neutral -> cs.onSurfaceVariant },
            Modifier.clearAndSetSemantics { }, size = 18.dp,
        )
        Text(
            text, style = MaterialTheme.typography.bodyMedium.let { if (kind == LineKind.Bad) it.copy(fontWeight = FontWeight.Bold) else it },
            color = if (kind == LineKind.Bad) sc.dangerText else cs.onSurface,
        )
    }
}

@Composable
internal fun CardTitle(title: String, right: String) {
    Row(Modifier.fillMaxWidth().padding(bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(title, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurface)
        Text(right, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

// ---- receiving -----------------------------------------------------------------------------------------------------

@Composable
internal fun ReceiveBlock(r: EnrollReceiveState, on: EnrollReceiveActions, modifier: Modifier = Modifier) {
    SshubCard(modifier) {
        CardTitle("From the terminal", r.channel.label())
        when (r.channel) {
            EnrollChannel.Bluetooth -> {
                Text("Receiving over Bluetooth", style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
                Note("From SSHub-${r.tag}, ${r.terminal}")
                Gap12()
                WaitingDots(word = "link open")
            }
            else -> CameraReceive(r, on)
        }
        r.note?.let { ValidationLine(LineKind.Bad, it) }
        if (r.channel != EnrollChannel.Bluetooth && r.camera == CameraPermission.Granted) {
            if (r.need > 0) {
                Text("frame ${r.have} of ${r.need} read", Modifier.padding(top = 8.dp), style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
                TickScale(r.need, 1, r.have.toFloat() / r.need, "Frames read: ${r.have} of ${r.need}")
            } else Note("No frame read yet.", Modifier.padding(top = 8.dp))
        }
    }
}

@Composable
private fun CameraReceive(r: EnrollReceiveState, on: EnrollReceiveActions) {
    val cs = MaterialTheme.colorScheme
    when (r.camera) {
        CameraPermission.Granted -> {
            ViewfinderFrame(Modifier.widthIn(max = 260.dp)) {
                LocalPlatform.current.camera.Preview(Modifier, on.onLuma)
            }
            Text(r.prompt, Modifier.padding(top = 8.dp), style = SshubText.monoBody, color = cs.onSurface)
            Note("Camera ready. Point it at the terminal.")
        }
        CameraPermission.DeniedForever -> {
            Text("Camera access is off for SSHub", style = SshubText.monoBody, color = cs.onSurface)
            Note("The phone reads the terminal's codes with the camera. Allow it in the system settings, or use another channel.")
            SshubButton("Open settings", on.onOpenSettings, Modifier.fillMaxWidth().padding(top = 8.dp))
        }
        else -> {
            Text("Camera access is needed", style = SshubText.monoBody, color = cs.onSurface)
            Note("The phone reads the terminal's codes with the camera. Nothing is recorded or stored.")
            SshubButton("Allow camera", on.onRequestCamera, Modifier.fillMaxWidth().padding(top = 8.dp))
        }
    }
}

// ---- sending -------------------------------------------------------------------------------------------------------

@Composable
internal fun SendBlock(
    s: EnrollSendState,
    on: EnrollSendActions,
    modifier: Modifier = Modifier,
    title: String = "To the terminal",
    right: String = s.channel.label(),
) {
    SshubCard(modifier) {
        CardTitle(title, right)
        when (s.channel) {
            EnrollChannel.Qr -> QrSend(s, on)
            EnrollChannel.Bluetooth -> {
                Text("Sending over Bluetooth", style = SshubText.monoBody, color = MaterialTheme.colorScheme.onSurface)
                Note("To ${s.terminal}")
                Gap12()
                WaitingDots(word = "link open")
            }
            EnrollChannel.Paste -> CopyText(s, on)
            EnrollChannel.Sound -> SoundPlayer(s.sound, s.terminal, on)
        }
    }
}

@Composable
private fun QrSend(s: EnrollSendState, on: EnrollSendActions) {
    val cs = MaterialTheme.colorScheme
    if (s.qr.isEmpty()) {
        Note("This reply is too large for a QR code. Use the text below, or another channel.")
    } else {
        var i by remember(s.qr) { mutableIntStateOf(0) }
        LaunchedEffect(s.qr) { if (s.qr.size > 1) while (true) { delay(250); i = (i + 1) % s.qr.size } }
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { QrView(s.qr[i], QR_DESCRIPTION) }
        Text("frame ${i + 1} of ${s.qr.size}", Modifier.padding(top = 6.dp), style = SshubText.monoBody, color = cs.onSurface)
        TickScale(s.qr.size, 1, (i + 1f) / s.qr.size, "QR frame ${i + 1} of ${s.qr.size}, the code loops")
        Note("One lap is ${s.qr.size} frames, then it loops.")
    }
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button, onClick = on.onToggleText)
            .semantics { stateDescription = if (s.textOpen) "Expanded" else "Collapsed" },
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("Show as text for a terminal without a camera", Modifier.weight(1f), style = MaterialTheme.typography.labelLarge, color = cs.onSurface)
        Glyph(GlyphKind.Chevron, cs.onSurfaceVariant, Modifier.rotate(if (s.textOpen) 90f else 0f).clearAndSetSemantics { })
    }
    if (s.textOpen) CopyText(s, on)
}

/** Base64 text and Copy through the clipboard port (D19: only on tap, sensitive flag set by the port, 60 s clear shown). */
@Composable
internal fun CopyText(s: EnrollSendState, on: EnrollSendActions) {
    TextArea(s.text, {}, label = "Text for the terminal, the same bytes as the QR code, base64", readOnly = true)
    val copied = s.clipboardClearsIn != null
    SshubButton(if (copied) "Copied" else "Copy the text", on.onCopy, Modifier.fillMaxWidth().padding(top = 8.dp))
    Text(
        if (s.clipboardClearsIn != null) "Copied, clears in ${s.clipboardClearsIn} s. Paste it into the terminal." else "The same bytes as the QR code, as base64.",
        Modifier.padding(top = 8.dp).semantics { liveRegion = LiveRegionMode.Polite },
        style = SshubText.secondaryMono, color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Note("Marked sensitive: the system hides it from clipboard previews and it is cleared after 60 s.", Modifier.padding(top = 4.dp))
}

/** This phone only plays. Fast about 8 s, slow about 16 s; the level bar is static under reduced motion. */
@Composable
internal fun SoundPlayer(s: EnrollSoundState, terminal: String, on: EnrollSendActions) {
    val cs = MaterialTheme.colorScheme
    val playing = s.phase == SoundPhase.Playing
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        ChoiceRow(
            "Fast", s.profile == SoundProfile.Fast, { if (!playing) on.onProfile(SoundProfile.Fast) },
            Modifier.semantics { if (playing) disabled() }, detail = "about 8 s for the reply",
        )
        ChoiceRow(
            "Slow", s.profile == SoundProfile.Slow, { if (!playing) on.onProfile(SoundProfile.Slow) },
            Modifier.semantics { if (playing) disabled() }, detail = "about 16 s, noisy rooms",
        )
        Note("Audio is opt-in on the terminal: turn it on there first. The phone does not record, the terminal does the listening. The times are about; they were not measured over the air.")
        Row(Modifier.fillMaxWidth().padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            PlayingMeter(playing)
            Column {
                Text(
                    when (s.phase) { SoundPhase.Idle -> "Ready"; SoundPhase.Playing -> "Playing"; SoundPhase.Done -> "Played" },
                    style = SshubText.monoBody, color = cs.onSurface, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                )
                Note("Hold the phone near $terminal.")
            }
        }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text("${s.elapsed} s", Modifier.weight(1f), style = SshubText.monoBody, color = cs.onSurface)
            Text("of about ${s.total} s", style = MaterialTheme.typography.labelMedium, color = cs.onSurfaceVariant)
        }
        TickScale(s.total.coerceAtLeast(1), if (s.total > 10) 4 else 2, s.elapsed.toFloat() / s.total.coerceAtLeast(1), "Played ${s.elapsed} seconds of about ${s.total}")
        if (playing) SshubButton("Stop", on.onStop, Modifier.fillMaxWidth())
        else SshubButton(
            if (s.phase == SoundPhase.Done) "Play again" else "Play", on.onPlay, Modifier.fillMaxWidth(),
            kind = ButtonKind.Outline, subtitle = if (s.profile == SoundProfile.Fast) "fast profile" else "slow profile",
            enabled = s.available, disabledReason = "this reply has no sound version",
        )
    }
}

/** Fourteen bars while playing; one static level bar otherwise and under reduced motion (spec 8). */
@Composable
private fun PlayingMeter(active: Boolean) {
    val cs = MaterialTheme.colorScheme
    val reduced = LocalReducedMotion.current
    Box(Modifier.height(44.dp).clearAndSetSemantics { }, contentAlignment = Alignment.BottomStart) {
        if (!active || reduced) {
            Box(Modifier.width(88.dp).height(if (active) 24.dp else 6.dp).background(cs.onSurfaceVariant, RoundedCornerShape(3.dp)))
        } else {
            val t = rememberInfiniteTransition(label = "meter")
            Row(horizontalArrangement = Arrangement.spacedBy(3.dp), verticalAlignment = Alignment.Bottom) {
                for (i in 0 until 14) {
                    val h by t.animateFloat(
                        6f, 44f, infiniteRepeatable(tween(1000, delayMillis = (i * 130) % 1000, easing = FastOutSlowInEasing), RepeatMode.Reverse), label = "bar$i",
                    )
                    Box(Modifier.width(3.dp).height(h.dp).background(cs.onSurface, RoundedCornerShape(2.dp)))
                }
            }
        }
    }
}
