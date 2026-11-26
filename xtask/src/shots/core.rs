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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffResult {
    SizeMismatch {
        baseline: (u32, u32),
        current: (u32, u32),
    },
    Compared {
        changed: usize,
        total: usize,
        diff: Rgba,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channels {
    Gray,
    GrayAlpha,
    Rgb,
    Rgba,
}

const DIM_DIVISOR: u8 = 4;
const CHANGED_PIXEL: [u8; 4] = [255, 0, 0, 255];

fn pixel_changed(a: &[u8], b: &[u8], tolerance: u8) -> bool {
    a.iter().zip(b).any(|(x, y)| x.abs_diff(*y) > tolerance)
}

fn dimmed(pixel: &[u8]) -> [u8; 4] {
    [
        pixel[0] / DIM_DIVISOR,
        pixel[1] / DIM_DIVISOR,
        pixel[2] / DIM_DIVISOR,
        255,
    ]
}

pub fn diff_images(a: &Rgba, b: &Rgba, tolerance: u8) -> DiffResult {
    if (a.width, a.height) != (b.width, b.height) {
        return DiffResult::SizeMismatch {
            baseline: (a.width, a.height),
            current: (b.width, b.height),
        };
    }
    let mut changed = 0;
    let mut pixels = Vec::with_capacity(a.pixels.len());
    for (left, right) in a.pixels.chunks_exact(4).zip(b.pixels.chunks_exact(4)) {
        if pixel_changed(left, right, tolerance) {
            changed += 1;
            pixels.extend_from_slice(&CHANGED_PIXEL);
        } else {
            pixels.extend_from_slice(&dimmed(left));
        }
    }
    DiffResult::Compared {
        changed,
        total: pixels.len() / 4,
        diff: Rgba {
            width: a.width,
            height: a.height,
            pixels,
        },
    }
}

pub fn changed_percent(changed: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    changed as f64 * 100.0 / total as f64
}

pub fn expand_to_rgba(channels: Channels, data: &[u8]) -> Vec<u8> {
    match channels {
        Channels::Rgba => data.to_vec(),
        Channels::Rgb => data
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        Channels::Gray => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        Channels::GrayAlpha => data
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
    }
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

    fn image(width: u32, height: u32, pixels: &[[u8; 4]]) -> Rgba {
        Rgba {
            width,
            height,
            pixels: pixels.iter().flatten().copied().collect(),
        }
    }

    fn compared(result: DiffResult) -> (usize, usize, Rgba) {
        match result {
            DiffResult::Compared {
                changed,
                total,
                diff,
            } => (changed, total, diff),
            other => panic!("expected Compared, got {other:?}"),
        }
    }

    #[test]
    fn equal_images_have_no_changed_pixels() {
        let a = image(2, 1, &[[1, 2, 3, 255], [4, 5, 6, 255]]);
        let (changed, total, _) = compared(diff_images(&a, &a.clone(), 0));
        assert_eq!((changed, total), (0, 2));
    }

    #[test]
    fn a_pixel_past_the_tolerance_is_changed() {
        let a = image(2, 1, &[[10, 10, 10, 255], [0, 0, 0, 255]]);
        let b = image(2, 1, &[[10, 10, 19, 255], [0, 0, 0, 255]]);
        let (changed, _, _) = compared(diff_images(&a, &b, 8));
        assert_eq!(changed, 1);
    }

    #[test]
    fn a_pixel_within_the_tolerance_is_unchanged() {
        let a = image(1, 1, &[[10, 10, 10, 255]]);
        let b = image(1, 1, &[[18, 2, 10, 255]]);
        let (changed, _, _) = compared(diff_images(&a, &b, 8));
        assert_eq!(changed, 0);
    }

    #[test]
    fn a_size_mismatch_is_reported_not_a_panic() {
        let a = image(1, 1, &[[0, 0, 0, 255]]);
        let b = image(2, 1, &[[0, 0, 0, 255], [0, 0, 0, 255]]);
        assert_eq!(
            diff_images(&a, &b, 8),
            DiffResult::SizeMismatch {
                baseline: (1, 1),
                current: (2, 1)
            }
        );
    }

    #[test]
    fn the_diff_image_marks_changed_pixels_red_and_dims_the_rest() {
        let a = image(2, 1, &[[200, 100, 40, 255], [0, 0, 0, 255]]);
        let b = image(2, 1, &[[200, 100, 40, 255], [255, 255, 255, 255]]);
        let (_, _, diff) = compared(diff_images(&a, &b, 8));
        assert_eq!(&diff.pixels[4..8], &[255, 0, 0, 255]);
        assert_eq!(&diff.pixels[0..4], &[50, 25, 10, 255]);
        assert_eq!((diff.width, diff.height), (2, 1));
    }

    #[test]
    fn the_changed_percent_is_a_share_of_the_total() {
        assert_eq!(changed_percent(1, 4), 25.0);
        assert_eq!(changed_percent(0, 4), 0.0);
        assert_eq!(changed_percent(0, 0), 0.0);
    }

    #[test]
    fn channels_expand_to_rgba() {
        assert_eq!(
            expand_to_rgba(Channels::Gray, &[7, 9]),
            [7, 7, 7, 255, 9, 9, 9, 255]
        );
        assert_eq!(expand_to_rgba(Channels::GrayAlpha, &[7, 3]), [7, 7, 7, 3]);
        assert_eq!(expand_to_rgba(Channels::Rgb, &[1, 2, 3]), [1, 2, 3, 255]);
        assert_eq!(expand_to_rgba(Channels::Rgba, &[1, 2, 3, 4]), [1, 2, 3, 4]);
    }
}
