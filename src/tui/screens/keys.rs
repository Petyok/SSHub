use ratatui::layout::Rect;
use ratatui::prelude::*;

use crate::app::App;
use crate::ssh::agent::AgentInfo;
use crate::store::Identity;
use crate::theme::catalog::{ColorRole, PaintRole, StyleRole};
use crate::theme::model::ResolvedTheme;

const CARD_W: u16 = 42;
const CARD_H: u16 = 6;
/// Narrowest a card may shrink to before content becomes unreadable.
const MIN_CARD_W: u16 = 26;
const CARD_GAP: u16 = 2;
/// Row of the body the empty-state message occupies. The agent block starts
/// below it, so the two can never share a line.
const EMPTY_ROW: u16 = 2;

/// One card row plus the blank line under it.
const fn row_stride_const() -> u16 {
    CARD_H + 1
}

/// Inner content width of the identities body for a given total width
/// (mirrors the margin logic in [`render_keys`]).
pub fn inner_width(total_width: u16) -> u16 {
    let margin = if total_width >= 132 {
        2
    } else if total_width >= 80 {
        1
    } else {
        0
    };
    total_width.saturating_sub(margin * 2)
}

/// How many columns of at least [`MIN_CARD_W`] fit into `inner_w`.
pub fn max_columns(inner_w: u16) -> usize {
    (((inner_w + CARD_GAP) / (MIN_CARD_W + CARD_GAP)) as usize).max(1)
}

/// Resolve the column count for the identities grid. `pref == 0` means auto
/// (the original 1-or-2 heuristic); otherwise the user's choice, clamped to
/// what fits.
pub fn resolve_columns(inner_w: u16, pref: usize) -> usize {
    if pref == 0 {
        if inner_w >= CARD_W * 2 + CARD_GAP {
            2
        } else {
            1
        }
    } else {
        pref.clamp(1, max_columns(inner_w))
    }
}

/// The `components.identities.*` roles one identity card is painted from,
/// resolved once per frame.
///
/// This family is the cards' own. The row-based identity form in
/// `screens/keychain.rs` is a form and reads `components.form.*`; neither
/// surface borrows the generic table roles.
#[derive(Clone, Copy)]
struct CardStyles {
    border: ratatui::style::Color,
    border_selected: ratatui::style::Color,
    selection: Style,
    name: Style,
    text: Style,
    metadata: Style,
    key_type: Style,
    loaded: ratatui::style::Color,
    missing: ratatui::style::Color,
    credential: ratatui::style::Color,
}

impl CardStyles {
    fn of(theme: &ResolvedTheme, card: Rect) -> Self {
        Self {
            border: crate::tui::blit::line_color(theme, PaintRole::IdentitiesCardBorder, card),
            border_selected: crate::tui::blit::line_color(
                theme,
                PaintRole::IdentitiesCardBorderSelected,
                card,
            ),
            selection: theme.style(StyleRole::IdentitiesCardSelection),
            name: theme.style(StyleRole::IdentitiesCardName),
            text: theme.style(StyleRole::IdentitiesCardText),
            metadata: theme.style(StyleRole::IdentitiesCardMetadata),
            key_type: theme.style(StyleRole::IdentitiesCardKeyType),
            loaded: theme.color(ColorRole::IdentitiesCardLoaded),
            missing: theme.color(ColorRole::IdentitiesCardMissing),
            credential: theme.color(ColorRole::IdentitiesCardCredential),
        }
    }

    /// A card's own style, backed by the selection when the card is selected.
    ///
    /// Only the background travels: the fg of every role stays its own, which
    /// is what keeps the name, the metadata and the status dot distinguishable
    /// inside a highlighted card.
    fn on_card(self, style: Style, selected: bool) -> Style {
        match (selected, self.selection.bg) {
            (true, Some(bg)) => style.bg(bg),
            _ => style,
        }
    }
}

pub fn render_keys(frame: &mut Frame, area: Rect, app: &App) {
    if area.height < 4 || area.width < 20 {
        return;
    }

    let theme = app.theme();
    let buf = frame.buffer_mut();
    let margin = if area.width >= 132 {
        2
    } else if area.width >= 80 {
        1
    } else {
        0
    };
    let inner_x = area.x + margin;
    let inner_w = area.width.saturating_sub(margin * 2);

    let agent = app.agent_info.as_ref();

    // The agent block is a fixed strip at the bottom, not a floater that follows
    // the last card: deriving its position from the grid put it straight onto a
    // half-scrolled card row, and its own fields are shorter than the row, so the
    // card showed through as borders and key paths spliced onto the panel's text.
    //
    // Rows, bottom up: notice, keys, socket, rule.
    const AGENT_STRIP: u16 = 4;
    let stride = row_stride_const();
    let available = area.height.saturating_sub(AGENT_STRIP);
    // Whole card rows only. A grid cut mid-card left a sliver of the next row
    // above the rule, and since the grid scrolls by lines that sliver moved while
    // everything around it stayed put. The rule then sits directly under the last
    // row, so there is no drifting gap between them either.
    let rows_that_fit = available / stride;
    let (grid, agent_y) = if rows_that_fit > 0 {
        let h = rows_that_fit * stride;
        (Rect::new(area.x, area.y, area.width, h), Some(area.y + h))
    } else {
        (area, None)
    };

    // Cards per row — user preference (0 = auto), clamped to what fits.
    let cards_per_row = resolve_columns(inner_w, app.config.appearance.identity_columns);
    let cpr_u16 = cards_per_row as u16;
    let card_w = (inner_w.saturating_sub((cpr_u16 - 1) * CARD_GAP)) / cpr_u16;

    // Cards are laid out in rows of `cards_per_row`. Once there are more rows
    // than fit, scroll by whole card-rows to keep the selected card on screen
    // (roughly centered).
    let row_stride = row_stride_const();
    let cpr = cards_per_row.max(1);
    let row_offset = app.keys_scroll_row_offset(grid.height, cpr, row_stride);

    // Scroll by lines rather than whole card rows (#35): the grid slides under
    // the selection instead of jumping a card height at a time. Cards are drawn
    // into a taller scratch buffer, padded by one row above and below, so a card
    // half-way off either edge is clipped by the blit rather than spilling over
    // the panels around it.
    let pad = row_stride;
    let scroll = app.keys_scroll_advance(row_offset * row_stride as usize);
    let ext = Rect::new(grid.x, grid.y, grid.width, grid.height + pad * 2);
    let mut layer = Buffer::empty(ext);
    for (i, identity) in app.identities.iter().enumerate() {
        let row = i / cpr;
        // Draw a card once any part of it is at or below the top of the view,
        // and stop once one starts past the bottom. With `pad` == one card row
        // of slack on each side, the first drawn card can be at most `pad`
        // above the view, which the padded buffer has room for.
        let top = (row as u16) * row_stride;
        if top + row_stride < scroll || top >= scroll + grid.height {
            continue;
        }

        let col = i % cpr;
        let card_x = inner_x + (col as u16) * (card_w + CARD_GAP);
        let y = grid.y + pad + top - scroll;

        let is_selected = i == app.identity_selected;
        let card = Rect::new(card_x, y, card_w, CARD_H);
        render_card(
            &mut layer,
            card,
            identity,
            is_selected,
            agent,
            CardStyles::of(theme, card),
        );
        // The card drew its frame in the solid fallback of whichever of the two
        // border roles its selection state calls for; this is the gradient half
        // of that, over the same rect and the same role.
        crate::tui::blit::paint_border(
            &mut layer,
            card,
            theme,
            if is_selected {
                PaintRole::IdentitiesCardBorderSelected
            } else {
                PaintRole::IdentitiesCardBorder
            },
        );
    }
    crate::tui::blit::blit(buf, ext, grid, &layer, 0, -(pad as i32));

    // After the card layer, not before it: the blit copies every cell of the
    // scratch buffer over `grid`, so an empty-state message written first was
    // erased again and never reached the screen.
    let empty = app.identities.is_empty();
    if empty {
        let msg = "No identities — press 'a' (key or user+password)";
        let x = inner_x + (inner_w.saturating_sub(msg.chars().count() as u16)) / 2;
        buf.set_string(
            x,
            area.y + EMPTY_ROW,
            msg,
            theme.style(StyleRole::IdentitiesEmpty),
        );
    }

    if let Some(y) = agent_y {
        render_agent_info(buf, inner_x, y, inner_w, agent, theme);
    }

    // Notice, on the last row so it cannot land on the agent strip.
    if let Some(notice) = app.identity_notice.as_deref() {
        let notice_y = area.y + area.height.saturating_sub(1);
        buf.set_string(
            inner_x,
            notice_y,
            truncate(notice, inner_w as usize),
            theme.style(StyleRole::IdentitiesNotice),
        );
    }
}

/// Draw one identity card at `card`. Its height is always [`CARD_H`]; only the
/// width varies with the column count.
fn render_card(
    buf: &mut Buffer,
    card: Rect,
    identity: &Identity,
    selected: bool,
    agent: Option<&AgentInfo>,
    styles: CardStyles,
) {
    let (x, y, w) = (card.x, card.y, card.width);
    let border_style = Style::default().fg(if selected {
        styles.border_selected
    } else {
        styles.border
    });

    // Top border
    let top = format!("┌{}┐", "─".repeat((w - 2) as usize));
    buf.set_string(x, y, &top, border_style);

    // Bottom border
    let bottom = format!("└{}┘", "─".repeat((w - 2) as usize));
    buf.set_string(x, y + CARD_H - 1, &bottom, border_style);

    // Side borders
    for row in 1..CARD_H - 1 {
        buf.set_string(x, y + row, "│", border_style);
        buf.set_string(x + w - 1, y + row, "│", border_style);
        // Clear interior
        for cx in x + 1..x + w - 1 {
            if let Some(cell) = buf.cell_mut((cx, y + row)) {
                cell.set_symbol(" ");
                if selected {
                    cell.set_style(styles.selection);
                }
            }
        }
    }

    let inner_x = x + 2;
    let inner_w = w.saturating_sub(4);
    let text_style = styles.on_card(styles.text, selected);

    // Row 1: Name + key type
    let name_style = styles.on_card(styles.name, selected);
    buf.set_string(
        inner_x,
        y + 1,
        truncate(&identity.name, inner_w as usize / 2),
        name_style,
    );

    let key_type = detect_key_type(identity);
    let type_x = x + w - 2 - key_type.len() as u16;
    let type_style = styles.on_card(styles.key_type, selected);
    buf.set_string(type_x, y + 1, &key_type, type_style);

    // Row 2: Username + fingerprint
    let username = identity.username.as_deref().unwrap_or("-");
    buf.set_string(
        inner_x,
        y + 2,
        truncate(username, inner_w as usize / 2),
        text_style,
    );

    if let Some(fp) = find_fingerprint(identity, agent) {
        let fp_x = inner_x + (inner_w / 2);
        let fp_style = styles.on_card(styles.metadata, selected);
        buf.set_string(fp_x, y + 2, truncate(&fp, (inner_w / 2) as usize), fp_style);
    }

    // Row 3: Key path (or a note for a keyless password credential)
    let path_str = identity
        .private_key
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "password login (no key)".into());
    let path_style = styles.on_card(styles.metadata, selected);
    buf.set_string(
        inner_x,
        y + 3,
        truncate(&path_str, inner_w as usize),
        path_style,
    );

    // Row 4: for a key, show agent load status (+ passphrase indicator);
    // for a keyless password credential, just the password status.
    if identity.private_key.is_some() {
        let loaded = is_loaded_in_agent(identity, agent);
        let (dot, dot_color, label) = if loaded {
            ("●", styles.loaded, " loaded")
        } else {
            ("○", styles.missing, " not loaded")
        };
        let status_style = styles.on_card(Style::default().fg(dot_color), selected);
        buf.set_string(inner_x, y + 4, dot, status_style);
        buf.set_string(inner_x + 1, y + 4, label, status_style);

        if identity.has_password {
            // Passphrase indicator, placed after the status label (whose width
            // varies: " loaded" vs " not loaded") with a 2-col gap, if it fits.
            let pw_x = inner_x + 1 + label.chars().count() as u16 + 2;
            let pw_text = "● passphrase";
            if pw_x + pw_text.chars().count() as u16 <= inner_x + inner_w {
                let pw_style = styles.on_card(Style::default().fg(styles.credential), selected);
                buf.set_string(pw_x, y + 4, pw_text, pw_style);
            }
        }
    } else {
        let (dot, color, text) = if identity.has_password {
            ("●", styles.credential, " password set")
        } else {
            ("○", styles.missing, " no password")
        };
        let status_style = styles.on_card(Style::default().fg(color), selected);
        buf.set_string(inner_x, y + 4, dot, status_style);
        buf.set_string(inner_x + 1, y + 4, text, status_style);
    }
}

fn render_agent_info(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    w: u16,
    agent: Option<&AgentInfo>,
    theme: &ResolvedTheme,
) {
    // These rows sit below the visible card grid; theme the separator and the
    // agent details independently so no legacy palette colour leaks through.
    let rule = Rect::new(x, y, w, 1);
    let line: String = std::iter::repeat_n('─', w as usize).collect();
    buf.set_string(
        x,
        y,
        &line,
        Style::default().fg(crate::tui::blit::line_color(
            theme,
            PaintRole::IdentitiesAgentSeparator,
            rule,
        )),
    );
    // The identities tab is a dashboard body and never floats over a session,
    // so a gradient separator may be run over the glyphs just written.
    crate::tui::blit::paint_line(buf, rule, theme, PaintRole::IdentitiesAgentSeparator);

    let label = theme.style(StyleRole::IdentitiesAgentLabel);
    let info_y = y + 1;
    match agent {
        Some(info) => {
            let socket = info.socket_path.as_deref().unwrap_or("(not set)");
            buf.set_string(x, info_y, "agent socket  ", label);
            buf.set_string(
                x + 14,
                info_y,
                truncate(socket, (w - 14) as usize),
                theme.style(StyleRole::IdentitiesAgentValue),
            );

            let key_count = info.keys.len();
            buf.set_string(x, info_y + 1, "loaded keys   ", label);
            buf.set_string(
                x + 14,
                info_y + 1,
                key_count.to_string(),
                theme.style(StyleRole::IdentitiesAgentCount),
            );
        }
        None => {
            buf.set_string(
                x,
                info_y,
                "SSH agent not detected",
                theme.style(StyleRole::IdentitiesEmpty),
            );
        }
    }
}

fn detect_key_type(identity: &Identity) -> String {
    let path = identity
        .private_key
        .as_ref()
        .map(|p| p.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if identity.private_key.is_none() {
        "password".into()
    } else if path.contains("ed25519") {
        "ed25519".into()
    } else if path.contains("ecdsa") {
        "ecdsa".into()
    } else if path.contains("rsa") {
        "rsa".into()
    } else if path.contains("dsa") {
        "dsa".into()
    } else {
        "key".into()
    }
}

fn find_fingerprint(identity: &Identity, agent: Option<&AgentInfo>) -> Option<String> {
    let agent = agent?;
    let key_path = identity.private_key.as_ref()?.to_string_lossy();
    agent
        .keys
        .iter()
        .find(|k| k.comment.contains(key_path.as_ref()))
        .map(|k| k.fingerprint.clone())
}

fn is_loaded_in_agent(identity: &Identity, agent: Option<&AgentInfo>) -> bool {
    let Some(agent) = agent else { return false };
    let Some(ref key_path) = identity.private_key else {
        return false;
    };
    let path_str = key_path.to_string_lossy();
    agent
        .keys
        .iter()
        .any(|k| k.comment.contains(path_str.as_ref()))
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let end = s
            .char_indices()
            .take(max)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use std::path::PathBuf;

    fn row_text(buf: &Buffer, y: u16, w: u16) -> String {
        (0..w)
            .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
            .collect()
    }

    /// Card roles resolved from `default` — the geometry tests below care only
    /// about where glyphs land, not which colour they wear.
    fn test_styles() -> CardStyles {
        CardStyles::of(
            &crate::test_support::resolved_default(),
            Rect::new(0, 0, CARD_W, CARD_H),
        )
    }

    fn identity(private_key: Option<&str>, has_password: bool) -> Identity {
        Identity {
            id: 1,
            name: "selectel-core".into(),
            username: Some("root".into()),
            private_key: private_key.map(PathBuf::from),
            certificate: None,
            has_password,
        }
    }

    /// Each identity card runs the gradient of the border role its *own*
    /// selection state names.
    ///
    /// Two roles, two cards, one render: the selected card must gradient
    /// `card.border_selected` and the unselected one `card.border`. Giving the
    /// two gradients disjoint colours is what proves a card cannot be painted
    /// with its neighbour's role.
    #[test]
    fn identity_cards_gradient_the_border_role_their_selection_state_names() {
        use crate::test_support::{frame_at, resolved_source, themed_app};

        // Two closed rings, deliberately disjoint: reds for the idle border,
        // greens for the selected one.
        let theme = resolved_source(
            "cards",
            "schema_version = 1\nname = \"Cards\"\nextends = \"default\"\n\n\
             [gradients.idle]\ndirection = \"perimeter\"\n\
             stops = [ { at = 0.0, color = \"#400000\" }, { at = 0.5, color = \"#ff0000\" }, \
             { at = 1.0, color = \"#400000\" } ]\n\
             [gradients.sel]\ndirection = \"perimeter\"\n\
             stops = [ { at = 0.0, color = \"#004000\" }, { at = 0.5, color = \"#00ff00\" }, \
             { at = 1.0, color = \"#004000\" } ]\n\n\
             [components.identities.card]\n\
             border = { gradient = \"gradients.idle\" }\n\
             border_selected = { gradient = \"gradients.sel\" }\n",
        );

        let mut app = themed_app(theme);
        app.identities = vec![
            Identity {
                id: 1,
                name: "first".into(),
                username: None,
                private_key: Some(PathBuf::from("/home/u/.ssh/first")),
                certificate: None,
                has_password: false,
            },
            Identity {
                id: 2,
                name: "second".into(),
                username: None,
                private_key: Some(PathBuf::from("/home/u/.ssh/second")),
                certificate: None,
                has_password: false,
            },
        ];
        app.identity_selected = 0;

        let area = Rect::new(0, 0, 120, 30);
        let buf = frame_at(area, |f| render_keys(f, area, &app));

        // A card's frame is the only thing drawn in these colours, so a
        // channel-dominant cell identifies which role reached the buffer.
        // Collecting the *distinct* values per family is what separates a real
        // gradient from a flattened one: a renderer that kept sampling each
        // role at a single `line_color` would still put red on the idle card
        // and green on the selected one — both rings start on their own hue —
        // but each family would contribute exactly one colour.
        let mut idle: Vec<(u8, u8, u8)> = Vec::new();
        let mut selected: Vec<(u8, u8, u8)> = Vec::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let ratatui::style::Color::Rgb(r, g, b) = buf.cell((x, y)).unwrap().fg {
                    let family = if b == 0 && r > 0 && g == 0 {
                        Some(&mut idle)
                    } else if b == 0 && g > 0 && r == 0 {
                        Some(&mut selected)
                    } else {
                        None
                    };
                    if let Some(seen) = family {
                        if !seen.contains(&(r, g, b)) {
                            seen.push((r, g, b));
                        }
                    }
                }
            }
        }

        // Both roles reached the frame at all…
        assert!(
            !selected.is_empty(),
            "the selected card did not use border_selected"
        );
        assert!(!idle.is_empty(), "the unselected card did not use border");
        // …and each one swept rather than flattening to its first stop.
        assert!(
            idle.len() >= 2,
            "`card.border` flattened to a single colour: {idle:?}"
        );
        assert!(
            selected.len() >= 2,
            "`card.border_selected` flattened to a single colour: {selected:?}"
        );
    }

    #[test]
    fn resolve_columns_auto_and_manual() {
        // Auto (pref 0): 2 when two full cards fit, else 1.
        assert_eq!(resolve_columns(CARD_W * 2 + CARD_GAP, 0), 2);
        assert_eq!(resolve_columns(CARD_W, 0), 1);
        // Manual pref clamps to what fits.
        assert_eq!(resolve_columns(200, 3), 3);
        assert_eq!(resolve_columns(60, 4), max_columns(60));
        assert!(resolve_columns(60, 4) >= 1);
        // pref of 1 always honoured.
        assert_eq!(resolve_columns(500, 1), 1);
    }

    #[test]
    fn key_card_status_row_does_not_overlap() {
        let mut buf = Buffer::empty(Rect::new(0, 0, CARD_W, CARD_H));
        let id = identity(Some("/home/u/.ssh/sshub_selectel-core"), true);
        render_card(
            &mut buf,
            Rect::new(0, 0, CARD_W, CARD_H),
            &id,
            false,
            None,
            test_styles(),
        );

        let row = row_text(&buf, 4, CARD_W);
        // Both labels present, and "passphrase" isn't glued onto "loaded".
        assert!(row.contains("not loaded"), "row: {row:?}");
        assert!(row.contains("passphrase"), "row: {row:?}");
        assert!(
            !row.contains("loaded● passphrase") && !row.contains("loaded●passphrase"),
            "labels overlap: {row:?}"
        );
        assert!(
            row.contains("loaded  ● passphrase") || row.contains("loaded ● passphrase"),
            "expected a gap before the passphrase marker: {row:?}"
        );
    }

    #[test]
    fn keyless_card_shows_password_credential() {
        let mut buf = Buffer::empty(Rect::new(0, 0, CARD_W, CARD_H));
        let id = identity(None, true);
        render_card(
            &mut buf,
            Rect::new(0, 0, CARD_W, CARD_H),
            &id,
            false,
            None,
            test_styles(),
        );

        assert!(
            row_text(&buf, 1, CARD_W).contains("password"),
            "badge missing"
        );
        assert!(
            row_text(&buf, 3, CARD_W).contains("no key"),
            "row3: expected keyless note"
        );
        assert!(
            row_text(&buf, 4, CARD_W).contains("password set"),
            "row4: expected password status"
        );
    }
}
