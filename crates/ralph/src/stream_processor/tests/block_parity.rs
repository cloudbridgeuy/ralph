use std::path::Path;

use serde_json::{json, Value};

use crate::stream_processor::render_block::render_block;
use crate::stream_processor::{StreamProcessor, VerboseToolsConfig};

enum Step {
    Line(String),
    Write(String, String),
}

fn say(id: &str, content: Value) -> Step {
    Step::Line(json!({"type":"assistant","message":{"id":id,"content":content}}).to_string())
}

fn text(id: &str, body: &str) -> Step {
    say(id, json!([{"type":"text","text":body}]))
}

fn call(id: &str, name: &str, input: Value) -> Step {
    say(
        &format!("m_{id}"),
        json!([{"type":"tool_use","id":id,"name":name,"input":input}]),
    )
}

fn reply(id: &str, content: Option<&str>, is_error: bool) -> Step {
    let mut result = json!({"type":"tool_result","tool_use_id":id,"is_error":is_error});
    if let Some(content) = content {
        result["content"] = json!(content);
    }
    Step::Line(json!({"type":"user","message":{"content":[result]}}).to_string())
}

fn write(path: &Path, content: &str) -> Step {
    Step::Write(path.display().to_string(), content.to_string())
}

fn assert_parity(steps: &[Step], verbose: &VerboseToolsConfig, highlight: bool) {
    let mut processor = StreamProcessor::with_options_and_verbose(highlight, true, verbose.clone());
    for step in steps {
        match step {
            Step::Write(path, content) => std::fs::write(path, content).unwrap(),
            Step::Line(line) => {
                let before = processor.output_blocks().len();
                let shown = processor.process_line(line).unwrap_or_default();
                let markdown = highlight.then_some(&processor.renderer.markdown);
                let ctx = processor.render_context();
                let rendered: String = processor.output_blocks()[before..]
                    .iter()
                    .map(|block| render_block(&ctx, block, markdown))
                    .collect();
                assert_eq!(
                    shown, rendered,
                    "line {line} verbose={verbose:?} highlight={highlight}"
                );
            }
        }
    }
    let before = processor.output_blocks().len();
    let markdown_theme = processor.renderer.markdown.clone();
    let highlighter = processor.renderer.code_highlighter.clone();
    let (diff, tools) = (
        processor.renderer.diff_colors,
        processor.renderer.tool_colors,
    );
    let (result, final_output) = processor.finish_rendered();
    let ctx = crate::render::RenderContext::new(&highlighter, highlight)
        .with_diff(diff)
        .with_tools(tools);
    let rendered: String = result.output_blocks[before..]
        .iter()
        .map(|block| render_block(&ctx, block, highlight.then_some(&markdown_theme)))
        .collect();
    assert_eq!(final_output, rendered);
}

fn assert_parity_everywhere(steps: &[Step]) {
    for verbose in [VerboseToolsConfig::new(), VerboseToolsConfig::all()] {
        for highlight in [false, true] {
            assert_parity(steps, &verbose, highlight);
        }
    }
}

fn many_lines(count: usize) -> String {
    (1..=count).map(|n| format!("line {n}\n")).collect()
}

#[test]
fn text_chunks_render_the_same_from_their_blocks() {
    assert_parity_everywhere(&[
        text(
            "m1",
            "# Title\n\n- one\n- two\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n",
        ),
        text("m1", "```rust\nfn main() {}\n```\n"),
        text("m1", "```diff\n-a\n+b\n```\nafter\n"),
        text("m2", "second reply\n"),
        text("m3", "tail without newline"),
    ]);
}

#[test]
fn bash_calls_and_results_render_the_same_from_their_blocks() {
    let long = many_lines(40);
    assert_parity_everywhere(&[
        call(
            "b1",
            "Bash",
            json!({"command":"ls -la","description":"List"}),
        ),
        reply("b1", Some("a.txt\nb.txt"), false),
        call("b2", "Bash", json!({"command":"seq 40"})),
        reply("b2", Some(&long), false),
        call("b3", "Bash", json!({"command":"false"})),
        reply("b3", Some("boom"), true),
        call("b4", "Bash", json!({"command":"true"})),
        reply("b4", None, false),
        call("b5", "Bash", json!({"command":"cat big"})),
        reply("b5", Some("x\n... (truncated)"), false),
    ]);
}

#[test]
fn search_and_read_tools_render_the_same_from_their_blocks() {
    assert_parity_everywhere(&[
        call(
            "g1",
            "Grep",
            json!({"pattern":"fn \\w+","path":"src","output_mode":"content","-i":true}),
        ),
        reply("g1", Some("src/a.rs:1:fn main\nsrc/b.rs:2:fn x"), false),
        call("g2", "Grep", json!({"pattern":"x"})),
        reply("g2", Some("no such dir"), true),
        call("g3", "Grep", json!({"pattern":"y"})),
        reply("g3", None, true),
        call(
            "r1",
            "Read",
            json!({"file_path":"src/main.rs","offset":2,"limit":5}),
        ),
        reply("r1", Some("     1→fn main() {}\n     2→"), false),
        call("r2", "Read", json!({})),
        reply("r2", Some("missing"), true),
        call("o1", "Glob", json!({"pattern":"**/*.rs","path":"src"})),
        reply("o1", Some("src/a.rs\nsrc/b/c.rs\n... (truncated)"), false),
        call("o2", "Glob", json!({})),
        reply("o2", None, true),
    ]);
}

#[test]
fn todo_and_other_tools_render_the_same_from_their_blocks() {
    let prompt = "a long prompt ".repeat(10);
    assert_parity_everywhere(&[
        call(
            "t1",
            "TodoWrite",
            json!({"todos":[{"content":"a","status":"completed","activeForm":"A"},{"content":"b"}]}),
        ),
        reply("t1", Some("Todos have been modified"), false),
        call("t2", "TodoWrite", json!({"todos":[]})),
        reply("t2", Some(&"bad ".repeat(80)), true),
        call("k1", "Task", json!({"prompt":prompt})),
        reply("k1", Some(&"done\n".repeat(60)), false),
        call("k2", "WebFetch", json!({"url":"https://example.com/a"})),
        reply("k2", None, false),
        call("k3", "Mystery", json!({"n":1})),
        reply("k3", Some("odd"), true),
        reply("unknown", Some("orphan"), false),
    ]);
}

#[test]
fn edit_results_render_the_same_from_their_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.rs");
    let path = file.display().to_string();
    let long_old = many_lines(35);
    assert_parity_everywhere(&[
        write(&file, "fn a() {}\n"),
        call(
            "e1",
            "Edit",
            json!({"file_path":path,"old_string":"fn a() {}","new_string":"fn b() {}"}),
        ),
        write(&file, "fn b() {}\n"),
        reply("e1", Some("The file was updated"), false),
        call(
            "e2",
            "Edit",
            json!({"file_path":path,"old_string":"same","new_string":"same"}),
        ),
        reply("e2", Some("ok"), false),
        call(
            "e3",
            "Edit",
            json!({"file_path":path,"old_string":long_old,"new_string":""}),
        ),
        reply("e3", Some("ok"), false),
        call("e4", "Edit", json!({"file_path":path})),
        write(&file, "fn c() {}\n"),
        reply("e4", Some("ok"), false),
        call("e5", "Edit", json!({"file_path":path})),
        reply("e5", Some("ok"), false),
        call(
            "e6",
            "Edit",
            json!({"file_path":path,"old_string":"a","new_string":"b"}),
        ),
        reply("e6", Some("-a\n+b"), false),
        call("e7", "Edit", json!({"old_string":"a"})),
        reply("e7", Some("-a\n+b"), false),
        call(
            "e8",
            "Edit",
            json!({"file_path":path,"old_string":"a","new_string":"b"}),
        ),
        reply("e8", Some("no match"), true),
        call(
            "e9",
            "Edit",
            json!({"file_path":dir.path().join("gone.rs").display().to_string()}),
        ),
        reply("e9", Some("ok"), false),
    ]);
}

#[test]
fn write_results_render_the_same_from_their_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = dir.path().join("new.py");
    let empty = dir.path().join("empty.txt");
    let old = dir.path().join("old.rs");
    let path = |p: &Path| p.display().to_string();
    assert_parity_everywhere(&[
        Step::Write(path(&old), "a\n".to_string()),
        call(
            "w1",
            "Write",
            json!({"file_path":path(&fresh),"content":"x"}),
        ),
        write(&fresh, &many_lines(55)),
        reply("w1", Some("created"), false),
        call(
            "w2",
            "Write",
            json!({"file_path":path(&empty),"content":""}),
        ),
        write(&empty, ""),
        reply("w2", Some("created"), false),
        call("w3", "Write", json!({"file_path":path(&old),"content":"b"})),
        write(&old, "b\n"),
        reply("w3", Some("updated"), false),
        call("w4", "Write", json!({"file_path":path(&old),"content":"b"})),
        reply("w4", Some("updated"), false),
        call(
            "w5",
            "Write",
            json!({"file_path":path(&dir.path().join("never.rs")),"content":"z"}),
        ),
        reply("w5", Some("ok"), false),
        call("w6", "Write", json!({"file_path":path(&old),"content":"c"})),
        reply("w6", Some("denied"), true),
    ]);
    std::fs::remove_file(&fresh).unwrap();
    std::fs::remove_file(&empty).unwrap();
    std::fs::write(&old, "a\n").unwrap();
}

fn notebook(cells: &[&str]) -> String {
    let cells: Vec<Value> = cells
        .iter()
        .enumerate()
        .map(|(n, source)| json!({"id":format!("c{n}"),"cell_type":"code","source":source}))
        .collect();
    json!({"cells":cells}).to_string()
}

#[test]
fn notebook_results_render_the_same_from_their_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("n.ipynb");
    let path = file.display().to_string();
    assert_parity_everywhere(&[
        write(&file, &notebook(&["x = 1\n", "y = 2\n"])),
        call(
            "n1",
            "NotebookEdit",
            json!({"notebook_path":path,"cell_id":"c0","new_source":"x = 3\n","cell_type":"code"}),
        ),
        write(&file, &notebook(&["x = 3\n", "y = 2\n"])),
        reply("n1", Some("updated"), false),
        call(
            "n2",
            "NotebookEdit",
            json!({"notebook_path":path,"cell_number":1,"edit_mode":"delete"}),
        ),
        write(&file, &notebook(&["x = 3\n"])),
        reply("n2", Some("deleted"), false),
        call(
            "n3",
            "NotebookEdit",
            json!({"notebook_path":path,"cell_id":"c0","new_source":"x = 3\n","cell_type":"markdown","edit_mode":"insert"}),
        ),
        reply("n3", Some("same"), false),
        call(
            "n4",
            "NotebookEdit",
            json!({"notebook_path":path,"cell_id":"c9","new_source":"q"}),
        ),
        reply("n4", Some("ok"), false),
        call(
            "n5",
            "NotebookEdit",
            json!({"notebook_path":path,"cell_id":"c0","new_source":"q"}),
        ),
        reply("n5", Some("bad cell"), true),
    ]);
}
