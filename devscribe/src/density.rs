//! The "density" setting: scales the shell chrome's row heights (title bar,
//! tab bar, status bar, sidebar rows). Deliberately scoped to chrome only —
//! the code editor's own text size is a separate "zoom" concern, not this
//! setting's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    Spacious,
    #[default]
    Comfortable,
    Compact,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Spacious, Density::Comfortable, Density::Compact];

    pub fn label(self) -> &'static str {
        match self {
            Density::Spacious => "Spacious",
            Density::Comfortable => "Comfortable",
            Density::Compact => "Compact",
        }
    }

    pub fn title_bar_h(self) -> f32 {
        match self {
            Density::Spacious => 46.0,
            Density::Comfortable => 38.0,
            Density::Compact => 32.0,
        }
    }

    pub fn tab_bar_h(self) -> f32 {
        match self {
            Density::Spacious => 46.0,
            Density::Comfortable => 38.0,
            Density::Compact => 32.0,
        }
    }

    pub fn status_bar_h(self) -> f32 {
        match self {
            Density::Spacious => 34.0,
            Density::Comfortable => 28.0,
            Density::Compact => 24.0,
        }
    }

    pub fn sidebar_row_h(self) -> f32 {
        match self {
            Density::Spacious => 34.0,
            Density::Comfortable => 28.0,
            Density::Compact => 22.0,
        }
    }
}
