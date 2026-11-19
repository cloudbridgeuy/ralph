use super::anchors::Group;
use super::era::{Block, Section, ERAS};
use super::reach::{recall_cell, Bucket, ReachTally};
use super::survival::{Cell, Table};
use super::tokens::Class;
use super::Decay;
use std::collections::BTreeSet;

const CLASSES: [(Class, &str); 4] = [
    (Class::Ticket, "ticket"),
    (Class::Number, "number"),
    (Class::Identifier, "identifier"),
    (Class::Rare, "rare"),
];
const BUCKETS: [(Bucket, &str); 5] = [
    (Bucket::One, "1"),
    (Bucket::TwoToFive, "2-5"),
    (Bucket::SixToTwenty, "6-20"),
    (Bucket::Over, ">20"),
    (Bucket::Miss, "miss"),
];
const COLUMN: usize = 12;

fn show(cell: Cell) -> String {
    match cell.percent() {
        Some(percent) => format!("{percent}% ({}/{})", cell.kept, cell.total),
        None => "-".to_string(),
    }
}

fn survival_lines(table: &Table) -> Vec<String> {
    if table.is_empty() {
        return vec!["survival: no summarized ancestors".to_string()];
    }
    let mut lines = vec!["survival (key tokens kept in the summary) by depth:".to_string()];
    let header: String = CLASSES
        .iter()
        .map(|(_, name)| format!("{name:<COLUMN$}"))
        .collect();
    lines.push(format!("{:<6}{}", "depth", header.trim_end()));
    let depths: BTreeSet<u32> = table.keys().map(|(depth, _)| *depth).collect();
    for depth in depths {
        let cells: String = CLASSES
            .iter()
            .map(|(class, _)| {
                let cell = table.get(&(depth, *class)).copied().unwrap_or_default();
                format!("{:<COLUMN$}", show(cell))
            })
            .collect();
        lines.push(format!("{depth:<6}{}", cells.trim_end()));
    }
    lines
}

fn reach_lines(reach: &ReachTally) -> Vec<String> {
    let count = |map: &std::collections::BTreeMap<Bucket, usize>, bucket| {
        map.get(&bucket).copied().unwrap_or(0)
    };
    let ranks: Vec<String> = BUCKETS
        .iter()
        .map(|(bucket, name)| format!("{name} {}", count(&reach.search, *bucket)))
        .collect();
    vec![
        format!(
            "search rank: {}, unreachable {}",
            ranks.join(", "),
            reach.unreachable
        ),
        format!("recall found: {}", show(recall_cell(reach))),
    ]
}

fn block_lines(block: &Block) -> Vec<String> {
    if block.is_empty() {
        return vec!["no data".to_string()];
    }
    let mut lines = survival_lines(&block.survival);
    lines.extend(reach_lines(&block.reach));
    lines.push(format!("pointer kept: {}", show(block.pointer)));
    lines
}

fn indented(lines: Vec<String>) -> impl Iterator<Item = String> {
    lines.into_iter().map(|line| format!("  {line}"))
}

fn section_lines(name: &str, anchors: usize, section: &Section) -> Vec<String> {
    let mut lines = vec![String::new(), format!("== {name} ({anchors} anchors) ==")];
    lines.extend(block_lines(&section.overall));
    lines.push("by era (approximate: summaries are written after their entries):".to_string());
    for era in ERAS {
        lines.push(format!("{}:", era.label()));
        let empty = Block::default();
        let block = section.eras.get(&era).unwrap_or(&empty);
        lines.extend(indented(block_lines(block)));
    }
    lines
}

pub fn render_decay(decay: &Decay) -> String {
    let count = |group| decay.anchors.iter().filter(|a| a.group == group).count();
    let (tagged, top_up) = (count(Group::Tagged), count(Group::TopUp));
    let mut lines = vec![format!(
        "anchors {} (tagged {tagged}, top-up {top_up}) uncompressed {}",
        decay.anchors.len(),
        decay.uncompressed
    )];
    let empty = Section::default();
    for (group, name, anchors) in [
        (Group::Tagged, "tagged", tagged),
        (Group::TopUp, "top-up", top_up),
    ] {
        let section = decay.sections.get(&group).unwrap_or(&empty);
        lines.extend(section_lines(name, anchors, section));
    }
    lines.join("\n")
}
