#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{Host, Reply, TestServer};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const FIXTURE: &str = include_str!("../../ralph_server/fixtures/config.toml");

fn write_config(dir: &Path, text: &str) {
    fs::write(dir.join("config.toml"), text).unwrap();
}

fn get(server: &TestServer) -> Reply {
    let target = format!("/api/config?token={}", server.token());
    server.exchange("GET", &target, Host::Own, &[])
}

fn json_of(reply: &Reply) -> Value {
    serde_json::from_slice(&reply.body_bytes).unwrap()
}

#[test]
fn config_200_shape_and_redaction() {
    let server = TestServer::start_with_store(|dir| write_config(dir, FIXTURE));
    let reply = get(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    let value = json_of(&reply);
    let name = server.store_path().file_name().unwrap().to_str().unwrap();
    assert_eq!(value["store"], name);
    let rows = value["rows"].as_array().unwrap();
    let theme = rows
        .iter()
        .find(|row| row["section"] == "theme" && row["key"] == "name")
        .unwrap();
    assert_eq!(theme["value"], "tokyo-night");
    assert_eq!(theme["redacted"], false);
    for row in rows {
        let keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for key in ["section", "key", "value", "source", "redacted"] {
            assert!(keys.contains(&key), "{key}");
        }
    }
    for leak in ["abc123", "/Users", server.store_path().to_str().unwrap()] {
        assert!(!reply.body.contains(leak), "{leak}");
    }
}

#[test]
fn config_global_rows_come_from_the_injected_global_path() {
    let server =
        TestServer::start_with_dirs(|_| {}, |dir| write_config(dir, "[chat]\nwrap_width = 70\n"));
    let value = json_of(&get(&server));
    let wrap = value["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["section"] == "chat" && row["key"] == "wrap_width")
        .unwrap();
    assert_eq!(wrap["source"], "global");
    assert_eq!(wrap["value"], 70);
}

#[test]
fn config_dotted_sensitive_price_key_is_absent_from_the_http_body() {
    let server = TestServer::start_with_store(|dir| {
        write_config(dir, "[wire.prices.\"/Users/alice/acme.corp\"]\ninput = 1\n")
    });
    let reply = get(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert!(!reply.body.contains("corp"), "{}", reply.body);
    assert!(!reply.body.contains("/Users"), "{}", reply.body);
    let masked = json_of(&reply)["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["section"] == "wire" && row["key"] == "[redacted-1]")
        .cloned()
        .expect(&reply.body);
    assert_eq!(masked["redacted"], true, "{}", reply.body);
    assert_eq!(masked["value"], "[redacted]", "{}", reply.body);
}

#[test]
fn config_windows_and_variable_path_keys_are_absent_from_the_http_body() {
    for (key, leaks) in [
        (r"C:\\Users\\alice\\acme", ["alice", "acme", "Users"]),
        (r"\\\\host\\share", ["host", "share", "host"]),
        ("D:/x", ["D:", "/x", "D:"]),
        ("%USERPROFILE%", ["USERPROFILE", "%", "USERPROFILE"]),
    ] {
        let server = TestServer::start_with_store(|dir| {
            write_config(dir, &format!("[wire.prices.\"{key}\"]\ninput = 1\n"))
        });
        let reply = get(&server);
        assert_eq!(reply.status, 200, "{}", reply.body);
        for leak in leaks {
            assert!(!reply.body.contains(leak), "{key}: {}", reply.body);
        }
        let masked = json_of(&reply)["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["section"] == "wire" && row["key"] == "[redacted-1]")
            .cloned()
            .expect(&reply.body);
        assert_eq!(masked["redacted"], true, "{}", reply.body);
        assert_eq!(masked["value"], "[redacted]", "{}", reply.body);
    }
}

#[test]
fn config_bare_secrets_in_tight_fields_are_absent_from_the_http_body() {
    for (config, leak) in [
        ("[mermaid.style.colors]\nx = \"ghp_SECRET\"\n", "ghp_SECRET"),
        (
            "[mermaid.style.config]\nnote = \"correct horse staple\"\n",
            "horse",
        ),
        ("[tickets]\nlinear = [\"my secret ticket\"]\n", "secret"),
        ("[wire.prices.\"hunter2 secret\"]\ninput = 1\n", "hunter2"),
    ] {
        let server = TestServer::start_with_store(|dir| write_config(dir, config));
        let reply = get(&server);
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert!(!reply.body.contains(leak), "{config}: {}", reply.body);
        let hidden = json_of(&reply)["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["redacted"] == true && row["value"] == "[redacted]");
        assert!(hidden, "{config}: {}", reply.body);
    }
}

#[test]
fn config_opaque_secrets_in_loose_fields_are_absent_from_the_http_body() {
    for (config, leak) in [
        (
            concat!(
                "[claude]\nmodel = \"gh",
                "p_AbCdEf0123456789AbCdEf0123456789abcd\"\n"
            ),
            "AbCdEf",
        ),
        ("[chat]\nname = \"Bearer abc123\"\n", "abc123"),
        (
            concat!(
                "[chat]\nname = \"xo",
                "xb-123456789012-1234567890123-AbCdEf0123456789AbCdEf01\"\n"
            ),
            "AbCdEf",
        ),
        (
            "[web]\nfont = \"aGVsbG8gd29ybGQgc2VjcmV0MTI1\"\n",
            "aGVsbG8",
        ),
    ] {
        let server = TestServer::start_with_store(|dir| write_config(dir, config));
        let reply = get(&server);
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert!(!reply.body.contains(leak), "{config}: {}", reply.body);
    }
}

#[test]
fn config_a_dated_model_name_stays_visible_in_the_http_body() {
    let server = TestServer::start_with_store(|dir| {
        write_config(dir, "[claude]\nmodel = \"claude-opus-4-1-20250805\"\n")
    });
    let reply = get(&server);
    assert!(
        reply.body.contains("claude-opus-4-1-20250805"),
        "{}",
        reply.body
    );
}

#[test]
fn config_invalid_is_422_without_paths() {
    let server = TestServer::start_with_store(|dir| write_config(dir, "[chat\nwrap_width = 1\n"));
    let reply = get(&server);
    assert_eq!(reply.status, 422, "{}", reply.body);
    assert_eq!(json_of(&reply)["error"], "config invalid");
    assert!(!reply.body.contains(server.store_path().to_str().unwrap()));
}

#[test]
fn config_unreadable_is_500() {
    let server =
        TestServer::start_with_store(|dir| fs::create_dir(dir.join("config.toml")).unwrap());
    let reply = get(&server);
    assert_eq!(reply.status, 500, "{}", reply.body);
    assert_eq!(json_of(&reply), json!({"error": "config unavailable"}));
}

#[test]
fn config_missing_store_is_404() {
    let server = TestServer::start();
    fs::remove_dir_all(server.store_path()).unwrap();
    let reply = get(&server);
    assert_eq!(reply.status, 404, "{}", reply.body);
    assert_eq!(json_of(&reply), json!({"error": "store not found"}));
}

#[test]
fn config_post_is_405_with_allow() {
    let server = TestServer::start();
    let target = format!("/api/config?token={}", server.token());
    let reply = server.exchange("POST", &target, Host::Own, &[]);
    assert_eq!(reply.status, 405);
    assert!(reply
        .headers
        .iter()
        .any(|(name, value)| name == "Allow" && value == "GET"));
}

#[test]
fn config_without_token_is_401() {
    let server = TestServer::start();
    let reply = server.exchange("GET", "/api/config", Host::Own, &[]);
    assert_eq!(reply.status, 401);
    assert_eq!(json_of(&reply), json!({"error": "unauthorized"}));
}

#[test]
fn config_wrong_host_is_403() {
    let server = TestServer::start();
    let target = format!("/api/config?token={}", server.token());
    let reply = server.exchange("GET", &target, Host::Named("evil.test"), &[]);
    assert_eq!(reply.status, 403);
}

#[test]
fn config_free_text_map_keys_are_absent_from_the_http_body() {
    for (config, leak, section) in [
        (
            "[mermaid.style.colors]\n\"hunter2 secret\" = \"#fff\"\n",
            "hunter2",
            "mermaid",
        ),
        (
            "[claude.permissions]\n\"secret words\" = true\n",
            "secret words",
            "claude",
        ),
        (
            "[chat.reset_turns]\n\"secret words\" = 1\n",
            "secret words",
            "chat",
        ),
    ] {
        let server = TestServer::start_with_store(|dir| write_config(dir, config));
        let reply = get(&server);
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert!(!reply.body.contains(leak), "{}", reply.body);
        let masked = json_of(&reply)["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["section"] == section && row["key"] == "[redacted-1]")
            .unwrap()
            .clone();
        assert_eq!(masked["redacted"], true, "{}", reply.body);
        assert_eq!(masked["value"], "[redacted]", "{}", reply.body);
    }
}

#[test]
fn config_text_in_a_numeric_setting_is_absent_from_the_http_body() {
    for (config, leak) in [
        (
            "[chat]\nreset_turns = \"my password is hunter2\"\n",
            "hunter2",
        ),
        ("[chat]\nreset_tokens = [\"my secret words\"]\n", "secret"),
        (
            "[chat]\nworker_quiet_secs = \"my password is hunter2\"\n",
            "hunter2",
        ),
    ] {
        let server = TestServer::start_with_store(|dir| write_config(dir, config));
        let reply = get(&server);
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert!(!reply.body.contains(leak), "{config}: {}", reply.body);
        let hidden = json_of(&reply)["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["section"] == "chat" && row["redacted"] == true);
        assert!(hidden, "{config}: {}", reply.body);
    }
}

#[test]
fn config_punctuated_and_dashed_secrets_are_absent_from_the_http_body() {
    for text in [
        "AbCdEf0123456789xyzQ!",
        "#AbCdEf0123456789xyzQ",
        "x*AbCdEf0123456789xyzQ",
        "AbCdEf0123|456789xyzQAbCd",
        "AbCdEf0123\u{200B}456789xyzQAbCd",
        "9f86d081-884c7d65-9a2feaa0-c55ad015-a3bf4f1b",
    ] {
        for config in [
            format!("[chat]\nname = \"{text}\"\n"),
            format!("[tickets]\nlinear = [\"{text}\"]\n"),
        ] {
            let server = TestServer::start_with_store(|dir| write_config(dir, &config));
            let reply = get(&server);
            assert_eq!(reply.status, 200, "{}", reply.body);
            for leak in ["AbCdEf", "9f86d081", "884c7d65"] {
                assert!(!reply.body.contains(leak), "{config}: {}", reply.body);
            }
            let hidden = json_of(&reply)["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["redacted"] == true && row["value"] == "[redacted]");
            assert!(hidden, "{config}: {}", reply.body);
        }
    }
}

#[test]
fn config_the_default_font_stays_visible_in_the_http_body() {
    let server = TestServer::start_with_store(|dir| write_config(dir, FIXTURE));
    let reply = get(&server);
    let value = json_of(&reply);
    let font = value["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["section"] == "web" && row["key"] == "font")
        .unwrap();
    assert_eq!(font["redacted"], false, "{}", reply.body);
}

#[test]
fn config_upper_case_secret_flags_are_absent_from_the_http_body() {
    for (config, leak) in [
        ("[changes]\ndiff_args = [\"GH_TOKEN=abc123\"]\n", "abc123"),
        ("[changes]\ndiff_args = [\"SECRET=hunter2\"]\n", "hunter2"),
    ] {
        let server = TestServer::start_with_store(|dir| write_config(dir, config));
        let reply = get(&server);
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert!(!reply.body.contains(leak), "{config}: {}", reply.body);
    }
}

#[test]
fn config_opaque_map_keys_are_absent_from_the_http_body() {
    for (config, leak, section) in [
        (
            concat!("[theme.status]\n\"gh", "p_AbCdEf0123456789AbCdEf0123456789abcd\" = \"red\"\n"),
            "ghp_AbCdEf",
            "theme",
        ),
        (
            "[mermaid.style.colors]\n\"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b\" = \"#fff\"\n",
            "9f86d081",
            "mermaid",
        ),
        (
            "[mermaid.style.config.x]\n\"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b.a1b2c3d4e5f60718293a4b5c6d7e8f9012345678\" = 1\n",
            "a1b2c3d4",
            "mermaid",
        ),
    ] {
        let server = TestServer::start_with_store(|dir| write_config(dir, config));
        let reply = get(&server);
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert!(!reply.body.contains(leak), "{}", reply.body);
        let masked = json_of(&reply)["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["section"] == section && row["key"] == "[redacted-1]")
            .unwrap()
            .clone();
        assert_eq!(masked["redacted"], true, "{}", reply.body);
        assert_eq!(masked["value"], "[redacted]", "{}", reply.body);
    }
}

#[test]
fn config_opaque_keys_and_values_in_an_array_of_tables_are_absent_from_the_http_body() {
    for (config, leak) in [
        (
            concat!(
                "[[mermaid.style.config.arr]]\n\"gh",
                "p_AbCdEf0123456789AbCdEf0123456789abcd\" = 1\n"
            ),
            "ghp_AbCdEf",
        ),
        (
            "[[mermaid.style.config.arr]]\nnote = \"AbCdEf0123456789AbCdEf0123456789abcd\"\n",
            "AbCdEf0123",
        ),
    ] {
        let server = TestServer::start_with_store(|dir| write_config(dir, config));
        let reply = get(&server);
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert!(!reply.body.contains(leak), "{}", reply.body);
        let array = json_of(&reply)["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["section"] == "mermaid" && row["key"] == "style.config.arr")
            .unwrap()
            .clone();
        assert_eq!(array["redacted"], true, "{}", reply.body);
        assert_eq!(array["value"], "[redacted]", "{}", reply.body);
    }
}
