use super::*;
use crate::permission_queue::PermissionQueue;
use crate::worker_events::Events;
use crate::worker_mcp::Inbox;
use ralph_core::worker::{WorkerEvent, ALLOW_ONCE};
use ralph_server::hook_bridge::{deliver, HookQueue, HookReply, HookSocket};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

struct Served {
    dir: tempfile::TempDir,
    socket: HookSocket,
    queue: Arc<HookQueue>,
    events: Events,
}

fn served() -> Served {
    let dir = tempfile::tempdir().unwrap();
    let socket = HookSocket::bind(&dir.path().join("hook.sock")).unwrap();
    let events = Events::default();
    let inbox = Inbox::new(dir.path().join("inbox"));
    let queue = Arc::new(PermissionQueue::new(
        "p".into(),
        "w".into(),
        inbox,
        Arc::clone(&events),
    ));
    socket.serve(Arc::clone(&queue)).unwrap();
    Served {
        dir,
        socket,
        queue,
        events,
    }
}

fn ask(tool_use_id: &str) -> BridgeAsk {
    BridgeAsk {
        tool_use_id: tool_use_id.to_string(),
        command: "echo hi".to_string(),
    }
}

fn next_event(events: &Events, wanted: fn(&WorkerEvent) -> bool) -> WorkerEvent {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let found = {
            let mut events = events.lock().unwrap();
            let at = events.iter().position(wanted);
            at.map(|at| events.remove(at))
        };
        if let Some(event) = found {
            return event;
        }
        assert!(Instant::now() < deadline, "no event in time");
        thread::sleep(Duration::from_millis(20));
    }
}

fn asked(events: &Events) -> String {
    match next_event(events, |e| matches!(e, WorkerEvent::PermissionAsked(_))) {
        WorkerEvent::PermissionAsked(content) => content.request_id,
        other => panic!("{other:?}"),
    }
}

fn answer(served: &Served, request_id: &str) -> Result<(), String> {
    let (reply, option) = served.queue.take_answer(request_id, ALLOW_ONCE)?;
    deliver(reply, option.option_id, request_id)
}

#[test]
fn the_chosen_option_reaches_the_hook() {
    let served = served();
    for (tool_use_id, request_id) in [("t1", "p.t1"), ("", "p.hook-1")] {
        let path = served.socket.path().to_path_buf();
        let hook = thread::spawn(move || ask_bridge(&path, &ask(tool_use_id)));
        assert_eq!(asked(&served.events), request_id);
        answer(&served, request_id).unwrap();
        assert_eq!(
            hook.join().unwrap(),
            HookOutcome::Answered(ALLOW_ONCE.to_string())
        );
    }
}

#[test]
fn a_hook_that_leaves_settles_its_request_and_a_late_answer_is_stale() {
    let served = served();
    let mut hook = UnixStream::connect(served.socket.path()).unwrap();
    hook.write_all(encode_line(&ask("t1")).as_bytes()).unwrap();
    assert_eq!(asked(&served.events), "p.t1");
    drop(hook);
    next_event(&served.events, |e| {
        matches!(e, WorkerEvent::PermissionSettled)
    });
    let late = answer(&served, "p.t1").unwrap_err();
    assert!(late.contains("stale request p.t1"), "{late}");
}

#[test]
fn an_answer_after_the_hook_gave_up_is_refused() {
    let (answer, chosen) = mpsc::channel();
    drop(chosen);
    let reply = HookReply {
        id: "t1".to_string(),
        answer,
    };
    let refused = deliver(reply, ALLOW_ONCE.to_string(), "p.t1").unwrap_err();
    assert!(refused.contains("stale request p.t1"), "{refused}");
}

#[test]
fn the_socket_is_private_and_removed_on_drop() {
    let Served { dir, socket, .. } = served();
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
