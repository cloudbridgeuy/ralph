use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ShotError {
    #[error("unknown screen `{0}`")]
    UnknownScreen(String),
    #[error("unknown theme `{0}`")]
    UnknownTheme(String),
    #[error("`{0}` is not among the current shots")]
    UnknownShot(String),
    #[error("name shots to approve or pass --all")]
    NothingSelected,
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

fn selected_names(selected: &[Shot]) -> BTreeSet<String> {
    selected.iter().map(Shot::name).collect()
}

pub fn stale_names(selected: &[Shot], existing: &[String]) -> Vec<String> {
    restrict_names(existing, selected)
}

pub fn stale_diffs(existing: &[String], changed: &[String]) -> Vec<String> {
    let kept = sorted_set(changed);
    existing
        .iter()
        .filter(|name| name.ends_with(".png") && !kept.contains(name))
        .cloned()
        .collect()
}

pub fn restrict_names(names: &[String], selected: &[Shot]) -> Vec<String> {
    let wanted = selected_names(selected);
    names
        .iter()
        .filter(|name| wanted.contains(*name))
        .cloned()
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

pub const CHANGED_THRESHOLD_PERCENT: f64 = 0.1;
pub const DEFAULT_TOLERANCE: u8 = 8;

#[derive(Debug, Clone, PartialEq)]
pub enum Measure {
    Pixels { changed: usize, total: usize },
    SizeMismatch,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Percent(f64),
    SizeMismatch,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Classification {
    pub changed: Vec<(String, Change)>,
    pub new: Vec<String>,
    pub missing: Vec<String>,
    pub unchanged: Vec<String>,
}

pub fn measure(result: &DiffResult) -> Measure {
    match result {
        DiffResult::SizeMismatch { .. } => Measure::SizeMismatch,
        DiffResult::Compared { changed, total, .. } => Measure::Pixels {
            changed: *changed,
            total: *total,
        },
    }
}

fn sorted_set(names: &[String]) -> BTreeSet<&String> {
    names.iter().collect()
}

pub fn common_names(baseline: &[String], current: &[String]) -> Vec<String> {
    let current = sorted_set(current);
    sorted_set(baseline)
        .into_iter()
        .filter(|name| current.contains(name))
        .cloned()
        .collect()
}

fn change_of(measure: &Measure) -> Option<Change> {
    match measure {
        Measure::SizeMismatch => Some(Change::SizeMismatch),
        Measure::Pixels { changed, total } => {
            let percent = changed_percent(*changed, *total);
            (percent > CHANGED_THRESHOLD_PERCENT).then_some(Change::Percent(percent))
        }
    }
}

pub fn classify(
    baseline: &[String],
    current: &[String],
    measures: &BTreeMap<String, Measure>,
) -> Classification {
    let baseline_set = sorted_set(baseline);
    let current_set = sorted_set(current);
    let mut classification = Classification {
        new: current_set
            .difference(&baseline_set)
            .map(|name| (*name).clone())
            .collect(),
        missing: baseline_set
            .difference(&current_set)
            .map(|name| (*name).clone())
            .collect(),
        ..Classification::default()
    };
    for name in baseline_set.intersection(&current_set) {
        let Some(measured) = measures.get(*name) else {
            continue;
        };
        match change_of(measured) {
            Some(change) => classification.changed.push(((*name).clone(), change)),
            None => classification.unchanged.push((*name).clone()),
        }
    }
    classification
}

pub fn is_clean(classification: &Classification) -> bool {
    classification.changed.is_empty()
        && classification.new.is_empty()
        && classification.missing.is_empty()
}

fn section(title: &str, lines: &[String]) -> String {
    let mut text = format!("## {title} ({})\n\n", lines.len());
    for line in lines {
        text.push_str(&format!("- {line}\n"));
    }
    if !lines.is_empty() {
        text.push('\n');
    }
    text
}

pub fn render_report(classification: &Classification) -> String {
    let changed: Vec<String> = classification
        .changed
        .iter()
        .map(|(name, change)| match change {
            Change::Percent(percent) => format!("{name}: {percent:.2}%"),
            Change::SizeMismatch => format!("{name}: size mismatch"),
        })
        .collect();
    format!(
        "# Shots report\n\n{}{}{}## Unchanged ({})\n",
        section("Changed", &changed),
        section("New", &classification.new),
        section("Missing", &classification.missing),
        classification.unchanged.len()
    )
}

pub fn select_approvals(
    names: &[String],
    all: bool,
    current_names: &[String],
) -> Result<Vec<String>, ShotError> {
    let current = sorted_set(current_names);
    if all {
        return Ok(current.into_iter().cloned().collect());
    }
    if names.is_empty() {
        return Err(ShotError::NothingSelected);
    }
    if let Some(unknown) = names.iter().find(|name| !current.contains(name)) {
        return Err(ShotError::UnknownShot(unknown.clone()));
    }
    Ok(sorted_set(names).into_iter().cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn diffs_of_shots_that_no_longer_changed_are_stale() {
        let existing = names(&["a.png", "b.png", "c.png"]);
        assert_eq!(
            stale_diffs(&existing, &names(&["b.png"])),
            names(&["a.png", "c.png"])
        );
    }

    #[test]
    fn no_diff_is_stale_when_every_one_still_changed() {
        let existing = names(&["a.png"]);
        assert!(stale_diffs(&existing, &names(&["a.png", "z.png"])).is_empty());
    }

    #[test]
    fn only_png_diffs_are_stale() {
        let existing = names(&["a.png", "notes.txt"]);
        assert_eq!(stale_diffs(&existing, &[]), names(&["a.png"]));
    }

    fn memory_shots() -> Vec<Shot> {
        filter(matrix(), Some(Screen::Memory), None)
    }

    #[test]
    fn a_memory_run_touches_only_memory_names() {
        let existing = names(&["memory-dark-1440.png", "threads-dark-1440.png"]);
        assert_eq!(
            stale_names(&memory_shots(), &existing),
            names(&["memory-dark-1440.png"])
        );
    }

    #[test]
    fn a_full_run_clears_every_matrix_name_and_keeps_others() {
        let existing = names(&["memory-dark-1440.png", "threads-light-390.png", "notes.png"]);
        assert_eq!(
            stale_names(&matrix(), &existing),
            names(&["memory-dark-1440.png", "threads-light-390.png"])
        );
    }

    #[test]
    fn restricted_sets_report_no_missing_for_other_screens() {
        let selected = memory_shots();
        let baseline = restrict_names(
            &names(&["memory-dark-1440.png", "threads-dark-1440.png"]),
            &selected,
        );
        let current = restrict_names(&names(&["memory-dark-1440.png"]), &selected);
        let measures = BTreeMap::from([(
            "memory-dark-1440.png".to_string(),
            Measure::Pixels {
                changed: 0,
                total: 10,
            },
        )]);
        let classification = classify(&baseline, &current, &measures);
        assert!(classification.missing.is_empty());
        assert!(is_clean(&classification));
    }

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

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn pixels(changed: usize, total: usize) -> Measure {
        Measure::Pixels { changed, total }
    }

    fn sample() -> Classification {
        let measures = BTreeMap::from([
            ("b.png".to_string(), pixels(5, 1000)),
            ("a.png".to_string(), pixels(1, 1000)),
            ("c.png".to_string(), pixels(0, 1000)),
            ("d.png".to_string(), Measure::SizeMismatch),
        ]);
        classify(
            &names(&["d.png", "c.png", "b.png", "a.png", "gone.png"]),
            &names(&["b.png", "new.png", "a.png", "c.png", "d.png"]),
            &measures,
        )
    }

    #[test]
    fn classification_fills_the_four_buckets() {
        let c = sample();
        assert_eq!(
            c.changed,
            vec![
                ("b.png".to_string(), Change::Percent(0.5)),
                ("d.png".to_string(), Change::SizeMismatch),
            ]
        );
        assert_eq!(c.new, names(&["new.png"]));
        assert_eq!(c.missing, names(&["gone.png"]));
        assert_eq!(c.unchanged, names(&["a.png", "c.png"]));
    }

    #[test]
    fn a_change_at_the_threshold_is_unchanged() {
        let measures = BTreeMap::from([("a.png".to_string(), pixels(1, 1000))]);
        let c = classify(&names(&["a.png"]), &names(&["a.png"]), &measures);
        assert!(c.changed.is_empty());
        assert_eq!(c.unchanged, names(&["a.png"]));
    }

    #[test]
    fn classification_order_is_sorted_whatever_the_input_order() {
        let c = classify(
            &names(&["z.png", "y.png"]),
            &names(&["m.png", "k.png"]),
            &BTreeMap::new(),
        );
        assert_eq!(c.missing, names(&["y.png", "z.png"]));
        assert_eq!(c.new, names(&["k.png", "m.png"]));
    }

    #[test]
    fn common_names_are_the_sorted_intersection() {
        assert_eq!(
            common_names(&names(&["c", "a", "b"]), &names(&["b", "c", "x"])),
            names(&["b", "c"])
        );
    }

    #[test]
    fn measuring_a_diff_result_drops_the_image() {
        let a = image(1, 1, &[[0, 0, 0, 255]]);
        let b = image(1, 1, &[[255, 0, 0, 255]]);
        assert_eq!(measure(&diff_images(&a, &b, 0)), pixels(1, 1));
        let wide = image(2, 1, &[[0, 0, 0, 255], [0, 0, 0, 255]]);
        assert_eq!(measure(&diff_images(&a, &wide, 0)), Measure::SizeMismatch);
    }

    #[test]
    fn only_a_run_with_nothing_changed_new_or_missing_is_clean() {
        let clean = Classification {
            unchanged: names(&["a.png"]),
            ..Classification::default()
        };
        assert!(is_clean(&clean));
        assert!(is_clean(&Classification::default()));
        assert!(!is_clean(&Classification {
            changed: vec![("a.png".to_string(), Change::SizeMismatch)],
            ..clean.clone()
        }));
        assert!(!is_clean(&Classification {
            new: names(&["n.png"]),
            ..clean.clone()
        }));
        assert!(!is_clean(&Classification {
            missing: names(&["m.png"]),
            ..clean
        }));
    }

    #[test]
    fn the_report_names_each_bucket() {
        let report = render_report(&sample());
        assert!(report.contains("## Changed (2)"));
        assert!(report.contains("- b.png: 0.50%"));
        assert!(report.contains("- d.png: size mismatch"));
        assert!(report.contains("## New (1)"));
        assert!(report.contains("- new.png"));
        assert!(report.contains("## Missing (1)"));
        assert!(report.contains("- gone.png"));
        assert!(report.contains("## Unchanged (2)"));
    }

    #[test]
    fn approvals_accept_all_named_and_reject_unknown() {
        let current = names(&["b.png", "a.png"]);
        assert_eq!(
            select_approvals(&[], true, &current),
            Ok(names(&["a.png", "b.png"]))
        );
        assert_eq!(
            select_approvals(&names(&["b.png", "b.png"]), false, &current),
            Ok(names(&["b.png"]))
        );
        assert_eq!(
            select_approvals(&names(&["c.png"]), false, &current),
            Err(ShotError::UnknownShot("c.png".to_string()))
        );
        assert_eq!(
            select_approvals(&[], false, &current),
            Err(ShotError::NothingSelected)
        );
    }
}
