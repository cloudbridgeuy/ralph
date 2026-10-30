use super::*;
use crate::wire::price::PriceOverride;
use crate::wire::provider::{provider, ProviderId};
use serde_json::{json, Value};

fn with_pricing<T>(overrides: &BTreeMap<String, PriceOverride>, f: impl Fn(&Pricing) -> T) -> T {
    let provider = provider(ProviderId::Anthropic);
    f(&Pricing {
        provider: provider.as_ref(),
        overrides,
    })
}

fn rows_of(usage: &str) -> Vec<CostRow> {
    with_pricing(&BTreeMap::new(), |pricing| cost_rows(usage, pricing))
}

fn view_of(usage: &str) -> String {
    with_pricing(&BTreeMap::new(), |pricing| costs_view(usage, pricing))
}

fn jsonl(rows: &[Value]) -> String {
    rows.iter().map(|row| format!("{row}\n")).collect()
}

fn turn(run: &str, process_turn: u64, cost: f64) -> Value {
    json!({
        "kind": "turn", "trigger": "user", "run": run, "process_turn": process_turn,
        "cost_usd": cost, "input_tokens": 10, "output_tokens": 5,
        "cache_creation_input_tokens": 100, "cache_read_input_tokens": 1000,
    })
}

fn worker(id: &str, session: &str, model: &str) -> Value {
    json!({
        "kind": "worker", "worker_id": id, "turn_id": 1, "harness": "claude",
        "model": model, "session_id": session, "cost_usd": null,
        "input_tokens": 3, "output_tokens": 7, "cache_creation_input_tokens": 50,
        "cache_read_input_tokens": 400, "cache_write_5m": 20, "cache_write_1h": 30,
    })
}

fn nap(model: &str, cost: f64) -> Value {
    json!({
        "kind": "nap", "harness": "claude", "model": model, "cost_usd": cost,
        "input_tokens": 1, "output_tokens": 2,
        "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0,
    })
}

fn find<'a>(rows: &'a [CostRow], harness: &str, model: &str) -> &'a CostRow {
    rows.iter()
        .find(|row| row.harness == harness && row.model == model)
        .unwrap()
}

fn close(usd: Usd, expected: f64) -> bool {
    usd.known
        .is_some_and(|known| (known - expected).abs() < 1e-9)
}

#[test]
fn groups_by_harness_and_model_with_main_and_nap_first() {
    let usage = jsonl(&[
        worker("GUZ-1", "s1", "claude-opus-5-5"),
        nap("claude-haiku-5-5", 0.01),
        turn("r1", 1, 0.5),
        worker("GUZ-2", "s2", "claude-sonnet-5-5"),
    ]);
    let keys: Vec<(String, String)> = rows_of(&usage)
        .into_iter()
        .map(|row| (row.harness, row.model))
        .collect();
    assert_eq!(
        keys,
        [
            ("main".into(), "unknown".into()),
            ("nap".into(), "claude-haiku-5-5".into()),
            ("claude".into(), "claude-opus-5-5".into()),
            ("claude".into(), "claude-sonnet-5-5".into()),
        ]
    );
}

#[test]
fn main_rows_split_by_model() {
    let mut opus = turn("r1", 1, 0.5);
    opus["model"] = json!("claude-opus-5-5");
    let rows = rows_of(&jsonl(&[opus, turn("r2", 1, 0.25)]));
    assert!(close(find(&rows, "main", "claude-opus-5-5").usd, 0.5));
    assert!(close(find(&rows, "main", "unknown").usd, 0.25));
}

#[test]
fn main_cost_takes_the_last_running_total_per_run() {
    let rows = rows_of(&jsonl(&[
        turn("r1", 1, 0.1),
        turn("r1", 2, 0.3),
        turn("r2", 1, 0.2),
    ]));
    let main = find(&rows, "main", "unknown");
    assert_eq!(main.runs, Some(2));
    assert!(close(main.usd, 0.5));
    assert!(!main.usd.missing);
}

#[test]
fn main_cost_marks_a_run_with_an_uncosted_process() {
    let bare = json!({"kind": "turn", "run": "r1", "process_turn": 1});
    let rows = rows_of(&jsonl(&[turn("r1", 1, 0.1), bare]));
    let main = find(&rows, "main", "unknown");
    assert!(close(main.usd, 0.1));
    assert!(main.usd.missing);
}

#[test]
fn main_cost_sums_every_process_of_a_run() {
    let rows = rows_of(&jsonl(&[
        turn("r1", 1, 0.1),
        turn("r1", 2, 0.3),
        turn("r1", 1, 0.05),
        turn("r1", 2, 0.15),
    ]));
    let main = find(&rows, "main", "unknown");
    assert_eq!(main.runs, Some(1));
    assert!(close(main.usd, 0.45));
}

#[test]
fn pings_count_toward_the_main_cost() {
    let ping = json!({
        "kind": "ping", "run": "r1", "cost_usd": 0.4, "input_tokens": 0,
        "output_tokens": 0, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 9,
    });
    let rows = rows_of(&jsonl(&[turn("r1", 1, 0.3), ping]));
    let main = find(&rows, "main", "unknown");
    assert!(close(main.usd, 0.4));
    assert_eq!(main.tokens.cache_read, 1009);
}

#[test]
fn a_reused_worker_id_counts_each_session() {
    let rows = rows_of(&jsonl(&[
        worker("GUZ-1", "s1", "claude-opus-5-5"),
        worker("GUZ-1", "s1", "claude-opus-5-5"),
        worker("GUZ-1", "s2", "claude-opus-5-5"),
    ]));
    let claude = find(&rows, "claude", "claude-opus-5-5");
    assert_eq!(claude.runs, Some(2));
    assert_eq!(claude.tokens.write_5m, Some(60));
    assert_eq!(claude.tokens.write_1h, Some(90));
    assert_eq!(claude.tokens.output, 21);
}

#[test]
fn worker_rows_are_priced_from_tokens() {
    let rows = rows_of(&jsonl(&[worker("GUZ-1", "s1", "claude-opus-5-5")]));
    assert!(close(
        find(&rows, "claude", "claude-opus-5-5").usd,
        0.000572
    ));
}

#[test]
fn unknown_model_shows_no_price() {
    let rows = rows_of(&jsonl(&[worker("GUZ-1", "s1", "gpt-9")]));
    assert_eq!(find(&rows, "claude", "gpt-9").usd.text(), "n/a");
}

#[test]
fn a_provider_prefixed_opencode_row_with_no_cost_shows_no_price() {
    let mut line = worker("GUZ-1", "s1", "github-copilot/gpt-6.1-sol");
    line["harness"] = json!("opencode");
    let rows = rows_of(&jsonl(&[line]));
    assert_eq!(
        find(&rows, "opencode", "github-copilot/gpt-6.1-sol")
            .usd
            .text(),
        "n/a"
    );
}

#[test]
fn a_cache_write_tier_gap_prices_the_rest() {
    let mut gap = worker("GUZ-1", "s1", "claude-opus-5-5");
    gap["cache_creation_input_tokens"] = json!(60);
    let rows = rows_of(&jsonl(&[gap]));
    let usd = find(&rows, "claude", "claude-opus-5-5").usd;
    assert!(close(usd, 0.000572));
    assert!(usd.missing);
}

fn haiku_row(input: u64, flag: Option<bool>) -> Value {
    let mut row = json!({
        "kind": "worker", "worker_id": "GUZ-1", "turn_id": 1, "harness": "claude",
        "model": "claude-haiku-5-5", "session_id": "s1", "cost_usd": null,
        "input_tokens": input, "output_tokens": 0,
        "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0,
    });
    if let Some(flag) = flag {
        row["long_prompt"] = json!(flag);
    }
    row
}

fn haiku_usd(row: Value) -> f64 {
    let rows = rows_of(&jsonl(&[row]));
    find(&rows, "claude", "claude-haiku-5-5").usd.known.unwrap()
}

#[test]
fn a_short_flag_prices_a_large_summed_row_at_the_base_tier() {
    assert!((haiku_usd(haiku_row(300_000, Some(false))) - 0.03).abs() < 1e-9);
}

#[test]
fn a_long_flag_prices_a_small_row_at_the_long_tier() {
    assert!((haiku_usd(haiku_row(10_000, Some(true))) - 0.005).abs() < 1e-9);
}

#[test]
fn a_row_without_the_flag_prices_from_its_sum() {
    assert!((haiku_usd(haiku_row(300_000, None)) - 0.15).abs() < 1e-9);
    assert!((haiku_usd(haiku_row(10_000, None)) - 0.001).abs() < 1e-9);
}

#[test]
fn tier_prompt_follows_the_flag_or_the_sum() {
    assert_eq!(tier_prompt(Some(true), 5), u64::MAX);
    assert_eq!(tier_prompt(Some(false), 5_000_000), 0);
    assert_eq!(tier_prompt(None, 5), 5);
}

#[test]
fn long_prompt_call_uses_the_builtin_table() {
    let call = |model: &str, input: u64| {
        long_prompt_call(
            model,
            &Usage {
                input_uncached: input,
                cache_read: 0,
                cache_writes: Vec::new(),
                output: 0,
                reasoning: None,
                other: Vec::new(),
            },
        )
    };
    assert!(!call("claude-haiku-5-5", 100_000));
    assert!(call("claude-haiku-5-5", 100_001));
    assert!(!call("gpt-6.1-sol", 1_000_000));
}

#[test]
fn an_override_wins_over_the_table() {
    let overrides = BTreeMap::from([(
        "claude-opus-5-5".to_string(),
        PriceOverride {
            input: 1000.0,
            cache_read: Some(0.0),
            cache_write_5m: Some(0.0),
            cache_write_1h: Some(0.0),
            output: Some(0.0),
        },
    )]);
    let usage = jsonl(&[worker("GUZ-1", "s1", "claude-opus-5-5")]);
    let rows = with_pricing(&overrides, |pricing| cost_rows(&usage, pricing));
    assert!(close(find(&rows, "claude", "claude-opus-5-5").usd, 0.003));
}

fn codex(model: &str) -> Value {
    json!({
        "kind": "worker", "worker_id": "GUZ-2", "turn_id": 1, "harness": "codex",
        "model": model, "session_id": "th1", "cost_usd": null,
        "input_tokens": 1_000_000, "output_tokens": 500_000, "reasoning_tokens": 400_000,
        "cache_creation_input_tokens": 0, "cache_read_input_tokens": 2_000_000,
    })
}

#[test]
fn codex_rows_price_cached_input_and_skip_reasoning() {
    let rows = rows_of(&jsonl(&[codex("gpt-6.1-sol")]));
    assert!(close(find(&rows, "codex", "gpt-6.1-sol").usd, 7.2));
}

#[test]
fn codex_luna_rows_use_the_luna_price() {
    let rows = rows_of(&jsonl(&[codex("gpt-6-luna")]));
    assert!(close(find(&rows, "codex", "gpt-6-luna").usd, 0.37));
}

#[test]
fn codex_cache_writes_without_a_tier_price_at_the_30m_rate() {
    let mut row = codex("gpt-6.1-sol");
    row["cache_creation_input_tokens"] = json!(1_000_000);
    let rows = rows_of(&jsonl(&[row, codex("gpt-6.1-sol")]));
    let usd = find(&rows, "codex", "gpt-6.1-sol").usd;
    assert!(close(usd, 16.9));
    assert_eq!(usd.text(), "16.90");
}

#[test]
fn a_gpt_override_without_a_30m_price_keeps_the_plus() {
    let overrides = BTreeMap::from([(
        "gpt-6.1-sol".to_string(),
        PriceOverride {
            input: 1.0,
            cache_read: Some(0.0),
            cache_write_5m: None,
            cache_write_1h: None,
            output: Some(0.0),
        },
    )]);
    let mut row = codex("gpt-6.1-sol");
    row["cache_creation_input_tokens"] = json!(10);
    let usage = jsonl(&[row]);
    let rows = with_pricing(&overrides, |pricing| cost_rows(&usage, pricing));
    assert_eq!(find(&rows, "codex", "gpt-6.1-sol").usd.text(), "1.00+");
}

#[test]
fn an_override_wins_over_the_openai_table() {
    let overrides = BTreeMap::from([(
        "gpt-6.1-sol".to_string(),
        PriceOverride {
            input: 1.0,
            cache_read: Some(0.0),
            cache_write_5m: None,
            cache_write_1h: None,
            output: Some(0.0),
        },
    )]);
    let usage = jsonl(&[codex("gpt-6.1-sol")]);
    let rows = with_pricing(&overrides, |pricing| cost_rows(&usage, pricing));
    assert!(close(find(&rows, "codex", "gpt-6.1-sol").usd, 1.0));
}

#[test]
fn nap_runs_count_rows_and_sum_cost() {
    let rows = rows_of(&jsonl(&[
        nap("claude-haiku-5-5", 0.01),
        nap("claude-haiku-5-5", 0.02),
    ]));
    let nap = find(&rows, "nap", "claude-haiku-5-5");
    assert_eq!(nap.runs, Some(2));
    assert_eq!(nap.usd.text(), "0.03");
}

#[test]
fn skips_unknown_kinds_and_bad_lines() {
    let usage = format!("not json\n{}\n", json!({"kind": "other"}));
    assert_eq!(view_of(&usage), NO_USAGE);
    assert_eq!(view_of(""), NO_USAGE);
}

#[test]
fn table_totals_main_and_computed_worker_cost() {
    let mut heavy = worker("GUZ-1", "s1", "claude-opus-5-5");
    heavy["output_tokens"] = json!(30_000);
    let view = view_of(&jsonl(&[turn("r1", 1, 1.5), heavy]));
    let lines: Vec<&str> = view.lines().collect();
    assert_eq!(lines.len(), 5);
    assert!(lines[0].starts_with("| HARNESS | MODEL | RUNS |"));
    assert_eq!(
        lines[2],
        "| main | unknown | 1 | 10 | 5 | 1.0k | 100 | – | – | – | 1.50 |"
    );
    assert_eq!(
        lines[3],
        "| claude | claude-opus-5-5 | 1 | 3 | 30.0k | 400 | 50 | 20 | 30 | – | 0.60 |"
    );
    assert_eq!(
        lines[4],
        "| total |  |  | 13 | 30.0k | 1.4k | 150 | 20 | 30 | – | 2.10 |"
    );
}

#[test]
fn counts_scale_to_one_decimal() {
    assert_eq!(count(999), "999");
    assert_eq!(count(1_250), "1.2k");
    assert_eq!(count(3_100_000), "3.1M");
    assert_eq!(count(2_000_000_000), "2.0B");
}

fn with_model(mut row: Value, model: &str) -> Value {
    row["model"] = json!(model);
    row
}

#[test]
fn one_run_with_a_late_model_counts_once() {
    let rows = rows_of(&jsonl(&[
        turn("r1", 1, 0.1),
        with_model(turn("r1", 2, 0.5), "claude-opus-5-5"),
    ]));
    let mains: Vec<&CostRow> = rows.iter().filter(|row| row.harness == "main").collect();
    assert_eq!(mains.len(), 1);
    assert_eq!(mains[0].model, "claude-opus-5-5");
    assert_eq!(mains[0].runs, Some(1));
    assert!(close(mains[0].usd, 0.5));
    assert!(close(total_row(&rows).usd, 0.5));
}

#[test]
fn relaunched_processes_cost_in_their_own_model_group() {
    let rows = rows_of(&jsonl(&[
        with_model(turn("r1", 1, 0.2), "claude-opus-5-5"),
        with_model(turn("r1", 2, 0.4), "claude-opus-5-5"),
        turn("r1", 1, 0.05),
        with_model(turn("r1", 2, 0.15), "claude-sonnet-5-5"),
    ]));
    let opus = find(&rows, "main", "claude-opus-5-5");
    let sonnet = find(&rows, "main", "claude-sonnet-5-5");
    assert_eq!(opus.runs, Some(1));
    assert_eq!(sonnet.runs, Some(1));
    assert!(close(opus.usd, 0.4));
    assert!(close(sonnet.usd, 0.15));
    assert!(close(total_row(&rows).usd, 0.55));
}
