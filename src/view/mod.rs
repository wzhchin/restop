pub mod blocks;
pub mod sidebar_and_page;
pub mod theme;

use crossterm::event::KeyEvent;
use ratatui::{layout::Rect, Frame};
use sidebar_and_page::SidebarAndPage;

use crate::resource::ResourceType;

pub trait Navigator {
    fn update_layout(&mut self, rect: Rect);

    fn render(
        &mut self,
        frame: &mut Frame,
        resources: &mut Vec<ResourceType>,
        focused: Option<usize>,
    );

    fn handle_event<'a>(&mut self, event: &NavigatorEvent, args: NavigatorArgs<'a>);

    fn focus_left(&mut self) {}
    fn focus_right(&mut self) {}
    fn focus_up(&mut self, _resources: &mut Vec<ResourceType>) {}
    fn focus_down(&mut self, _resources: &mut Vec<ResourceType>) {}
}

#[derive(Debug)]
pub enum LayoutType {
    SidebarAndPage(SidebarAndPage),
}

impl Navigator for LayoutType {
    fn focus_left(&mut self) {
        match self {
            LayoutType::SidebarAndPage(a) => a.focus_left(),
        }
    }

    fn focus_right(&mut self) {
        match self {
            LayoutType::SidebarAndPage(a) => a.focus_right(),
        }
    }

    fn focus_up(&mut self, resources: &mut Vec<ResourceType>) {
        match self {
            LayoutType::SidebarAndPage(a) => a.focus_up(resources),
        }
    }

    fn focus_down(&mut self, resources: &mut Vec<ResourceType>) {
        match self {
            LayoutType::SidebarAndPage(a) => a.focus_down(resources),
        }
    }

    fn update_layout(&mut self, rect: Rect) {
        match self {
            LayoutType::SidebarAndPage(a) => a.update_layout(rect),
        }
    }

    fn render(
        &mut self,
        frame: &mut Frame,
        resources: &mut Vec<ResourceType>,
        focused: Option<usize>,
    ) {
        match self {
            LayoutType::SidebarAndPage(a) => a.render(frame, resources, focused),
        }
    }

    fn handle_event<'a>(&mut self, event: &NavigatorEvent, args: NavigatorArgs<'a>) {
        match self {
            LayoutType::SidebarAndPage(sp) => sp.handle_event(event, args),
        }
    }
}

pub struct OverviewArg {
    pub width: u16,
    pub focused: bool,
}

#[derive(Clone)]
pub struct PageArg {
    pub rect: Rect,
    pub active: bool,
}

pub enum NavigatorEvent {
    KeyEvent(KeyEvent),
}

pub struct NavigatorArgs<'a> {
    pub resources: &'a mut Vec<ResourceType>,
}
