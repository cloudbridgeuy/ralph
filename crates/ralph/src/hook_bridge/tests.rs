use super::*;
use ralph_core::worker::ALLOW_ONCE;
use std::sync::Mutex;

fn served(
    answer: Option<&'static str>,
) -> (tempfile::TempDir, HookSocket, Arc<Mutex<Vec<String>>>) {
    let dir = tempfile::tempdir().unwrap();
    let socket = HookSocket::bind(&dir.path().join("hook.sock")).unwrap();
    let ids = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&ids);
    socket
        .serve(move |id, _ask| {
            seen.lock().unwrap().push(id);
            answer.map(str::to_string)
        })
        .unwrap();
    (dir, socket, ids)
}

fn ask(tool_use_id: &str) -> BridgeAsk {
    BridgeAsk {
        tool_use_id: tool_use_id.to_string(),
        command: "echo hi".to_string(),
    }
}

#[test]
fn the_chosen_option_reaches_the_hook() {
    let (_dir, socket, ids) = served(Some(ALLOW_ONCE));
    assert_eq!(
        ask_bridge(socket.path(), &ask("t1")),
        HookOutcome::Answered(ALLOW_ONCE.to_string())
    );
    assert_eq!(
        ask_bridge(socket.path(), &ask("")),
        HookOutcome::Answered(ALLOW_ONCE.to_string())
    );
    assert_eq!(
        *ids.lock().unwrap(),
        vec!["t1".to_string(), "hook-1".to_string()]
    );
}

#[test]
fn no_answer_closes_the_bridge() {
    let (_dir, socket, _ids) = served(None);
    assert!(matches!(
        ask_bridge(socket.path(), &ask("t1")),
        HookOutcome::BridgeGone(_)
    ));
}

#[test]
fn the_socket_is_private_and_removed_on_drop() {
    let (dir, socket, _ids) = served(None);
    let path = socket.path().to_path_buf();
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    drop(socket);
    assert!(!path.exists());
    assert!(matches!(
        ask_bridge(&dir.path().join("hook.sock"), &ask("t1")),
        HookOutcome::BridgeGone(_)
    ));
}
