use super::*;
use crate::chat_help::help_text;
use crate::chat_thread::ThreadName;
use crate::chat_thread_cmd::{is_cell_command, op_refusal};

fn route(line: &str) -> Route {
    route_input(line, 0, &EmptySubmit::Inbox, &BTreeSet::new())
}

fn name(word: &str) -> ThreadName {
    ThreadName::parse(word).unwrap()
}

fn op(op: ThreadOp, line: &str) -> Route {
    Route::ThreadOp {
        op: Ok(op),
        line: line.to_string(),
    }
}

fn switch(target: SwitchTarget, line: &str) -> Route {
    Route::Switch {
        parsed: Ok(target),
        line: line.to_string(),
    }
}

#[test]
fn thread_subcommands_route_to_the_thread_operations() {
    assert_eq!(route("/thread ls"), op(ThreadOp::Ls, "/thread ls"));
    assert_eq!(
        route("/thread pull auth"),
        op(ThreadOp::Pull(name("auth")), "/thread pull auth")
    );
    assert_eq!(
        route("/thread park auth"),
        op(ThreadOp::Park(name("auth")), "/thread park auth")
    );
    assert_eq!(
        route("/thread merge auth"),
        op(
            ThreadOp::Merge {
                name: name("auth"),
                view: false
            },
            "/thread merge auth"
        )
    );
    assert_eq!(
        route("/thread merge auth --view"),
        op(
            ThreadOp::Merge {
                name: name("auth"),
                view: true
            },
            "/thread merge auth --view"
        )
    );
}

#[test]
fn thread_switch_takes_a_thread_name_and_main_stays_a_keyword() {
    assert_eq!(
        route("/thread switch auth"),
        switch(SwitchTarget::Thread(name("auth")), "/thread switch auth")
    );
    assert_eq!(
        route("/thread main"),
        switch(SwitchTarget::Main, "/thread main")
    );
    for line in [
        "/thread switch main",
        "/thread auth",
        "/thread",
        "/thread main now",
    ] {
        assert!(
            matches!(route(line), Route::Switch { parsed: Err(_), .. }),
            "{line}"
        );
    }
}

#[test]
fn reserved_looking_thread_names_stay_valid_names() {
    assert_eq!(
        route("/thread switch ls"),
        switch(SwitchTarget::Thread(name("ls")), "/thread switch ls")
    );
    assert_eq!(
        route("/thread park drop"),
        op(ThreadOp::Park(name("drop")), "/thread park drop")
    );
    assert_eq!(
        route("/thread pull switch"),
        op(ThreadOp::Pull(name("switch")), "/thread pull switch")
    );
}

#[test]
fn malformed_thread_subcommands_get_a_usage_refusal() {
    for line in [
        "/thread ls auth",
        "/thread pull",
        "/thread park a b",
        "/thread merge",
        "/thread merge a b",
        "/thread merge --view",
        "/thread merge a --force",
        "/thread pull Bad_Name",
    ] {
        assert!(
            matches!(route(line), Route::ThreadOp { op: Err(_), .. }),
            "{line}"
        );
    }
}

#[test]
fn the_old_top_level_thread_commands_are_plain_messages() {
    for line in [
        "/threads",
        "/pull auth",
        "/park auth",
        "/merge auth",
        "/merge auth --view",
        "/main",
    ] {
        assert_eq!(route(line), Route::Send(line.to_string()), "{line}");
    }
}

#[test]
fn only_the_thread_command_is_a_cell_command() {
    for line in [
        "/thread ls",
        "/thread merge a --view",
        "/thread main",
        "/thread a b",
    ] {
        assert!(is_cell_command(line), "{line}");
    }
    for line in ["/threads", "/merge a", "/pull a", "/park a", "/main"] {
        assert!(!is_cell_command(line), "{line}");
    }
}

#[test]
fn the_thread_chat_refusal_points_at_the_thread_subcommands() {
    assert_eq!(
        op_refusal(true, &ThreadOp::Park(name("auth"))),
        Some(
            "/thread park runs in main's chat. Run /thread main, then /thread park <name>."
                .to_string()
        )
    );
}

#[test]
fn help_lists_the_thread_subcommands_and_none_of_the_old_commands() {
    let wide = help_text(2_000);
    for row in [
        "/thread ls",
        "/thread merge <name> --view",
        "/thread park <name>",
        "/thread pull <name>",
        "/thread switch <name>",
        "/thread main",
    ] {
        assert!(wide.contains(row), "{row}");
    }
    let old = ["/threads", "/pull ", "/park ", "/merge ", "/main"];
    let names = Command::ALL.map(Command::name);
    for gone in old {
        assert!(!names.contains(&gone.trim()), "{gone}");
        assert!(!wide.contains(&format!("\n{gone}")), "{gone}");
    }
}

fn op_debug(line: &str) -> String {
    match route(line) {
        Route::ThreadOp { op: Ok(op), .. } => format!("{op:?}"),
        other => format!("not a thread op: {other:?}"),
    }
}

#[test]
fn thread_drop_and_revive_route_to_the_thread_operations() {
    assert_eq!(op_debug("/thread drop auth"), "Drop(ThreadName(\"auth\"))");
    assert_eq!(
        op_debug("/thread revive auth"),
        "Revive(ThreadName(\"auth\"))"
    );
}

#[test]
fn drop_and_revive_accept_subcommand_words_as_thread_names() {
    assert_eq!(op_debug("/thread drop drop"), "Drop(ThreadName(\"drop\"))");
    assert_eq!(op_debug("/thread revive ls"), "Revive(ThreadName(\"ls\"))");
}

#[test]
fn malformed_drop_and_revive_get_a_usage_refusal() {
    for line in [
        "/thread drop",
        "/thread drop a b",
        "/thread drop Bad_Name",
        "/thread revive",
        "/thread revive a b",
    ] {
        assert!(
            matches!(route(line), Route::ThreadOp { op: Err(_), .. }),
            "{line}"
        );
    }
}

#[test]
fn help_lists_drop_and_revive() {
    let wide = help_text(2_000);
    for row in ["/thread drop <name>", "/thread revive <name>"] {
        assert!(wide.contains(row), "{row}");
    }
}
