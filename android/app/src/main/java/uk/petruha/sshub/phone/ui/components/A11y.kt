package uk.petruha.sshub.phone.ui.components

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.platform.LocalConfiguration

/** "A3F9 007C" is read as "A 3 F 9, 0 0 7 C" (spec 9): characters separated, groups pause. */
fun spokenHex(groups: List<String>): String = groups.joinToString(", ") { g -> g.uppercase().toList().joinToString(" ") }

/** Splits a hex string into groups of 4; the last group may be shorter. Never splits inside a group when rendered. */
fun hexGroups(hex: String, size: Int = 4): List<String> = hex.filter { !it.isWhitespace() && it != '-' }.chunked(size)

/** Spec 4.3: 1 column / 2 columns / 3 columns thresholds follow the effective font scale. */
@Composable
@ReadOnlyComposable
fun fontScale(): Float = LocalConfiguration.current.fontScale
