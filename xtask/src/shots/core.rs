use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ShotError {
    #[error("unknown screen `{0}`")]
    UnknownScreen(String),
    #[error("unknown theme `{0}`")]
    UnknownTheme(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Memory,
    Threads,
    Workers,
    Schedules,
    Watches,
    Config,
    Catalogue,
}

impl Screen {
    pub const THEMED: [Screen; 6] = [
        Screen::Memory,
        Screen::Threads,
        Screen::Workers,
        Screen::Schedules,
        Screen::Watches,
        Screen::Config,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Screen::Memory => "memory",
            Screen::Threads => "threads",
            Screen::Workers => "workers",
            Screen::Schedules => "schedules",
            Screen::Watches => "watches",
            Screen::Config => "config",
            Screen::Catalogue => "catalogue",
        }
    }
}

impl FromStr for Screen {
    type Err = ShotError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Screen::THEMED
            .into_iter()
            .chain([Screen::Catalogue])
            .find(|screen| screen.name() == value)
            .ok_or_else(|| ShotError::UnknownScreen(value.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub const ALL: [Theme; 2] = [Theme::Dark, Theme::Light];

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }
}

impl FromStr for Theme {
    type Err = ShotError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Theme::ALL
            .into_iter()
            .find(|theme| theme.name() == value)
            .ok_or_else(|| ShotError::UnknownTheme(value.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Desktop,
    Mobile,
}

impl Width {
    pub const ALL: [Width; 2] = [Width::Desktop, Width::Mobile];

    pub fn pixels(self) -> u32 {
        match self {
            Width::Desktop => 1440,
            Width::Mobile => 390,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shot {
    pub screen: Screen,
    pub theme: Theme,
    pub width: Width,
}

impl Shot {
    pub fn name(&self) -> String {
        shot_name(self.screen, self.theme, self.width)
    }
}

pub fn shot_name(screen: Screen, theme: Theme, width: Width) -> String {
    match screen {
        Screen::Catalogue => format!("catalogue-{}.png", width.pixels()),
        _ => format!("{}-{}-{}.png", screen.name(), theme.name(), width.pixels()),
    }
}

pub fn matrix() -> Vec<Shot> {
    let themed = Screen::THEMED.into_iter().flat_map(|screen| {
        Theme::ALL.into_iter().flat_map(move |theme| {
            Width::ALL.into_iter().map(move |width| Shot {
                screen,
                theme,
                width,
            })
        })
    });
    let catalogue = Width::ALL.into_iter().map(|width| Shot {
        screen: Screen::Catalogue,
        theme: Theme::Dark,
        width,
    });
    themed.chain(catalogue).collect()
}

pub fn filter(shots: Vec<Shot>, screen: Option<Screen>, theme: Option<Theme>) -> Vec<Shot> {
    shots
        .into_iter()
        .filter(|shot| screen.is_none_or(|wanted| shot.screen == wanted))
        .filter(|shot| {
            shot.screen == Screen::Catalogue || theme.is_none_or(|wanted| shot.theme == wanted)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_themed_shot_name_has_screen_theme_and_width() {
        assert_eq!(
            shot_name(Screen::Memory, Theme::Dark, Width::Desktop),
            "memory-dark-1440.png"
        );
        assert_eq!(
            shot_name(Screen::Config, Theme::Light, Width::Mobile),
            "config-light-390.png"
        );
    }

    #[test]
    fn the_catalogue_name_has_no_theme() {
        assert_eq!(
            shot_name(Screen::Catalogue, Theme::Dark, Width::Desktop),
            "catalogue-1440.png"
        );
        assert_eq!(
            shot_name(Screen::Catalogue, Theme::Light, Width::Desktop),
            "catalogue-1440.png"
        );
    }

    #[test]
    fn an_unknown_screen_is_an_error() {
        assert_eq!(
            "nope".parse::<Screen>(),
            Err(ShotError::UnknownScreen("nope".to_string()))
        );
        assert_eq!("watches".parse::<Screen>(), Ok(Screen::Watches));
    }

    #[test]
    fn an_unknown_theme_is_an_error() {
        assert_eq!(
            "blue".parse::<Theme>(),
            Err(ShotError::UnknownTheme("blue".to_string()))
        );
        assert_eq!("light".parse::<Theme>(), Ok(Theme::Light));
    }

    #[test]
    fn the_matrix_has_26_unique_names() {
        let names: Vec<String> = matrix().iter().map(Shot::name).collect();
        assert_eq!(names.len(), 26);
        assert_eq!(names.iter().collect::<HashSet<_>>().len(), 26);
    }

    #[test]
    fn filtering_by_screen_and_theme_narrows_the_matrix() {
        let by_screen = filter(matrix(), Some(Screen::Memory), None);
        assert_eq!(by_screen.len(), 4);
        let by_theme = filter(matrix(), None, Some(Theme::Light));
        assert_eq!(by_theme.len(), 14);
        let both = filter(matrix(), Some(Screen::Catalogue), Some(Theme::Light));
        assert_eq!(both.len(), 2);
    }
}
