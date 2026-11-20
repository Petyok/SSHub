package io.github.petyok.sshub.ui.components

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import io.github.petyok.sshub.ui.theme.SshubText
import io.github.petyok.sshub.ui.theme.sshub

enum class NavTab(val label: String, val glyph: GlyphKind) {
    Ring("Ring", GlyphKind.Ring), Approvals("Approvals", GlyphKind.Approvals), Enroll("Enroll", GlyphKind.Enroll), Security("Security", GlyphKind.Security)
}

/** Spec 7.9: M3 NavigationBar, 4 tabs, labels wrap, badge for pending approvals (0 is not shown). */
@Composable
fun SshubNavBar(selected: NavTab?, pending: Int, onSelect: (NavTab) -> Unit, modifier: Modifier = Modifier) {
    Column(modifier) {
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        NavigationBar(containerColor = MaterialTheme.colorScheme.surfaceContainer, tonalElevation = 0.dp) {
            NavTab.entries.forEach { tab ->
                val on = tab == selected
                NavigationBarItem(
                    selected = on, onClick = { onSelect(tab) },
                    icon = {
                        Box(contentAlignment = Alignment.TopEnd) {
                            Glyph(tab.glyph, if (on) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant)
                            if (tab == NavTab.Approvals && pending > 0) CountBadge(pending, Modifier.padding(start = 14.dp))
                        }
                    },
                    label = { Text(tab.label, style = MaterialTheme.typography.labelMedium.copy(fontWeight = if (on) FontWeight.Bold else FontWeight.Normal), softWrap = true) },
                    alwaysShowLabel = true,
                    colors = NavigationBarItemDefaults.colors(
                        selectedIconColor = MaterialTheme.colorScheme.onSurface, selectedTextColor = MaterialTheme.colorScheme.onSurface,
                        unselectedIconColor = MaterialTheme.colorScheme.onSurfaceVariant, unselectedTextColor = MaterialTheme.colorScheme.onSurfaceVariant,
                        indicatorColor = MaterialTheme.colorScheme.secondaryContainer,
                    ),
                    modifier = Modifier.semantics { if (tab == NavTab.Approvals && pending > 0) contentDescription = "Approvals, $pending waiting" },
                )
            }
        }
    }
}
