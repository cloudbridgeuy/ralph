use super::*;

#[test]
fn the_default_verify_mode_is_observe() {
    assert_eq!(VerifyMode::default(), VerifyMode::Observe);
}

#[test]
fn verify_mode_parses_each_lowercase_name() {
    assert_eq!("off".parse(), Ok(VerifyMode::Off));
    assert_eq!("observe".parse(), Ok(VerifyMode::Observe));
    assert_eq!("enforce".parse(), Ok(VerifyMode::Enforce));
}

#[test]
fn verify_mode_trims_the_name() {
    assert_eq!(" enforce ".parse(), Ok(VerifyMode::Enforce));
}

#[test]
fn a_bad_verify_mode_is_refused_and_lists_the_choices() {
    let error = "strict".parse::<VerifyMode>().unwrap_err();
    assert!(error.contains("\"strict\""), "{error}");
    assert!(error.contains("off, observe or enforce"), "{error}");
    assert!("Observe".parse::<VerifyMode>().is_err());
    assert!("".parse::<VerifyMode>().is_err());
}
