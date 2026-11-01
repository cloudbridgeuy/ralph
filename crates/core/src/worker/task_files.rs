use super::WORKERS_DIR;
use std::path::{Path, PathBuf};

pub const TASK_FILE: &str = "task.md";

pub fn task_dir_path(store: &Path, id: &str) -> PathBuf {
    store.join(WORKERS_DIR).join(format!("{id}.task"))
}

pub fn task_file_path(task_dir: &Path) -> PathBuf {
    task_dir.join(TASK_FILE)
}

pub fn task_pointer(task_dir: &Path) -> String {
    format!(
        "Read your task from {}",
        task_file_path(task_dir).to_string_lossy()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_dir_sits_beside_the_worker_files() {
        assert_eq!(
            task_dir_path(Path::new("/s"), "A"),
            PathBuf::from("/s/workers/A.task")
        );
    }

    #[test]
    fn the_task_file_lives_inside_the_task_dir() {
        assert_eq!(
            task_file_path(Path::new("/s/workers/A.task")),
            PathBuf::from("/s/workers/A.task/task.md")
        );
    }

    #[test]
    fn the_pointer_names_the_task_file_and_nothing_else() {
        assert_eq!(
            task_pointer(Path::new("/s/workers/A.task")),
            "Read your task from /s/workers/A.task/task.md"
        );
    }
}
