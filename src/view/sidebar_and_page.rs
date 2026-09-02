use chrono::{DateTime, Local};
use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    Frame,
};

use crate::{component::stateful_lines::StatefulGroupedLines, resource::ResourceType};

use super::{BlockArg, DetailArg, Navigator, NavigatorArgs};

const PROCESS_LIST_MIN_WIDTH: u16 = 24;
const PROCESS_DETAIL_MIN_WIDTH: u16 = 32;
const PROCESS_DETAIL_MAX_WIDTH: u16 = 64;
const PROCESS_PANE_GAP: u16 = 1;

#[derive(Debug, Default)]
pub struct SidebarAndPage {
    pub sidebar: Rect,
    pub sidebar_state: StatefulGroupedLines<'static>,
    pub page: Rect,
    process_list: Rect,
    process_detail: Option<Rect>,
    pub page_focused: bool,
}

impl SidebarAndPage {
    fn focused_resource_index(&self) -> Option<usize> {
        self.sidebar_state.focused_index().or(Some(0))
    }

    fn render_top(&mut self, frame: &mut Frame, top: Rect) {
        let current_local: DateTime<Local> = Local::now();
        let time = current_local.format("%Y-%m-%d %H:%M:%S");

        let mut spans = vec![];
        if top.width > 30 {
            spans.push(Span::styled("  ", Style::new()));
        }
        if top.width > 28 {
            spans.push(Span::styled("ResTop", Style::new().bold().italic()));
        }
        if top.width > 22 {
            spans.push(Span::styled(" * ", Style::new()));
        }
        spans.push(Span::styled(time.to_string(), Style::new()));

        let header = Line::from(spans);

        frame.render_widget(header, top);
    }

    fn split_process_panes(rect: Rect) -> (Rect, Rect) {
        let required_width = PROCESS_LIST_MIN_WIDTH
            .saturating_add(PROCESS_DETAIL_MIN_WIDTH)
            .saturating_add(PROCESS_PANE_GAP);
        if rect.width < required_width {
            return (Rect { width: 0, ..rect }, rect);
        }

        let max_detail_width = rect
            .width
            .saturating_sub(PROCESS_LIST_MIN_WIDTH.saturating_add(PROCESS_PANE_GAP));
        let detail_width = (rect.width / 2)
            .clamp(PROCESS_DETAIL_MIN_WIDTH, PROCESS_DETAIL_MAX_WIDTH)
            .min(max_detail_width);
        let list_width = rect
            .width
            .saturating_sub(detail_width.saturating_add(PROCESS_PANE_GAP));
        let list = Rect {
            width: list_width,
            ..rect
        };
        let detail = Rect {
            x: rect
                .x
                .saturating_add(list_width)
                .saturating_add(PROCESS_PANE_GAP),
            width: detail_width,
            ..rect
        };
        (list, detail)
    }

    fn update_layout_with_sidebar(&mut self, rect: Rect, hide_sidebar: bool) {
        if hide_sidebar {
            self.sidebar = Rect { width: 0, ..rect };
            self.page = rect;
            let content = Rect {
                y: rect.y.saturating_add(1),
                height: rect.height.saturating_sub(1),
                ..rect
            };
            let (process_list, process_detail) = Self::split_process_panes(content);
            self.process_list = process_list;
            self.process_detail = Some(process_detail);
            return;
        }

        let lr = Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(2),
                Constraint::Fill(3),
            ])
            .split(rect);

        self.sidebar = lr[0];
        self.page = lr[2];
        self.process_list = self.page;
        self.process_detail = None;
    }

    fn overview(&mut self, frame: &mut Frame, resources: &mut [ResourceType]) {
        let rect = self.sidebar;
        let top = Rect {
            y: rect.y,
            height: 1,
            ..rect
        };

        self.render_top(frame, top);

        let rect = Rect {
            y: rect.y + 1,
            height: rect.height - 1,
            ..rect
        };

        let mut args = BlockArg {
            width: rect.width,
            focused: !self.page_focused,
        };
        let mut overviews = vec![];
        for ele in resources.iter() {
            if let Ok(ov) = ele.block(&mut args) {
                overviews.push(ov)
            }
        }
        self.sidebar_state.update_blocks(overviews);
        self.sidebar_state.render(frame, rect, !self.page_focused);
    }
}

impl Navigator for SidebarAndPage {
    fn update_layout(&mut self, rect: Rect) {
        self.update_layout_with_sidebar(rect, false);
    }

    fn focus_left(&mut self) {
        if self.page_focused {
            self.page_focused = false;
        }
    }

    fn focus_right(&mut self) {
        if !self.page_focused {
            self.page_focused = true;
        }
    }

    fn focus_up(&mut self, resources: &mut Vec<crate::resource::ResourceType>) {
        if self.page_focused {
            if let Some(rt) = self
                .sidebar_state
                .focused_index()
                .or(Some(0))
                .and_then(|id| resources.get_mut(id))
            {
                rt.cached_page_state().focus_prev();
            }
        } else {
            self.sidebar_state.focus_prev()
        }
    }

    fn focus_down(&mut self, resources: &mut Vec<crate::resource::ResourceType>) {
        if self.page_focused {
            if let Some(rt) = self
                .sidebar_state
                .focused_index()
                .or(Some(0))
                .and_then(|id| resources.get_mut(id))
            {
                rt.cached_page_state().focus_next();
            }
        } else {
            self.sidebar_state.focus_next()
        }
    }

    fn render(
        &mut self,
        frame: &mut Frame,
        resources: &mut Vec<crate::resource::ResourceType>,
        _: Option<usize>,
    ) {
        let sidebar_hidden = self
            .focused_resource_index()
            .and_then(|id| resources.get(id))
            .map(|resource| resource.hides_sidebar())
            .unwrap_or(false);
        self.update_layout_with_sidebar(frame.area(), sidebar_hidden);

        if !sidebar_hidden {
            self.overview(frame, resources);
        }

        if let Some(rt) = self
            .focused_resource_index()
            .and_then(|id| resources.get_mut(id))
        {
            if sidebar_hidden {
                rt.render_header(frame, self.page);
                rt.render_process_list(
                    frame,
                    &DetailArg {
                        rect: self.process_list,
                        active: self.page_focused,
                    },
                );
                if let Some(process_detail) = self.process_detail {
                    rt.render_process_detail(
                        frame,
                        &DetailArg {
                            rect: process_detail,
                            active: self.page_focused,
                        },
                    );
                }
            } else {
                let mut args = DetailArg {
                    rect: self.page,
                    active: self.page_focused,
                };
                rt.render_detail(frame, &mut args);
            }
        }
    }

    fn handle_event<'a>(&mut self, event: &super::NavigatorEvent, args: NavigatorArgs<'a>) {
        if self.page_focused {
            if let Some(rt) = self
                .sidebar_state
                .focused_index()
                .or(Some(0))
                .and_then(|id: usize| args.resources.get_mut(id))
            {
                let handled = rt.handle_navi_event(event);
                if handled {
                    return;
                }
            }
        }
        match event {
            super::NavigatorEvent::KeyEvent(key) => {
                match key.code {
                    KeyCode::Up => self.focus_up(args.resources),
                    KeyCode::Down => self.focus_down(args.resources),
                    KeyCode::Left => self.focus_left(),
                    KeyCode::Right => self.focus_right(),
                    _ => {}
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::SidebarAndPage;
    use crate::view::Navigator;

    #[test]
    fn hidden_sidebar_gives_page_the_full_terminal() {
        let mut layout = SidebarAndPage::default();
        let area = Rect::new(0, 0, 120, 40);

        layout.update_layout_with_sidebar(area, true);

        assert_eq!(layout.sidebar.width, 0);
        assert_eq!(layout.page, area);
        assert_eq!(layout.process_list.y, area.y + 1);
        assert!(layout.process_list.right() <= layout.process_detail.unwrap().x);
        assert!(layout.process_detail.unwrap().x > layout.process_list.x);
    }

    #[test]
    fn normal_layout_restores_sidebar_after_detail_closes() {
        let mut layout = SidebarAndPage::default();
        let area = Rect::new(0, 0, 120, 40);

        layout.update_layout_with_sidebar(area, true);
        layout.update_layout(area);

        assert!(layout.sidebar.width > 0);
        assert!(layout.page.width > 0);
        assert_eq!(layout.process_detail, None);
        assert_eq!(layout.process_list, layout.page);
        assert!(layout.sidebar.right() < layout.page.left());
    }

    #[test]
    fn narrow_terminal_keeps_detail_separate_when_list_cannot_fit() {
        let mut layout = SidebarAndPage::default();
        let area = Rect::new(0, 0, 40, 20);

        layout.update_layout_with_sidebar(area, true);

        assert_eq!(layout.process_list.width, 0);
        assert_eq!(layout.process_detail.unwrap().width, area.width);
    }
}
