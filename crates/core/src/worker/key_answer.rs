use super::{answer_message, Choice, Classes, Grants};

pub struct GrantsChange<'a> {
    pub before: Grants,
    pub after: Grants,
    pub path: &'a str,
}

pub fn key_answer_report(
    worker: &str,
    choice: Choice,
    sent: Result<String, String>,
    grants: GrantsChange,
) -> String {
    let sent = match sent {
        Ok(sent) => sent,
        Err(error) => return format!("worker {worker}: {error}"),
    };
    let added = Classes {
        writes: grants.after.writes && !grants.before.writes,
        spend: grants.after.spend && !grants.before.spend,
    };
    let note = match choice {
        Choice::AllowAlways if added == Classes::default() => {
            "Allowed; no new grant saved.".to_string()
        }
        _ => answer_message(choice, added, Some(grants.path)),
    };
    format!("{sent} {note}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_report_names_the_worker_the_choice_and_only_a_newly_saved_grant() {
        let writes = Grants {
            writes: true,
            spend: false,
        };
        let report = |choice, before, after, sent: Result<&str, &str>| {
            key_answer_report(
                "w1",
                choice,
                sent.map(str::to_string).map_err(str::to_string),
                GrantsChange {
                    before,
                    after,
                    path: "/s/g.json",
                },
            )
        };
        let none = Grants::default();
        let sent = Ok("Answered w1.");
        assert_eq!(
            report(Choice::AllowOnce, none, none, sent),
            "Answered w1. Allowed once."
        );
        assert_eq!(
            report(Choice::Deny, none, none, sent),
            "Answered w1. Denied."
        );
        assert_eq!(
            report(Choice::AllowAlways, none, writes, sent),
            "Answered w1. Always allowing writes; saved to /s/g.json."
        );
        assert_eq!(
            report(Choice::AllowAlways, writes, writes, sent),
            "Answered w1. Allowed; no new grant saved."
        );
        assert_eq!(
            report(Choice::AllowAlways, none, writes, Err("timed out")),
            "worker w1: timed out"
        );
    }
}
