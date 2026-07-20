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
fn session_lost_restarts_both_keeping_taster_clean() {
    // 安全義務(findings「Phase 4 規劃承接項」):cyrano 在安全路徑注入失敗 →
    // 健康 taster 帶著剛判定的訊息 context 停留 = 消毒者無記憶被打破。
    // 恢復必須讓 taster 乾淨(重啟為全新 session)
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(SAFE_VERDICT))]);
    let mut cyrano = ScriptedCli::new(vec![]);
    cyrano.fail_next_inject = true; // cyrano 注入即死(安全路徑上失敗)
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
    assert_eq!(taster.respawns, 1, "健康 taster 必須一併重啟以保證乾淨");
    assert!(taster.messages.is_empty(), "重啟後 taster 不得殘留訊息 context");
    assert_eq!(cyrano.respawns, 1, "死掉的 cyrano 必須重啟");
    assert!(gateway.sent.borrow().is_empty());
}

#[test]
fn taster_loss_at_start_restarts_both() {
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![]);
    taster.fail_next_inject = true; // 起手注入即死
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
    assert_eq!(taster.respawns, 1);
    assert_eq!(cyrano.respawns, 1);
}

#[test]
fn failed_taster_respawn_blocks_next_message() {
    // final review Important:taster respawn 失敗 = taster 可能仍活著且髒
    // (持有前一則不可信訊息 context)。下一則訊息絕不能被注入到髒 taster——
    // 這是「消毒者無記憶」的安全義務,fail-closed 優先於可用性。
    //
    // 注意(deviation from spec):taster_dirty 活在 Orchestrator 本身(不是
    // fakes),兩次 process_update 必須在同一個 orchestrator 實例上呼叫才能
    // 驗證跨呼叫的 fail-closed 行為——比照既有 duplicate_update_processed_once
    // 的寫法,兩次呼叫都在同一個 scope block 內,離開 block 後才檢查 fakes。
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(SAFE_VERDICT))]);
    taster.fail_respawns = 2; // 第一次(mid-loop recover)與第二次(頂部重試)都失敗
    let mut cyrano = ScriptedCli::new(vec![]);
    cyrano.fail_next_inject = true; // 安全路徑上死亡,觸發第一次 recover
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let (first, second) = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        let first = orchestrator.process_update(&text_update(1, 42, "第一則"));
        let second = orchestrator.process_update(&text_update(2, 42, "第二則"));
        (first, second)
    };

    assert_eq!(first, Some(MessageOutcome::SessionLost));
    assert_eq!(
        second,
        Some(MessageOutcome::SessionLost),
        "頂部重試恢復仍失敗,fail-closed 拒收本則"
    );
    assert_eq!(taster.respawns, 0, "兩次 respawn 都失敗,成功計數不動");
    assert_eq!(
        taster.messages.len(),
        1,
        "安全斷言核心:髒 taster 絕不可收到第二則訊息(respawn 未成功清空前)"
    );
}

#[test]
fn taster_recovers_on_retry_then_processes() {
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(SAFE_VERDICT))]);
    taster.fail_respawns = 1; // 只有第一次(mid-loop recover)失敗,頂部重試成功
    let mut cyrano = ScriptedCli::new(vec![]);
    cyrano.fail_next_inject = true;
    let mut store = InMemoryStore::new();
    let (_slept, sleeper) = recording_sleeper();

    let (first, second) = {
        let mut orchestrator = Orchestrator::new(
            &gateway, &mut taster, &mut cyrano, &mut store,
            PipelineConfig::default(), sleeper,
        );
        let first = orchestrator.process_update(&text_update(1, 42, "第一則"));
        let second = orchestrator.process_update(&text_update(2, 42, "next"));
        (first, second)
    };

    assert_eq!(first, Some(MessageOutcome::SessionLost));
    // 頂部重試成功 → 正常處理 → taster 腳本已耗盡(只給了一個 artifact)→ timeout
    assert_eq!(second, Some(MessageOutcome::TasterTimeout));
    assert_eq!(taster.respawns, 1, "重試恢復成功,respawn 計數才前進");
    assert_eq!(taster.messages.len(), 1, "清空後只收到 update 2 的訊息(重新注入)");
    assert!(taster.messages[0].contains("next"));
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
fn injects_only_after_waiting_for_idle() {
    // 設計輸入 A/C:每則訊息注入前必先等 CLI 就緒(wait_idle),消除掉字空窗
    let gateway = FakeGateway::new();
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

    assert_eq!(outcome, Some(MessageOutcome::Replied));
    assert!(taster.idle_waits >= 1, "taster 注入前必須先 wait_idle");
    assert!(cyrano.idle_waits >= 1, "cyrano 注入前必須先 wait_idle");
}

#[test]
fn idle_timeout_still_injects_best_effort() {
    // wait_idle 逾時(卡就緒偵測)不得變成失敗:best-effort 續注入,仍走完管線
    let gateway = FakeGateway::new();
    let mut taster = ScriptedCli::new(vec![ok_artifact(&taster_artifact(SAFE_VERDICT))]);
    taster.idle_timeouts = 1; // taster 首次 wait_idle 回 Timeout
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

    assert_eq!(outcome, Some(MessageOutcome::Replied), "idle 逾時仍應 best-effort 完成");
    assert_eq!(taster.messages.len(), 1, "逾時後仍注入(訊息有送達 taster)");
    // task reviewer 發現:原斷言不驗證 wait_idle 真的被呼叫過(idle_timeouts
    // 消耗機制本身就需要至少一次呼叫,但斷言未釘死)——即使對舊 orchestrator
    // (未編排 wait_idle)本測試也會通過,無法辨別本任務的編排是否存在。
    // 補這行讓測試對編排缺失有辨別力,同時保留原意圖(逾時不致失敗)
    assert!(taster.idle_waits >= 1, "本測試須真的呼叫過 wait_idle 才有辨別力");
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
