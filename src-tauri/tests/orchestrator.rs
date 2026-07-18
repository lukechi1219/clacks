mod support;

use clacks::app::{Orchestrator, PipelineConfig};
use clacks::core::contract::ContractViolation;
use clacks::core::pipeline::MessageOutcome;
use clacks::core::session;
use clacks::ports::{IncomingMessage, Update};
use support::*;

const SAFE_VERDICT: &str =
    r#"{"safe":true,"sanitized_text":"想請教一個問題","removed":[],"reason":"乾淨"}"#;
const UNSAFE_VERDICT: &str =
    r#"{"safe":false,"sanitized_text":"","removed":["整段"],"reason":"要求執行破壞性指令"}"#;

#[test]
fn happy_path_sanitizes_replies_and_clears_taster() {
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(SAFE_VERDICT))]);
    let mut cyrano = ScriptedCli::new(vec![ok_artifact(r#"{"text":"這是 cyrano 的回覆"}"#)]);
    let mut store = InMemoryStore::new();
    let (slept, sleeper) = recording_sleeper();

    let outcome = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.process_update(&text_update(1, 42, "hello\x1b[201~"))
    };

    assert_eq!(outcome, Some(MessageOutcome::Replied));
    assert_eq!(taster.messages.len(), 1);
    assert!(taster.messages[0].contains("---BEGIN UNTRUSTED MESSAGE---"));
    assert!(!taster.messages[0].contains('\x1b'), "控制字元須在信封層被中和");
    assert_eq!(taster.controls, vec!["/clear"]);
    assert_eq!(cyrano.messages.len(), 1);
    assert!(cyrano.messages[0].contains("想請教一個問題"));
    assert_eq!(
        gateway.sent.borrow().as_slice(),
        &[(42, "這是 cyrano 的回覆".to_string())]
    );
    // 控制緩衝落地驗證(smoke 競態實證):/clear 後套 CONTROL_BUFFER
    assert_eq!(slept.borrow().as_slice(), &[session::CONTROL_BUFFER]);
}

#[test]
fn unsafe_verdict_rejected_nothing_reaches_cyrano() {
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(UNSAFE_VERDICT))]);
    let mut cyrano = ScriptedCli::new(vec![]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let outcome = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.process_update(&text_update(1, 42, "惡意訊息"))
    };

    assert_eq!(
        outcome,
        Some(MessageOutcome::RejectedByTaster { reason: "要求執行破壞性指令".to_string() })
    );
    assert!(cyrano.messages.is_empty());
    assert!(gateway.sent.borrow().is_empty());
    assert_eq!(taster.controls, vec!["/clear"]); // 拒收也要清(無記憶不可協商)
}

#[test]
fn malformed_taster_reply_is_contract_violation() {
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact("不是 JSON"))]);
    let mut cyrano = ScriptedCli::new(vec![]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let outcome = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.process_update(&text_update(1, 42, "hi"))
    };

    assert!(matches!(
        outcome,
        Some(MessageOutcome::ContractViolation(ContractViolation::NotJson(_)))
    ));
    assert!(gateway.sent.borrow().is_empty());
    assert_eq!(taster.controls, vec!["/clear"]);
}

#[test]
fn taster_timeout_still_clears_and_cyrano_untouched() {
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![]); // 腳本耗盡 = timeout
    let mut cyrano = ScriptedCli::new(vec![]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let outcome = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.process_update(&text_update(1, 42, "hi"))
    };

    assert_eq!(outcome, Some(MessageOutcome::TasterTimeout));
    assert_eq!(taster.controls, vec!["/clear"]);
    assert!(cyrano.messages.is_empty());
}

#[test]
fn empty_cyrano_reply_never_sent() {
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(SAFE_VERDICT))]);
    let mut cyrano = ScriptedCli::new(vec![ok_artifact(r#"{"text":""}"#)]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let outcome = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.process_update(&text_update(1, 42, "hi"))
    };

    assert!(matches!(
        outcome,
        Some(MessageOutcome::ContractViolation(ContractViolation::EmptyReply))
    ));
    assert!(gateway.sent.borrow().is_empty());
}

#[test]
fn duplicate_update_processed_once() {
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(UNSAFE_VERDICT))]);
    let mut cyrano = ScriptedCli::new(vec![]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let (first, second) = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        let update = text_update(1, 42, "hi");
        (orchestrator.process_update(&update), orchestrator.process_update(&update))
    };

    assert!(first.is_some());
    assert_eq!(second, None);
    assert_eq!(taster.messages.len(), 1);
}

#[test]
fn non_text_updates_skipped_without_reply() {
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![]);
    let mut cyrano = ScriptedCli::new(vec![]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let (no_message, photo_only) = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        (
            orchestrator.process_update(&Update { update_id: 1, message: None }),
            orchestrator.process_update(&Update {
                update_id: 2,
                message: Some(IncomingMessage { chat_id: 42, text: None }),
            }),
        )
    };

    assert_eq!(no_message, Some(MessageOutcome::SkippedNonText));
    assert_eq!(photo_only, Some(MessageOutcome::SkippedNonText));
    assert!(taster.messages.is_empty());
    assert!(gateway.sent.borrow().is_empty());
}

#[test]
fn store_failure_fails_closed() {
    // 去重狀態不明時不處理:重覆回覆的風險大於漏回一則
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![]);
    let mut cyrano = ScriptedCli::new(vec![]);
    let mut store = FailingStore;
    let (_slept, sleeper) = recording_sleeper();

    let outcome = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.process_update(&text_update(1, 42, "hi"))
    };

    assert!(matches!(outcome, Some(MessageOutcome::StoreFailed(_))));
    assert!(taster.messages.is_empty());
}

#[test]
fn send_failure_reported() {
    let gateway = FakeGateway::new();
    *gateway.send_error.borrow_mut() = Some("network down".to_string());
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(SAFE_VERDICT))]);
    let mut cyrano = ScriptedCli::new(vec![ok_artifact(r#"{"text":"回覆"}"#)]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let outcome = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.process_update(&text_update(1, 42, "hi"))
    };

    assert_eq!(outcome, Some(MessageOutcome::SendFailed("network down".to_string())));
}

#[test]
fn taster_inject_failure_is_session_lost() {
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![]);
    taster.fail_next_inject = true;
    let mut cyrano = ScriptedCli::new(vec![]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let outcome = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.process_update(&text_update(1, 42, "hi"))
    };

    assert_eq!(outcome, Some(MessageOutcome::SessionLost));
    assert!(gateway.sent.borrow().is_empty());
}

#[test]
fn poll_once_advances_offset_and_processes_each_update() {
    let gateway = FakeGateway::new();
    gateway.script_poll(Ok(vec![
        text_update(7, 42, "第一則"),
        text_update(9, 42, "第二則"),
    ]));
    // 兩則都走 unsafe 路徑(不需 cyrano 腳本,測試聚焦 poll 邏輯)
    let mut taster = ScriptedCli::new(vec![
        ok_artifact(&taster_artifact(UNSAFE_VERDICT)),
        ok_artifact(&taster_artifact(UNSAFE_VERDICT)),
    ]);
    let mut cyrano = ScriptedCli::new(vec![]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let (next_offset, outcomes) = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.poll_once(5).unwrap()
    };

    assert_eq!(next_offset, 10); // max(update_id) + 1
    assert_eq!(outcomes.len(), 2);
    assert_eq!(gateway.polled_offsets.borrow().as_slice(), &[5]);
}

#[test]
fn poll_once_skips_updates_seen_in_earlier_polls() {
    let gateway = FakeGateway::new();
    gateway.script_poll(Ok(vec![text_update(7, 42, "同一則")]));
    gateway.script_poll(Ok(vec![text_update(7, 42, "同一則")])); // 重送
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(UNSAFE_VERDICT))]);
    let mut cyrano = ScriptedCli::new(vec![]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let (first, second) = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        (orchestrator.poll_once(0).unwrap(), orchestrator.poll_once(8).unwrap())
    };

    assert_eq!(first.1.len(), 1);
    assert!(second.1.is_empty()); // 去重擋下,無結果
    assert_eq!(second.0, 8); // offset 不倒退
    assert_eq!(taster.messages.len(), 1);
}

#[test]
fn poll_once_propagates_gateway_error() {
    let gateway = FakeGateway::new();
    gateway.script_poll(Err(clacks::ports::GatewayError("os 53".to_string())));
    let mut taster = ScriptedCli::new(vec![]);
    let mut cyrano = ScriptedCli::new(vec![]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let result = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.poll_once(0)
    };

    assert!(result.is_err()); // 重試/退避是 run_forever 的職責,poll_once 如實上報
}

#[test]
fn empty_poll_keeps_offset() {
    let gateway = FakeGateway::new(); // 無腳本 = 永遠空 poll
    let mut taster = ScriptedCli::new(vec![]);
    let mut cyrano = ScriptedCli::new(vec![]);
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let (next_offset, outcomes) = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        orchestrator.poll_once(17).unwrap()
    };

    assert_eq!(next_offset, 17);
    assert!(outcomes.is_empty());
}
