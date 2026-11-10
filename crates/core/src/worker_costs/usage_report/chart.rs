use super::{Group, EST};
use crate::worker_costs::NO_USAGE;
use chrono::{Duration, NaiveDate};

pub const REPORTED_FILL: &str = "#7aa2f7";
pub const EST_FILL: &str = "#9ece6a";
const BACKGROUND: &str = "#1e1f29";
const TEXT: &str = "#e6e6e6";
const MUTED: &str = "#8a8fa3";
const GRID: &str = "#333645";
const WIDTH: f64 = 900.0;
const DAILY_HEIGHT: f64 = 420.0;
const PLOT_LEFT: f64 = 70.0;
const PLOT_RIGHT: f64 = 870.0;
const PLOT_TOP: f64 = 110.0;
const PLOT_BOTTOM: f64 = 370.0;
const BAR_FILL_RATIO: f64 = 0.8;
const MAX_X_LABELS: usize = 16;
const BAR_LABEL_X: f64 = 220.0;
const BAR_START_X: f64 = 230.0;
const BAR_MAX_WIDTH: f64 = 560.0;
const BAR_ROW: f64 = 28.0;
const BAR_TOP: f64 = 75.0;
const BAR_THICKNESS: f64 = 18.0;

pub struct Heading<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
}

fn escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&#x27;".to_string(),
            other => other.to_string(),
        })
        .collect()
}

fn money(usd: f64) -> String {
    if usd < 100.0 {
        return format!("${usd:.2}");
    }
    let whole = format!("{usd:.0}");
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("${grouped}")
}

fn reported(group: &Group) -> f64 {
    group.reported.known.unwrap_or(0.0)
}

fn computed(group: &Group) -> f64 {
    group.est.known.unwrap_or(0.0)
}

fn total(group: &Group) -> f64 {
    reported(group) + computed(group)
}

fn open(height: f64, heading: &Heading) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{height}\" font-family=\"Menlo,monospace\"><rect width=\"100%\" height=\"100%\" fill=\"{BACKGROUND}\"/><text x=\"20\" y=\"32\" fill=\"{TEXT}\" font-size=\"18\" font-weight=\"bold\">{}</text><text x=\"20\" y=\"52\" fill=\"{MUTED}\" font-size=\"12\">{}</text>",
        escape(heading.title),
        escape(heading.subtitle),
    )
}

struct Style<'a>(&'a str, u8, &'a str);

fn text(x: f64, y: f64, style: Style, content: &str) -> String {
    let Style(fill, size, anchor) = style;
    format!(
        "<text x=\"{x:.1}\" y=\"{y:.1}\" fill=\"{fill}\" font-size=\"{size}\" text-anchor=\"{anchor}\">{}</text>",
        escape(content)
    )
}

fn bar(x: f64, y: f64, width: f64, height: f64, fill: &str) -> String {
    format!(
        "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{width:.1}\" height=\"{height:.1}\" fill=\"{fill}\"/>"
    )
}

fn empty(height: f64, heading: &Heading) -> String {
    format!(
        "{}{}</svg>",
        open(height, heading),
        text(20.0, 90.0, Style(MUTED, 13, "start"), NO_USAGE)
    )
}

pub fn pad_days(days: &[Group]) -> Vec<Group> {
    let parse = |group: &Group| NaiveDate::parse_from_str(&group.key, "%Y-%m-%d").ok();
    let (Some(first), Some(last)) = (days.first().and_then(parse), days.last().and_then(parse))
    else {
        return days.to_vec();
    };
    let span = (last - first).num_days();
    (0..=span)
        .map(|offset| {
            let key = (first + Duration::days(offset)).to_string();
            days.iter()
                .find(|group| group.key == key)
                .cloned()
                .unwrap_or_else(|| Group {
                    key,
                    ..Group::default()
                })
        })
        .collect()
}

fn nice_step(max: f64) -> f64 {
    let raw = max / 4.0;
    let magnitude = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|factor| factor * magnitude)
        .find(|step| *step >= raw)
        .unwrap_or(10.0 * magnitude)
}

fn legend() -> String {
    let entry = |x: f64, fill: &str, label: &str| {
        format!(
            "<circle cx=\"{x}\" cy=\"72\" r=\"5\" fill=\"{fill}\"/>{}",
            text(x + 12.0, 76.0, Style(TEXT, 12, "start"), label)
        )
    };
    format!(
        "{}{}",
        entry(690.0, REPORTED_FILL, "reported"),
        entry(790.0, EST_FILL, EST)
    )
}

fn x_label(key: &str) -> &str {
    key.get(5..).unwrap_or(key)
}

pub fn daily_svg(days: &[Group], heading: &Heading) -> String {
    let days = pad_days(days);
    if days.is_empty() {
        return empty(DAILY_HEIGHT, heading);
    }
    let peak = days.iter().map(total).fold(0.0, f64::max);
    let step = nice_step(if peak > 0.0 { peak } else { 1.0 });
    let ticks = (peak / step).ceil().max(1.0) as usize;
    let axis_max = step * ticks as f64;
    let plot_height = PLOT_BOTTOM - PLOT_TOP;
    let height_of = |usd: f64| usd / axis_max * plot_height;
    let mut svg = open(DAILY_HEIGHT, heading) + &legend();
    for tick in 0..=ticks {
        let y = PLOT_BOTTOM - height_of(step * tick as f64);
        svg += &format!(
            "<line x1=\"{PLOT_LEFT}\" x2=\"{PLOT_RIGHT}\" y1=\"{y:.1}\" y2=\"{y:.1}\" stroke=\"{GRID}\"/>"
        );
        svg += &text(
            PLOT_LEFT - 8.0,
            y + 4.0,
            Style(MUTED, 11, "end"),
            &money(step * tick as f64),
        );
    }
    let slot = (PLOT_RIGHT - PLOT_LEFT) / days.len() as f64;
    let width = slot * BAR_FILL_RATIO;
    let stride = days.len().div_ceil(MAX_X_LABELS);
    for (index, day) in days.iter().enumerate() {
        let x = PLOT_LEFT + slot * index as f64 + (slot - width) / 2.0;
        let mut top = PLOT_BOTTOM;
        for (usd, fill) in [(reported(day), REPORTED_FILL), (computed(day), EST_FILL)] {
            if usd > 0.0 {
                top -= height_of(usd);
                svg += &bar(x, top, width, height_of(usd), fill);
            }
        }
        svg += &text(
            x + width / 2.0,
            top - 5.0,
            Style(TEXT, 10, "middle"),
            &format!("{:.0}", total(day)),
        );
        if index % stride == 0 {
            svg += &text(
                x + width / 2.0,
                PLOT_BOTTOM + 16.0,
                Style(MUTED, 10, "middle"),
                x_label(&day.key),
            );
        }
    }
    svg + "</svg>"
}

fn value_text(group: &Group, grand: f64) -> String {
    let share = if grand > 0.0 {
        total(group) / grand * 100.0
    } else {
        0.0
    };
    let mark = if computed(group) > 0.0 {
        format!(" {EST}")
    } else {
        String::new()
    };
    format!("{}{mark} ({share:.0}%)", money(total(group)))
}

pub fn bars_svg(groups: &[Group], heading: &Heading) -> String {
    let height = BAR_TOP + BAR_ROW * groups.len() as f64 + 20.0;
    if groups.is_empty() {
        return empty(height, heading);
    }
    let grand: f64 = groups.iter().map(total).sum();
    let peak = groups.iter().map(total).fold(0.0, f64::max);
    let mut svg = open(height, heading);
    for (index, group) in groups.iter().enumerate() {
        let y = BAR_TOP + BAR_ROW * index as f64;
        let baseline = y + BAR_THICKNESS - 4.0;
        if group.metered == 0 || group.all_excluded() || group.usd().known.is_none() {
            svg += &text(
                BAR_START_X,
                baseline,
                Style(MUTED, 12, "start"),
                &group.line(),
            );
            continue;
        }
        svg += &text(BAR_LABEL_X, baseline, Style(TEXT, 12, "end"), &group.key);
        let scale = if peak > 0.0 {
            BAR_MAX_WIDTH / peak
        } else {
            0.0
        };
        let mut x = BAR_START_X;
        for (usd, fill) in [
            (reported(group), REPORTED_FILL),
            (computed(group), EST_FILL),
        ] {
            if usd > 0.0 {
                svg += &bar(x, y, usd * scale, BAR_THICKNESS, fill);
                x += usd * scale;
            }
        }
        svg += &text(
            x + 6.0,
            baseline,
            Style(MUTED, 11, "start"),
            &value_text(group, grand),
        );
    }
    svg + "</svg>"
}

#[cfg(test)]
mod tests;
