package io.github.petyok.sshub.release

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasText

/** A node whose content description (any entry of a merged node) starts with `prefix`. */
fun descStartsWith(prefix: String) = SemanticsMatcher("content description starts with \"$prefix\"") { n ->
    n.config.getOrNull(SemanticsProperties.ContentDescription)?.any { it.startsWith(prefix) } == true
}

/** A node with text that contains `part`. */
fun textContains(part: String) = hasText(part, substring = true)
