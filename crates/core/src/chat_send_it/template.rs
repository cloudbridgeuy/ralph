use super::{read_run, write_run, Limits, RunFile, RunPlan};
use crate::worker::is_ticket_key;

pub const TICKET: &str = "{ticket}";

fn key<'a>(what: &str, text: &'a str) -> Result<&'a str, String> {
    if is_ticket_key(text) {
        Ok(text)
    } else {
        Err(format!(
            "{what} '{text}' may only use letters, digits and dashes, with at least one letter or digit."
        ))
    }
}

pub fn template_name(name: &str) -> Result<&str, String> {
    key("Template name", name)
}

fn swap_token(text: &str, from: &str, to: &str) -> String {
    let joins = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-');
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (at, _) in text.match_indices(from) {
        let end = at + from.len();
        if joins(text[..at].chars().next_back()) || joins(text[end..].chars().next()) {
            continue;
        }
        out.push_str(&text[last..at]);
        out.push_str(to);
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

pub fn template_text(plan: &RunPlan, ticket: Option<&str>) -> Result<String, String> {
    let text = write_run(&RunFile {
        plan: plan.clone(),
        approved: None,
    })?;
    match ticket {
        Some(ticket) => Ok(swap_token(&text, key("Ticket", ticket)?, TICKET)),
        None => Ok(text),
    }
}

pub fn save_notice(name: &str, ticket: Option<&str>, text: &str, replaced: bool) -> String {
    let what = match ticket {
        None => ". No ticket given: the plan is saved as is.".to_string(),
        Some(ticket) if text.contains(TICKET) => format!(": {ticket} became {TICKET}."),
        Some(ticket) => format!(": {ticket} is not in the plan, so it is saved as is."),
    };
    let verb = if replaced { "replaced" } else { "saved" };
    format!("Template {name} {verb}{what} Use it with /send-it <ticket> {name}.")
}

pub fn fill_template(text: &str, ticket: &str, defaults: Limits) -> Result<RunFile, String> {
    let file = read_run(&text.replace(TICKET, key("Ticket", ticket)?), defaults)?;
    Ok(RunFile {
        approved: None,
        ..file
    })
}

pub fn match_template(text: &str, names: &[String]) -> Option<(String, String)> {
    let mut words = text.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (Some(ticket), Some(name), None) if names.iter().any(|known| known == name) => {
            Some((ticket.to_string(), name.to_string()))
        }
        _ => None,
    }
}

pub fn render_templates(names: &[String]) -> String {
    if names.is_empty() {
        "No templates. Save the stored plan with /send-it save <name> [<ticket>].".to_string()
    } else {
        format!(
            "Templates: {}. Use one with /send-it <ticket> <name>.",
            names.join(", ")
        )
    }
}
