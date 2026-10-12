pub mod audit;
pub mod auth;
pub mod broadcast;
pub mod field_picker;
pub mod group_form;
pub mod group_manage;
pub mod help;
pub mod host_form;
pub mod keybind_editor;
pub mod keychain;
pub mod keygen;
pub mod keys;
pub mod known_hosts;
pub mod log_browser;
pub mod palette;
pub mod push_key_pickers;
pub mod session_picker;
pub mod settings;
pub mod sftp;
pub mod snippet_form;
pub mod snippet_manage;
pub mod snippet_picker;
pub mod tag_filter;
pub mod theme_picker;
pub mod tunnel_reconnect;
pub mod tunnels;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph, Widget, Wrap};

/// A form popup's paragraph, built from lines that each say which field they
/// show (by position in the form's `ALL`), so a renderer hands its caller both
/// what to draw and where each field is.
pub struct FormBody {
    para: Paragraph<'static>,
}

impl FormBody {
    /// `block` must draw all four borders: the field rows start inside them.
    pub fn new(
        rows: Vec<(Line<'static>, Option<usize>)>,
        block: Block<'static>,
        wrap: Option<Wrap>,
    ) -> Self {
        let mut para =
            Paragraph::new(rows.into_iter().map(|(l, _)| l).collect::<Vec<_>>()).block(block);
        if let Some(wrap) = wrap {
            para = para.wrap(wrap);
        }
        Self { para }
    }
}

impl Widget for FormBody {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.para.render(area, buf);
    }
}
