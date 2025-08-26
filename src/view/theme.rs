use std::sync::Arc;

use ratatui::style::{Color, Modifier, Style};

pub type SharedTheme = Arc<Theme>;

#[derive(Debug, Clone)]
pub struct Theme {
    fg: Color,
    label_fg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            fg: Color::Reset,
            label_fg: Color::DarkGray,
        }
    }
}

impl Theme {
    pub fn fg(&self) -> Color {
        self.fg
    }

    pub fn title(&self, focused: bool) -> Style {
        if focused {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        }
    }

    #[allow(unused_variables)]
    pub fn value(&self, focused: bool) -> Style {
        Style::default()
    }

    #[allow(unused_variables)]
    pub fn key(&self, focused: bool) -> Style {
        Style::default().fg(self.label_fg)
    }
}
