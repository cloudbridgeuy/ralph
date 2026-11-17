use super::*;

#[test]
fn the_child_command_runs_the_text_through_sh() {
    let command = child_command("echo hi", None);
    assert_eq!(command.get_program(), "sh");
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(args, ["-c", "echo hi"]);
}

#[test]
fn the_child_command_keeps_the_current_dir_without_a_cwd() {
    assert_eq!(child_command("true", None).get_current_dir(), None);
}

#[test]
fn the_child_command_runs_in_the_given_cwd() {
    let command = child_command("true", Some(Path::new("/repo/.worktrees/E1")));
    assert_eq!(
        command.get_current_dir(),
        Some(Path::new("/repo/.worktrees/E1"))
    );
}
