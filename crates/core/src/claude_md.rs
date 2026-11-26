use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 5;
const FILE_NAME: &str = "CLAUDE.md";

pub struct Loaded {
    pub path: PathBuf,
    pub text: String,
}

pub struct Layout<'a> {
    pub git_root: &'a Path,
    pub cwd: &'a Path,
    pub user_dir: Option<&'a Path>,
    pub project: bool,
}

pub fn git_root(cwd: &Path, is_root: &impl Fn(&Path) -> bool) -> PathBuf {
    cwd.ancestors()
        .find(|dir| is_root(dir))
        .unwrap_or(cwd)
        .to_path_buf()
}

pub fn claude_md_paths(layout: &Layout) -> Vec<PathBuf> {
    let user = layout.user_dir.map(|dir| dir.join(FILE_NAME));
    let mut dirs: Vec<&Path> = layout
        .cwd
        .ancestors()
        .take_while(|dir| dir.starts_with(layout.git_root))
        .filter(|_| layout.project)
        .collect();
    dirs.reverse();
    let project = dirs.into_iter().map(|dir| dir.join(FILE_NAME));
    user.into_iter().chain(project).collect()
}

pub fn inline_claude_md(
    paths: &[PathBuf],
    home: Option<&Path>,
    read: &impl Fn(&Path) -> Option<Loaded>,
) -> String {
    paths
        .iter()
        .filter_map(|path| read(path))
        .map(|file| {
            let body = expand(&file, 0, &mut Vec::new(), home, read);
            format!(
                "Contents of {}:\n\n{}",
                file.path.display(),
                body.trim_end()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn append_claude_md(system: &str, claude_md: &str) -> String {
    if claude_md.is_empty() {
        system.to_string()
    } else {
        format!("{system}\n\n{claude_md}")
    }
}

fn resolve(token: &str, dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let path = token.strip_prefix('@').filter(|path| !path.is_empty())?;
    if let Some(rest) = path.strip_prefix("~/") {
        return home.map(|home| home.join(rest));
    }
    let path = Path::new(path);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        dir.join(path)
    })
}

fn expand(
    file: &Loaded,
    depth: usize,
    stack: &mut Vec<PathBuf>,
    home: Option<&Path>,
    read: &impl Fn(&Path) -> Option<Loaded>,
) -> String {
    stack.push(file.path.clone());
    let dir = file.path.parent().unwrap_or(Path::new(""));
    let mut fenced = false;
    let mut out = Vec::new();
    for line in file.text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let token = line.trim();
        let import = (!fenced && !token.contains(char::is_whitespace))
            .then(|| resolve(token, dir, home))
            .flatten();
        match import {
            None => out.push(line.to_string()),
            Some(path) => out.extend(import_text(&path, depth, stack, home, read)),
        }
    }
    stack.pop();
    out.join("\n")
}

fn import_text(
    path: &Path,
    depth: usize,
    stack: &mut Vec<PathBuf>,
    home: Option<&Path>,
    read: &impl Fn(&Path) -> Option<Loaded>,
) -> Option<String> {
    if depth >= MAX_DEPTH {
        return None;
    }
    let loaded = read(path).filter(|loaded| !stack.contains(&loaded.path))?;
    Some(expand(&loaded, depth + 1, stack, home, read))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn files(entries: &[(&str, &str)]) -> HashMap<PathBuf, String> {
        entries
            .iter()
            .map(|(path, text)| (PathBuf::from(path), text.to_string()))
            .collect()
    }

    fn run(map: &HashMap<PathBuf, String>, paths: &[&str]) -> String {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        inline_claude_md(&paths, Some(Path::new("/home/u")), &|path| {
            map.get(path).map(|text| Loaded {
                path: path.to_path_buf(),
                text: text.clone(),
            })
        })
    }

    fn layout<'a>(project: bool, user_dir: Option<&'a Path>) -> Layout<'a> {
        Layout {
            git_root: Path::new("/r"),
            cwd: Path::new("/r/a/b"),
            user_dir,
            project,
        }
    }

    #[test]
    fn a_single_file_gets_a_labelled_section() {
        let map = files(&[("/r/CLAUDE.md", "rules")]);
        assert_eq!(
            run(&map, &["/r/CLAUDE.md"]),
            "Contents of /r/CLAUDE.md:\n\nrules"
        );
    }

    #[test]
    fn a_nested_import_replaces_its_line() {
        let map = files(&[
            ("/r/CLAUDE.md", "top\n@a.md\nend"),
            ("/r/a.md", "A\n@b.md"),
            ("/r/b.md", "B"),
        ]);
        assert_eq!(
            run(&map, &["/r/CLAUDE.md"]),
            "Contents of /r/CLAUDE.md:\n\ntop\nA\nB\nend"
        );
    }

    #[test]
    fn paths_resolve_relative_home_and_absolute() {
        let map = files(&[
            ("/r/docs/CLAUDE.md", "@x.md\n@~/h.md\n@/abs/z.md"),
            ("/r/docs/x.md", "X"),
            ("/home/u/h.md", "H"),
            ("/abs/z.md", "Z"),
        ]);
        assert!(run(&map, &["/r/docs/CLAUDE.md"]).ends_with(":\n\nX\nH\nZ"));
    }

    #[test]
    fn the_sixth_hop_is_not_read() {
        let map = files(&[
            ("/r/f0.md", "@f1.md"),
            ("/r/f1.md", "@f2.md"),
            ("/r/f2.md", "@f3.md"),
            ("/r/f3.md", "@f4.md"),
            ("/r/f4.md", "@f5.md"),
            ("/r/f5.md", "five\n@f6.md"),
            ("/r/f6.md", "six"),
        ]);
        let text = run(&map, &["/r/f0.md"]);
        assert!(text.contains("five"));
        assert!(!text.contains("six"));
    }

    #[test]
    fn a_cycle_includes_each_file_once_and_ends() {
        let map = files(&[("/r/a.md", "A\n@b.md"), ("/r/b.md", "B\n@a.md")]);
        assert_eq!(run(&map, &["/r/a.md"]), "Contents of /r/a.md:\n\nA\nB");
    }

    #[test]
    fn a_missing_import_is_skipped() {
        let map = files(&[("/r/CLAUDE.md", "a\n@gone.md\nb")]);
        assert_eq!(
            run(&map, &["/r/CLAUDE.md"]),
            "Contents of /r/CLAUDE.md:\n\na\nb"
        );
    }

    #[test]
    fn an_import_inside_a_code_fence_stays_text() {
        let map = files(&[("/r/CLAUDE.md", "```\n@a.md\n```"), ("/r/a.md", "A")]);
        assert!(run(&map, &["/r/CLAUDE.md"]).contains("@a.md"));
    }

    #[test]
    fn the_user_file_comes_first_then_root_to_cwd() {
        let paths = claude_md_paths(&layout(true, Some(Path::new("/h/.claude"))));
        let expected: Vec<PathBuf> = [
            "/h/.claude/CLAUDE.md",
            "/r/CLAUDE.md",
            "/r/a/CLAUDE.md",
            "/r/a/b/CLAUDE.md",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(paths, expected);
    }

    #[test]
    fn project_off_leaves_only_the_user_file() {
        let paths = claude_md_paths(&layout(false, Some(Path::new("/h/.claude"))));
        assert_eq!(paths, vec![PathBuf::from("/h/.claude/CLAUDE.md")]);
    }

    #[test]
    fn both_flags_off_inline_nothing() {
        let paths = claude_md_paths(&layout(false, None));
        assert!(paths.is_empty());
        assert_eq!(run(&HashMap::new(), &[]), "");
    }

    #[test]
    fn empty_claude_md_leaves_the_system_unchanged() {
        assert_eq!(append_claude_md("rules", ""), "rules");
        assert_eq!(append_claude_md("rules", "md"), "rules\n\nmd");
    }

    #[test]
    fn git_root_is_the_nearest_ancestor_or_the_cwd() {
        let is_root = |dir: &Path| dir == Path::new("/r");
        assert_eq!(git_root(Path::new("/r/a/b"), &is_root), PathBuf::from("/r"));
        assert_eq!(git_root(Path::new("/x/y"), &is_root), PathBuf::from("/x/y"));
    }
}
