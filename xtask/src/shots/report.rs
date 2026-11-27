use super::core::{changed_percent, sorted_set, DiffResult, ShotError};
use std::collections::BTreeMap;

const CHANGED_THRESHOLD_PERCENT: f64 = 0.1;

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
    pub unreadable: Vec<String>,
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
            classification.unreadable.push((*name).clone());
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
        && classification.unreadable.is_empty()
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
        "# Shots report\n\n{}{}{}{}## Unchanged ({})\n",
        section("Changed", &changed),
        section("New", &classification.new),
        section("Missing", &classification.missing),
        section("Unreadable", &classification.unreadable),
        classification.unchanged.len()
    )
}

pub fn require_image_size(name: &str, width: u32, height: u32) -> Result<(), ShotError> {
    if width == 0 || height == 0 {
        return Err(ShotError::EmptyImage(name.to_string()));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub struct Approvals {
    pub approved: Vec<String>,
    pub skipped: Vec<String>,
    pub warned: Vec<String>,
}

pub fn temp_copy_name(name: &str) -> String {
    format!("{name}.tmp")
}

pub fn parse_timed_out(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn render_timed_out(names: &[String]) -> String {
    names.iter().map(|name| format!("{name}\n")).collect()
}

pub fn merge_timed_out(existing: &[String], selected: &[String], fresh: &[String]) -> Vec<String> {
    let selected = sorted_set(selected);
    let mut merged: Vec<String> = existing
        .iter()
        .filter(|name| !selected.contains(name))
        .chain(fresh)
        .cloned()
        .collect();
    merged.sort();
    merged.dedup();
    merged
}

pub fn select_approvals(
    names: &[String],
    all: bool,
    current_names: &[String],
    listed_names: &[String],
    timed_out: &[String],
) -> Result<Approvals, ShotError> {
    if all && !names.is_empty() {
        return Err(ShotError::AllWithNames);
    }
    let current = sorted_set(current_names);
    if all {
        let listed = sorted_set(listed_names);
        let missing: Vec<String> = listed
            .iter()
            .filter(|name| !current.contains(*name))
            .map(|name| (*name).clone())
            .collect();
        if !missing.is_empty() {
            return Err(ShotError::MissingShots(missing.join(", ")));
        }
        let (approved, skipped): (Vec<String>, Vec<String>) = current
            .into_iter()
            .cloned()
            .partition(|name| listed.contains(name));
        let timed_out = sorted_set(timed_out);
        let refused: Vec<String> = approved
            .iter()
            .filter(|name| timed_out.contains(name))
            .cloned()
            .collect();
        if !refused.is_empty() {
            return Err(ShotError::TimedOutShots(refused.join(", ")));
        }
        return Ok(Approvals {
            approved,
            skipped,
            warned: Vec::new(),
        });
    }
    if names.is_empty() {
        return Err(ShotError::NothingSelected);
    }
    if let Some(unknown) = names.iter().find(|name| !current.contains(name)) {
        return Err(ShotError::UnknownShot(unknown.clone()));
    }
    let approved: Vec<String> = sorted_set(names).into_iter().cloned().collect();
    let timed_out = sorted_set(timed_out);
    let warned = approved
        .iter()
        .filter(|name| timed_out.contains(name))
        .cloned()
        .collect();
    Ok(Approvals {
        approved,
        skipped: Vec::new(),
        warned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shots::core::{diff_images, filter, matrix, restrict_names, Rgba, Screen};

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
    fn a_common_name_without_a_measure_is_unreadable() {
        let measures = BTreeMap::from([("a.png".to_string(), pixels(0, 10))]);
        let c = classify(
            &names(&["a.png", "b.png"]),
            &names(&["a.png", "b.png"]),
            &measures,
        );
        assert_eq!(c.unreadable, names(&["b.png"]));
        assert_eq!(c.unchanged, names(&["a.png"]));
        assert!(!is_clean(&c));
    }

    #[test]
    fn the_report_lists_unreadable_shots() {
        let c = Classification {
            unreadable: names(&["b.png"]),
            ..Classification::default()
        };
        let report = render_report(&c);
        assert!(report.contains("## Unreadable (1)"));
        assert!(report.contains("- b.png"));
    }

    fn image(width: u32, height: u32, pixels: &[[u8; 4]]) -> Rgba {
        Rgba {
            width,
            height,
            pixels: pixels.iter().flatten().copied().collect(),
        }
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
    fn all_with_names_is_an_error() {
        assert_eq!(
            select_approvals(&names(&["a.png"]), true, &names(&["a.png"]), &[], &[]),
            Err(ShotError::AllWithNames)
        );
    }

    #[test]
    fn a_zero_size_image_is_refused() {
        assert_eq!(require_image_size("a.png", 4, 3), Ok(()));
        assert_eq!(
            require_image_size("a.png", 0, 3),
            Err(ShotError::EmptyImage("a.png".to_string()))
        );
        assert_eq!(
            require_image_size("a.png", 4, 0),
            Err(ShotError::EmptyImage("a.png".to_string()))
        );
    }

    fn approvals(approved: &[&str], skipped: &[&str]) -> Approvals {
        Approvals {
            approved: names(approved),
            skipped: names(skipped),
            warned: Vec::new(),
        }
    }

    #[test]
    fn approve_all_takes_every_listed_shot() {
        let current = names(&["b.png", "a.png"]);
        let listed = names(&["a.png", "b.png"]);
        assert_eq!(
            select_approvals(&[], true, &current, &listed, &[]),
            Ok(approvals(&["a.png", "b.png"], &[]))
        );
    }

    #[test]
    fn approve_all_skips_files_outside_the_shot_list() {
        let current = names(&["a.png", "extra.png"]);
        let listed = names(&["a.png"]);
        assert_eq!(
            select_approvals(&[], true, &current, &listed, &[]),
            Ok(approvals(&["a.png"], &["extra.png"]))
        );
    }

    #[test]
    fn approve_all_refuses_when_a_listed_shot_is_missing() {
        let current = names(&["a.png"]);
        let listed = names(&["a.png", "b.png", "c.png"]);
        assert_eq!(
            select_approvals(&[], true, &current, &listed, &[]),
            Err(ShotError::MissingShots("b.png, c.png".to_string()))
        );
    }

    #[test]
    fn named_approvals_ignore_the_shot_list() {
        let current = names(&["b.png", "a.png"]);
        assert_eq!(
            select_approvals(&names(&["b.png", "b.png"]), false, &current, &[], &[]),
            Ok(approvals(&["b.png"], &[]))
        );
        assert_eq!(
            select_approvals(&names(&["c.png"]), false, &current, &[], &[]),
            Err(ShotError::UnknownShot("c.png".to_string()))
        );
        assert_eq!(
            select_approvals(&[], false, &current, &[], &[]),
            Err(ShotError::NothingSelected)
        );
    }

    #[test]
    fn the_temp_copy_name_is_not_a_png() {
        assert_eq!(temp_copy_name("a-dark-1440.png"), "a-dark-1440.png.tmp");
        let listed = crate::shots::core::png_names_from(vec![
            temp_copy_name("a.png").into(),
            "b.png".into(),
        ]);
        assert_eq!(listed, Ok(names(&["b.png"])));
    }

    #[test]
    fn timed_out_names_round_trip() {
        let list = names(&["a.png", "b.png"]);
        assert_eq!(render_timed_out(&list), "a.png\nb.png\n");
        assert_eq!(parse_timed_out(&render_timed_out(&list)), list);
        assert_eq!(render_timed_out(&[]), "");
        assert_eq!(parse_timed_out("\n a.png \n\nb.png\n"), list);
        assert!(parse_timed_out("").is_empty());
    }

    #[test]
    fn merge_keeps_entries_outside_the_selection() {
        let existing = names(&["a.png", "b.png"]);
        let selected = names(&["a.png", "c.png"]);
        assert_eq!(
            merge_timed_out(&existing, &selected, &names(&["c.png"])),
            names(&["b.png", "c.png"])
        );
        assert_eq!(
            merge_timed_out(&existing, &selected, &[]),
            names(&["b.png"])
        );
        assert_eq!(
            merge_timed_out(&existing, &selected, &names(&["c.png", "c.png"])),
            names(&["b.png", "c.png"])
        );
    }

    #[test]
    fn approve_all_refuses_a_timed_out_shot() {
        let current = names(&["a.png", "b.png"]);
        let listed = names(&["a.png", "b.png"]);
        assert_eq!(
            select_approvals(&[], true, &current, &listed, &names(&["b.png", "z.png"])),
            Err(ShotError::TimedOutShots("b.png".to_string()))
        );
    }

    #[test]
    fn a_named_approval_of_a_timed_out_shot_is_allowed_with_a_warning() {
        let current = names(&["a.png", "b.png"]);
        let selection =
            select_approvals(&names(&["b.png"]), false, &current, &[], &names(&["b.png"]));
        assert_eq!(
            selection,
            Ok(Approvals {
                approved: names(&["b.png"]),
                skipped: Vec::new(),
                warned: names(&["b.png"]),
            })
        );
    }

    #[test]
    fn restricted_sets_report_no_missing_for_other_screens() {
        let selected = filter(matrix(), Some(Screen::Memory), None);
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
}
