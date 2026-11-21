pub mod footer;
pub mod header;
pub mod hosts_panel;
pub mod middle_stack;
pub mod panel_box;
pub mod qr_frames;
pub mod right_stack;
pub mod sync_panel;
pub mod tab_bar;

/// One-row `/` filter bar for the Tunnels and Identities tabs, in the same
/// style as the SFTP pane's search bar: a block cursor and key hints while the
/// query is being typed, a quiet reminder once it is kept.
pub fn filter_bar(
    buf: &mut ratatui::buffer::Buffer,
    row: ratatui::layout::Rect,
    query: &str,
    editing: bool,
    theme: &crate::theme::model::ResolvedTheme,
) {
    let bar = theme.style(crate::theme::catalog::StyleRole::SftpSearch);
    for x in row.x..row.x + row.width {
        if let Some(cell) = buf.cell_mut((x, row.y)) {
            cell.set_style(bar);
            cell.set_symbol(" ");
        }
    }
    let text = if editing {
        format!(" filter: {query}\u{2581}   Esc clear · Enter keep")
    } else {
        format!(" filter: {query}   / edit · Esc clear")
    };
    buf.set_string(
        row.x,
        row.y,
        super::text::ellipsize(&text, row.width as usize),
        bar,
    );
}
