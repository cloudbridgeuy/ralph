use super::*;

#[test]
fn send_it_commands_round_trip_through_json() {
    let command = SendItCommand::Save {
        name: "n".into(),
        ticket: None,
    };
    let json = serde_json::to_string(&command).unwrap();
    assert_eq!(
        serde_json::from_str::<SendItCommand>(&json).unwrap(),
        command
    );
}
