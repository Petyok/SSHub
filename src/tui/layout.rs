use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Screen regions: sidebar, search bar, group tree, optional detail panel, status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootAreas {
    pub sidebar: Rect,
    pub search: Rect,
    pub group_tree: Rect,
    pub detail: Option<Rect>,
    pub status: Rect,
}

const SIDEBAR_WIDTH: u16 = 14;

pub fn root_layout(area: Rect, show_detail_panel: bool) -> RootAreas {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    let main_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(10)])
        .split(outer[0]);

    let content = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(main_row[1]);

    if show_detail_panel {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(content[1]);
        RootAreas {
            sidebar: main_row[0],
            search: content[0],
            group_tree: body[0],
            detail: Some(body[1]),
            status: outer[1],
        }
    } else {
        RootAreas {
            sidebar: main_row[0],
            search: content[0],
            group_tree: content[1],
            detail: None,
            status: outer[1],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn sidebar_gets_fixed_width() {
        let areas = root_layout(Rect::new(0, 0, 80, 24), true);
        assert_eq!(areas.sidebar.width, SIDEBAR_WIDTH);
    }

    #[test]
    fn detail_panel_gets_right_column_when_enabled() {
        let areas = root_layout(Rect::new(0, 0, 80, 24), true);
        assert!(areas.detail.is_some());
        assert!(areas.group_tree.width < areas.detail.unwrap().width);
    }

    #[test]
    fn group_tree_full_width_when_detail_hidden() {
        let areas = root_layout(Rect::new(0, 0, 80, 24), false);
        assert!(areas.detail.is_none());
        assert_eq!(areas.group_tree.width, 80 - SIDEBAR_WIDTH);
    }
}
