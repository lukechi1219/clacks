mod support;

use clacks::ports::{CliSession, MessageStore, TelegramGateway, WaitError};
use std::time::Duration;
use support::*;

#[test]
fn scripted_cli_records_and_pops_in_order() {
    let mut cli = ScriptedCli::new(vec![ok_artifact("a"), ok_artifact("b")]);
    cli.inject_message("m1").unwrap();
    cli.inject_control("/clear").unwrap();
    assert_eq!(cli.messages, vec!["m1"]);
    assert_eq!(cli.controls, vec!["/clear"]);
    assert_eq!(cli.wait_artifact(Duration::from_secs(1)).unwrap().raw, "a");
    assert_eq!(cli.wait_artifact(Duration::from_secs(1)).unwrap().raw, "b");
}

#[test]
fn scripted_cli_times_out_when_script_exhausted() {
    let mut cli = ScriptedCli::new(vec![]);
    assert_eq!(
        cli.wait_artifact(Duration::from_secs(1)).unwrap_err(),
        WaitError::Timeout
    );
}

#[test]
fn scripted_cli_inject_failure_is_single_shot() {
    let mut cli = ScriptedCli::new(vec![]);
    cli.fail_next_inject = true;
    assert!(cli.inject_message("x").is_err());
    assert!(cli.inject_message("y").is_ok());
    assert_eq!(cli.messages, vec!["y"]);
}

#[test]
fn fake_gateway_scripts_polls_and_records_sends() {
    let gateway = FakeGateway::new();
    gateway.script_poll(Ok(vec![text_update(1, 9, "hi")]));
    let updates = gateway.poll_updates(0).unwrap();
    assert_eq!(updates.len(), 1);
    assert!(gateway.poll_updates(1).unwrap().is_empty()); // 腳本耗盡 = 空 poll
    gateway.send_reply(9, "yo").unwrap();
    assert_eq!(gateway.polled_offsets.borrow().as_slice(), &[0, 1]);
    assert_eq!(gateway.sent.borrow().as_slice(), &[(9, "yo".to_string())]);
}

#[test]
fn in_memory_store_dedups() {
    let mut store = InMemoryStore::new();
    assert!(store.first_seen(7).unwrap());
    assert!(!store.first_seen(7).unwrap());
    assert!(store.first_seen(8).unwrap());
}

#[test]
fn taster_artifact_nests_verdict_json_with_escaping() {
    let raw = taster_artifact(r#"{"safe":true}"#);
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["text"], r#"{"safe":true}"#);
}
