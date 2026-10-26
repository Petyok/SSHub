package uk.petruha.sshub.phone.ui.components

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
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.error
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import uk.petruha.sshub.phone.ui.theme.SshubShape
import uk.petruha.sshub.phone.ui.theme.SshubText
import uk.petruha.sshub.phone.ui.theme.sshub

/** Row toggle (spec 7.8): the whole row is the target, `Role.Switch`, on/off also by thumb position and a check. */
@Composable
fun ToggleRow(label: String, checked: Boolean, onChange: (Boolean) -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    val cs = MaterialTheme.colorScheme
    Row(
        modifier.fillMaxWidth().heightIn(min = 56.dp)
            .toggleable(checked, enabled = enabled, role = Role.Switch, onValueChange = onChange)
            .semantics { stateDescription = if (checked) "On" else "Off" },
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(label, Modifier.weight(1f), style = SshubText.monoBody, color = cs.onSurface)
        Box(
            Modifier.width(52.dp).size(width = 52.dp, height = 32.dp)
                .background(if (checked) cs.secondary else cs.surfaceContainerHigh, CircleShape)
                .border(2.dp, if (checked) cs.secondary else cs.onSurfaceVariant, CircleShape).clearAndSetSemantics { },
            contentAlignment = if (checked) Alignment.CenterEnd else Alignment.CenterStart,
        ) {
            Box(Modifier.padding(horizontal = 4.dp).size(if (checked) 20.dp else 16.dp).background(if (checked) cs.onSecondary else cs.onSurfaceVariant, CircleShape), contentAlignment = Alignment.Center) {
                if (checked) Glyph(GlyphKind.Check, cs.secondary, size = 12.dp, stroke = 2.dp)
            }
        }
    }
}

/** Checkbox row (`.ckb`): 24dp box, 2dp border, radius 5, whole row toggleable. */
@Composable
fun CheckboxRow(label: String, checked: Boolean, onChange: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val cs = MaterialTheme.colorScheme
    Row(
        modifier.fillMaxWidth().heightIn(min = 48.dp).toggleable(checked, role = Role.Checkbox, onValueChange = onChange),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(
            Modifier.size(24.dp).background(if (checked) cs.onSurface else androidx.compose.ui.graphics.Color.Transparent, RoundedCornerShape(5.dp))
                .border(2.dp, cs.onSurface, RoundedCornerShape(5.dp)).clearAndSetSemantics { },
            contentAlignment = Alignment.Center,
        ) { if (checked) Glyph(GlyphKind.Check, cs.background, size = 18.dp, stroke = 3.dp) }
        Text(label, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = cs.onSurface)
    }
}

/** Typed-id / text field (spec 7.12): 2dp onSurface border, mono Bold 20. Error = danger border + cross + words. */
@Composable
fun MonoField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    error: String? = null,
    password: Boolean = false,
    numeric: Boolean = false,
    capitalize: Boolean = false,
    label: String,
    visual: VisualTransformation = VisualTransformation.None,
) {
    val cs = MaterialTheme.colorScheme
    val sc = MaterialTheme.sshub
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        BasicTextField(
            value, onValueChange,
            Modifier.fillMaxWidth().heightIn(min = 56.dp)
                .background(cs.background, SshubShape.field).border(2.dp, if (error != null) sc.danger else cs.onSurface, SshubShape.field)
                .semantics { contentDescription = label; if (error != null) error(error) },
            textStyle = SshubText.input.merge(TextStyle(color = cs.onSurface)),
            cursorBrush = SolidColor(cs.onSurface),
            visualTransformation = if (password) PasswordVisualTransformation() else visual,
            keyboardOptions = KeyboardOptions(
                capitalization = if (capitalize) KeyboardCapitalization.Characters else KeyboardCapitalization.None,
                autoCorrectEnabled = false,
                keyboardType = if (password && numeric) KeyboardType.NumberPassword else if (password) KeyboardType.Password else KeyboardType.Ascii,
                imeAction = ImeAction.Done,
            ),
            decorationBox = { inner ->
                Box(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 14.dp, vertical = 12.dp), contentAlignment = Alignment.CenterStart) {
                    if (value.isEmpty() && placeholder.isNotEmpty()) Text(placeholder, style = SshubText.input.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Normal), color = cs.onSurfaceVariant)
                    inner()
                }
            },
        )
        if (error != null) Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
            Glyph(GlyphKind.Cross, sc.dangerText, size = 16.dp)
            Text(error, style = MaterialTheme.typography.bodyMedium, color = sc.dangerText)
        }
    }
}

/** Multi-line paste box (spec 7.12): 84dp min, 1dp border, mono 14. */
@Composable
fun TextArea(value: String, onValueChange: (String) -> Unit, label: String, modifier: Modifier = Modifier, placeholder: String = "", readOnly: Boolean = false) {
    val cs = MaterialTheme.colorScheme
    BasicTextField(
        value, onValueChange, modifier.fillMaxWidth().heightIn(min = 84.dp)
            .border(1.dp, cs.onSurfaceVariant, MaterialTheme.shapes.small).padding(10.dp).semantics { contentDescription = label },
        readOnly = readOnly,
        textStyle = SshubText.monoBody.merge(TextStyle(color = cs.onSurface)),
        cursorBrush = SolidColor(cs.onSurface),
        keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Ascii),
        decorationBox = { inner ->
            Box { if (value.isEmpty() && placeholder.isNotEmpty()) Text(placeholder, style = SshubText.monoBody, color = cs.onSurfaceVariant); inner() }
        },
    )
}

/** Segmented switch (spec 7.9). Selected = secondaryContainer + a check glyph, optional count. */
@Composable
fun SegmentedHeader(options: List<Pair<String, Int?>>, selected: Int, onSelect: (Int) -> Unit, modifier: Modifier = Modifier) {
    val sc = MaterialTheme.sshub
    val cs = MaterialTheme.colorScheme
    Row(modifier.fillMaxWidth().heightIn(min = 48.dp).border(1.dp, sc.boundary, RoundedCornerShape(24.dp)).background(cs.background, RoundedCornerShape(24.dp))) {
        options.forEachIndexed { i, (label, count) ->
            val on = i == selected
            Row(
                Modifier.weight(1f).heightIn(min = 48.dp)
                    .background(if (on) cs.secondaryContainer else androidx.compose.ui.graphics.Color.Transparent, RoundedCornerShape(24.dp))
                    .selectable(on, role = Role.Tab) { onSelect(i) },
                horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically,
            ) {
                if (on) Glyph(GlyphKind.Check, cs.onSurface, Modifier.padding(end = 6.dp), size = 16.dp)
                Text(label, style = SshubText.monoBody, color = cs.onSurface)
                if (count != null) CountBadge(count, Modifier.padding(start = 6.dp), muted = count == 0)
            }
        }
    }
}

/** Channel-style chooser button group (`.chs`): the selected one has a 2dp border and a check. */
@Composable
fun ChoiceRow(label: String, selected: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier, glyph: GlyphKind? = null, detail: String? = null) {
    val cs = MaterialTheme.colorScheme
    Row(
        modifier.fillMaxWidth().heightIn(min = 56.dp)
            .border(if (selected) 2.dp else 1.dp, if (selected) cs.onSurface else MaterialTheme.sshub.boundary, MaterialTheme.shapes.medium)
            .selectable(selected, role = Role.RadioButton, onClick = onClick).padding(horizontal = 14.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (glyph != null) Glyph(glyph, cs.onSurface, Modifier.clearAndSetSemantics { })
        Column(Modifier.weight(1f)) {
            Text(label, style = SshubText.monoBody, color = cs.onSurface)
            if (detail != null) Text(detail, style = MaterialTheme.typography.bodySmall, color = cs.onSurfaceVariant)
        }
        if (selected) Glyph(GlyphKind.Check, cs.onSurface, size = 20.dp)
    }
}
