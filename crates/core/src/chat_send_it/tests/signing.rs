use super::approval::Refusal;
use super::*;

pub(super) const KEY: &[u8] = &[7; 32];
pub(super) const NONCE: &str = "n1";
pub(super) const TRUST: Trust<'static> = Trust {
    key: KEY,
    live: Some(NONCE),
};

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

pub(super) fn approved(plan: RunPlan) -> RunFile {
    RunFile {
        approved: Some(signed(&plan)),
        plan,
        stopped: None,
    }
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
fn an_old_approved_string_reads_as_an_unsigned_record() {
    let text = format!(
        "approved = \"{}\"\n{}",
        plan_hash(&plan()),
        write_run(&run()).unwrap()
    );
    let file = read_run(&text, limits()).unwrap();
    assert_eq!(file.approved, Some(Approval::default()));
    assert!(!file.is_approved(TRUST));
    assert!(!write_run(&file).unwrap().contains("approved"));
}

fn run() -> RunFile {
    RunFile {
        plan: plan(),
        approved: None,
        stopped: None,
    }
}

fn verdict(approval: &Approval, plan: &RunPlan, trust: Trust) -> Result<(), Refusal> {
    approval::verify(Some(approval), plan, trust)
}

#[test]
fn a_fresh_signed_approval_verifies() {
    assert_eq!(verdict(&signed(&plan()), &plan(), TRUST), Ok(()));
}

#[test]
fn no_record_is_unsigned() {
    assert_eq!(
        approval::verify(None, &plan(), TRUST),
        Err(Refusal::Unsigned)
    );
}

#[test]
fn a_flipped_mac_character_is_tampered() {
    let mut approval = signed(&plan());
    let first = if approval.mac.starts_with('0') {
        "1"
    } else {
        "0"
    };
    approval.mac.replace_range(..1, first);
    assert_eq!(verdict(&approval, &plan(), TRUST), Err(Refusal::Tampered));
    approval.mac = "not hex".to_string();
    assert_eq!(verdict(&approval, &plan(), TRUST), Err(Refusal::Tampered));
}

#[test]
fn an_edited_field_breaks_the_mac() {
    let approval = Approval {
        peer_uid: 0,
        ..signed(&plan())
    };
    assert_eq!(verdict(&approval, &plan(), TRUST), Err(Refusal::Tampered));
}

#[test]
fn a_record_signed_with_another_key_is_tampered() {
    let other = Trust {
        key: &[8; 32],
        ..TRUST
    };
    assert_eq!(
        verdict(&signed(&plan()), &plan(), other),
        Err(Refusal::Tampered)
    );
}

#[test]
fn a_validly_signed_record_of_another_kind_is_wrong_kind() {
    let approval = Approval {
        kind: "other".to_string(),
        ..signed(&plan())
    }
    .seal(KEY)
    .unwrap();
    assert_eq!(verdict(&approval, &plan(), TRUST), Err(Refusal::WrongKind));
}

#[test]
fn an_edited_plan_step_with_the_old_record_is_wrong_plan() {
    let approval = signed(&plan());
    let mut edited = plan();
    edited.steps[0].task = "run the tests".to_string();
    assert_eq!(verdict(&approval, &edited, TRUST), Err(Refusal::WrongPlan));
}

#[test]
fn a_record_whose_nonce_is_not_live_is_replayed() {
    let approval = signed(&plan());
    for live in [None, Some("n2")] {
        assert_eq!(
            verdict(&approval, &plan(), Trust { live, ..TRUST }),
            Err(Refusal::Replayed)
        );
    }
}

#[test]
fn run_file_approval_follows_the_verdict() {
    let file = approved(plan());
    assert!(file.is_approved(TRUST));
    assert!(!file.is_approved(Trust::NONE));
    assert!(!run().is_approved(TRUST));
}
