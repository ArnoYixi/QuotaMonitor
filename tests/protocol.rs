use quotamonitor::{
    engine::{Command, Event, Status, ViewState, Worker},
    rpc::Rpc,
    runtime::BackendCommand,
    store,
};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

fn root() -> PathBuf {
    let p = std::env::temp_dir().join(format!("quotamonitor-integration-{}", uuid::Uuid::new_v4()));
    store::ensure_private_dir(&p).unwrap();
    p
}
fn backend() -> BackendCommand {
    BackendCommand {
        executable: std::env::var_os("QUOTA_TEST_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "python".into()
                } else {
                    "python3".into()
                }
            }),
        prefix: vec![
            "-u".into(),
            format!("{}/tests/fake_codex.py", env!("CARGO_MANIFEST_DIR")),
        ],
    }
}
fn await_state(
    rx: &mpsc::Receiver<Event>,
    predicate: impl Fn(&ViewState) -> bool,
    max_secs: u64,
) -> ViewState {
    let deadline = Instant::now() + Duration::from_secs(max_secs);
    loop {
        let e = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("expected state before deadline");
        if let Event::State(s) = e {
            if predicate(&s) {
                return s;
            }
        }
    }
}
#[test]
fn parses_interleaved_notifications_and_scopes_accounts() {
    let a = root();
    let b = root();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut rpc = Rpc::spawn(&backend(), &a, false, cancel.clone()).unwrap();
    let q = rpc
        .request("account/rateLimits/read", json!({}), Duration::from_secs(3))
        .unwrap();
    assert_eq!(q["rateLimitResetCredits"]["availableCount"], 3);
    assert!(!rpc.notifications.is_empty());
    rpc.request(
        "account/rateLimitResetCredit/consume",
        json!({"idempotencyKey":"fixed","creditId":"soon"}),
        Duration::from_secs(3),
    )
    .unwrap();
    let retry = rpc
        .request(
            "account/rateLimitResetCredit/consume",
            json!({"idempotencyKey":"fixed","creditId":"soon"}),
            Duration::from_secs(3),
        )
        .unwrap();
    assert_eq!(retry["outcome"], "alreadyRedeemed");
    let mut other = Rpc::spawn(&backend(), &b, false, cancel).unwrap();
    let q = other
        .request("account/rateLimits/read", json!({}), Duration::from_secs(3))
        .unwrap();
    assert_eq!(q["rateLimitResetCredits"]["availableCount"], 3);
    drop(rpc);
    drop(other);
    fs::remove_dir_all(a).unwrap();
    fs::remove_dir_all(b).unwrap();
}
#[test]
fn lost_response_and_process_restart_reuse_same_redemption() {
    let home = root();
    fs::write(
        home.join("fixture.json"),
        json!({"drop_first_response":true}).to_string(),
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let worker = Worker::start(
        "first".into(),
        home.clone(),
        backend(),
        30,
        false,
        false,
        tx,
        Arc::new(|| {}),
    );
    await_state(&rx, |s| s.status == Status::Ready, 10);
    worker.send(Command::Reset);
    let failed = await_state(&rx, |s| s.status == Status::Error && s.pending, 35);
    assert!(failed.message.contains("超时"));
    let pending = store::load_journal(&home).unwrap().pending.unwrap();
    assert_eq!(pending.credit_id, "soon");
    drop(worker);
    let (tx, rx) = mpsc::channel();
    let worker = Worker::start(
        "second".into(),
        home.clone(),
        backend(),
        30,
        false,
        false,
        tx,
        Arc::new(|| {}),
    );
    await_state(&rx, |s| s.status == Status::Ready && s.pending, 10);
    // A pending redemption must not prevent reauthentication after token expiry.
    worker.send(Command::Login { device: false });
    await_state(&rx, |s| s.status == Status::WaitingLogin && s.pending, 10);
    worker.send(Command::CancelLogin);
    worker.send(Command::Reset);
    let done = await_state(&rx, |s| s.status == Status::Ready && !s.pending, 10);
    assert!(done.message.contains("未重复"));
    let ledger: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("fixture-ledger.json")).unwrap()).unwrap();
    assert_eq!(ledger["redeemed"].as_object().unwrap().len(), 1);
    let requests = ledger["requests"].as_array().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert!(store::load_journal(&home).unwrap().pending.is_none());
    drop(worker);
    fs::remove_dir_all(home).unwrap();
}
#[test]
fn pending_reset_cannot_be_replayed_into_another_account() {
    let home = root();
    let mut j = store::ResetJournal::default();
    j.pending = Some(store::PendingReset {
        idempotency_key: "old".into(),
        credit_id: "soon".into(),
        account_id: Some("different-account".into()),
        created_at: 1,
    });
    store::save_journal(&home, &j).unwrap();
    let (tx, rx) = mpsc::channel();
    let worker = Worker::start(
        "identity".into(),
        home.clone(),
        backend(),
        30,
        false,
        false,
        tx,
        Arc::new(|| {}),
    );
    await_state(&rx, |s| s.status == Status::Ready, 10);
    worker.send(Command::Reset);
    let s = await_state(&rx, |s| s.status == Status::Error, 10);
    assert!(s.message.contains("不一致"));
    assert!(!home.join("fixture-ledger.json").exists());
    drop(worker);
    fs::remove_dir_all(home).unwrap();
}
