use super::*;
use crate::wire::provider::openai::OpenAi;

fn part(text: &str, marked: bool) -> Value {
    if marked {
        json!({"type": "text", "text": text, "prompt_cache_breakpoint": {}})
    } else {
        json!({"type": "text", "text": text})
    }
}

fn openai_call(seq: u64, start: i64, body: Value, outcome: UsageOutcome) -> CallRecord {
    let mut record = call(seq, start, start + 10, vec![], outcome);
    record.path = "/v1/chat/completions".into();
    record.request_body = RequestBody::Json(body);
    record
}

fn chat_body(parts: Vec<Value>, explicit: bool) -> Value {
    let mut body = json!({
        "model": "gpt-6.1-sol",
        "messages": [{"role": "user", "content": parts}],
    });
    if explicit {
        body["prompt_cache_options"] = json!({"mode": "explicit"});
    }
    body
}

#[test]
fn the_writer_decides_the_cache_mode() {
    let a = openai_call(
        1,
        0,
        chat_body(
            vec![part("p0", true), part("p1", false), part("p2", false)],
            true,
        ),
        written(0, 3_000),
    );
    let b = openai_call(
        2,
        20,
        chat_body(
            vec![part("p0", false), part("p1", false), part("q2", false)],
            false,
        ),
        usage(3_000, 0),
    );
    let view = diff_view(&DiffInput {
        a: Some(&a),
        b: &b,
        session: &[a.clone(), b.clone()],
        provider: &OpenAi,
        base_picked: false,
    });
    assert_eq!(view.shared_segments, 2);
    assert_eq!(view.explanation.verdict, Verdict::NoSharedBreakpoint);
}
