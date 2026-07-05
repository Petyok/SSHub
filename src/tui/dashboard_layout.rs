//! Dashboard bento-grid layout.
//!
//! Target: 132 cols × 38 rows. Scales gracefully below.
//!
//! ```text
//! Rows 1-3:   Header (wordmark + stats + clock)
//! Row  4:     Horizontal rule (dim)
//! Row  5:     Tab bar
//! Row  6:     Horizontal rule (dim)
//! Rows 7-N-2: Body — 3-column bento grid
//! Row  N-1:   Bold rule (dim)
//! Row  N:     Footer keybind bar
//! ```

use ratatui::layout::Rect;

/// All screen regions for the dashboard view.
#[derive(Debug, Clone, Copy)]
pub struct DashboardAreas {
    /// Full terminal area.
    pub full: Rect,
    /// Header: rows 0..3 (3 lines for wordmark + stats).
    pub header: Rect,
    /// Tab bar: 1 row.
    pub tab_bar: Rect,
    /// Body: the main content area between tab bar and footer.
    pub body: Rect,
    /// Left column inside body (hosts panel).
    pub col_left: Rect,
    /// Middle column inside body.
    pub col_mid: Rect,
    /// Right column inside body.
    pub col_right: Rect,
    /// Footer: last row.
    pub footer: Rect,
}

/// Compute dashboard areas from terminal size.
///
/// Layout adapts to terminal width:
/// - ≥132 cols: 3 columns (42+1+42+1+42 + 4 margin = 132)
/// - <132 cols: columns shrink proportionally, margins reduce
pub fn dashboard_layout(area: Rect) -> DashboardAreas {
    let w = area.width;
    let h = area.height;

    // Vertical bands
    let header_h = 3u16;
    let rule1 = 1u16;
    let tab_h = 1u16;
    let rule2 = 1u16;
    let footer_h = 1u16;
    let rule3 = 1u16;
    let chrome = header_h + rule1 + tab_h + rule2 + rule3 + footer_h; // 8 rows
    let body_h = h.saturating_sub(chrome);

    let header = Rect::new(area.x, area.y, w, header_h.min(h));
    // Clamp to a zero-height rect when the terminal is too short for the
    // chrome — renderers skip zero-height areas instead of panicking.
    let tab_y = area.y + header_h + rule1;
    let tab_bar = if tab_y + tab_h <= area.y + h {
        Rect::new(area.x, tab_y, w, tab_h)
    } else {
        Rect::new(area.x, area.y, w, 0)
    };
    let body_y = area.y + header_h + rule1 + tab_h + rule2;
    let body = Rect::new(area.x, body_y, w, body_h);
    let footer = Rect::new(area.x, area.y + h.saturating_sub(footer_h), w, footer_h);

    // Horizontal: 3 columns with gutters
    // Target: margin(2) + col(42) + gutter(1) + col(42) + gutter(1) + col(42) + margin(2) = 132
    let margin = if w >= 132 {
        2
    } else if w >= 80 {
        1
    } else {
        0
    };
    let inner_w = w.saturating_sub(margin * 2);
    let gutter = 1u16;
    let col_w = if inner_w >= 3 + 2 * gutter {
        (inner_w - 2 * gutter) / 3
    } else {
        inner_w / 3
    };

    let col_left = Rect::new(area.x + margin, body_y, col_w, body_h);
    let col_mid = Rect::new(col_left.x + col_w + gutter, body_y, col_w, body_h);
    let col_right = Rect::new(
        col_mid.x + col_w + gutter,
        body_y,
        // Take remaining space (handles rounding)
        w.saturating_sub(col_mid.x + col_w + gutter + margin - area.x),
        body_h,
    );

    DashboardAreas {
        full: area,
        header,
        tab_bar,
        body,
        col_left,
        col_mid,
        col_right,
        footer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_132x38() {
        let a = dashboard_layout(Rect::new(0, 0, 132, 38));
        assert_eq!(a.header.height, 3);
        assert_eq!(a.tab_bar.height, 1);
        assert_eq!(a.footer.height, 1);
        assert!(a.body.height >= 25);
        assert_eq!(a.col_left.width, 42);
        assert_eq!(a.col_mid.width, 42);
        // Right col takes remaining
        assert!(a.col_right.width >= 40);
    }

    #[test]
    fn narrow_80x24() {
        let a = dashboard_layout(Rect::new(0, 0, 80, 24));
        assert!(a.col_left.width > 0);
        assert!(a.col_mid.width > 0);
        assert!(a.col_right.width > 0);
        assert!(a.body.height > 0);
    }

    #[test]
    fn tiny_terminal() {
        let a = dashboard_layout(Rect::new(0, 0, 40, 10));
        assert!(a.col_left.width > 0);
        assert!(a.body.height > 0);
    }
}
