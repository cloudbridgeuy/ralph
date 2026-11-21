use super::*;

pub(super) const KEY: &[u8] = &[7; 32];
pub(super) const NONCE: &str = "n1";

pub(super) fn signing() -> Signing<'static> {
    Signing {
        key: KEY,
        at: "2026-01-01T00:00:00Z",
        peer_uid: 501,
        nonce: NONCE,
    }
}

pub(super) fn signed(plan: &RunPlan) -> Approval {
    Approval::sign(&signing(), plan).unwrap()
}

pub(super) fn signed_go(
    file: &RunFile,
    shown: Option<&str>,
    typed_at_prompt: bool,
) -> Result<RunFile, String> {
    approve(file, shown, typed_at_prompt, &signing())
}

#[test]
fn a_signed_approval_carries_the_full_plan_hash_and_a_mac() {
    let file = signed_go(&run(), Some(&plan_hash(&plan())), true).unwrap();
    let approval = file.approved.clone().unwrap();
    assert_eq!(approval.kind, "send-it-go");
    assert_eq!(approval.subject_hash.len(), 64);
    assert!(approval.subject_hash.starts_with(&plan_hash(&plan())));
    assert_eq!(approval.mac.len(), 64);
    let text = write_run(&file).unwrap();
    assert!(text.contains("[approved]"), "{text}");
    assert!(
        text.contains(&format!("mac = \"{}\"", approval.mac)),
        "{text}"
    );
    assert!(
        text.contains(&format!("subject_hash = \"{}\"", approval.subject_hash)),
        "{text}"
    );
    assert_eq!(read_run(&text, limits()), Ok(file));
}

#[test]
fn another_key_signs_a_different_mac() {
    let other = Signing {
        key: &[8; 32],
        ..signing()
    };
    let first = Approval::sign(&signing(), &plan()).unwrap();
    assert_ne!(first.mac, Approval::sign(&other, &plan()).unwrap().mac);
}

#[test]
fn an_old_approved_string_reads_as_not_approved() {
    let text = format!(
        "{}approved = \"{}\"\n",
        write_run(&run()).unwrap(),
        plan_hash(&plan())
    );
    assert_eq!(read_run(&text, limits()).unwrap().approved, None);
}

fn run() -> RunFile {
    RunFile {
        plan: plan(),
        approved: None,
        stopped: None,
    }
}
