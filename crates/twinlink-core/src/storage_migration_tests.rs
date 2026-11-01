//! Upgrade/restart and failure-injection checks; no network or user data.
use super::*;

fn legacy(path: &Path, version: i64, final_path: bool) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE messages(room TEXT NOT NULL,id TEXT NOT NULL,timestamp INTEGER NOT NULL,value TEXT NOT NULL,PRIMARY KEY(room,id));
        CREATE TABLE transfers(room TEXT NOT NULL,id TEXT NOT NULL,value TEXT NOT NULL,PRIMARY KEY(room,id));").unwrap();
    conn.execute("INSERT INTO messages VALUES('room','11111111-1111-4111-8111-111111111111',1,?1)",
        [r#"{"id":"11111111-1111-4111-8111-111111111111","text":"legacy","outgoing":true,"timestamp":1}"#]).unwrap();
    if version >= 2 {
        conn.execute_batch("CREATE TABLE downloads(room TEXT NOT NULL,file_id TEXT NOT NULL,sender TEXT NOT NULL,name TEXT NOT NULL,size INTEGER NOT NULL,hash TEXT NOT NULL,status TEXT NOT NULL,target_dir TEXT,temp_path TEXT,next_index INTEGER NOT NULL DEFAULT 0,error TEXT,created_at INTEGER NOT NULL,PRIMARY KEY(room,file_id));").unwrap();
        if final_path { conn.execute_batch("ALTER TABLE downloads ADD COLUMN final_path TEXT;").unwrap(); }
    }
    conn.pragma_update(None, "user_version", version).unwrap();
}

fn assert_history(store: &Store) {
    let messages = store.messages("room").unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "legacy");
    assert_eq!(messages[0].delivery, "unknown");
    let db = store.0.lock().unwrap();
    assert_eq!(db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
    db.prepare("SELECT final_path FROM downloads").unwrap();
}

#[test]
fn upgrades_legacy_databases_and_reopens_without_reset_or_data_loss() {
    for (version, column_present) in [(1, false), (2, false), (2, true), (3, true)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        legacy(&path, version, column_present);
        for _ in 0..3 {
            let store = Store::open(&path).unwrap();
            assert_history(&store);
        }
    }
}

#[test]
fn failed_upgrade_rolls_back_schema_and_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    legacy(&path, 1, false);
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE INDEX quarantined_events ON messages(id);").unwrap();
    }
    assert!(Store::open(&path).is_err());
    {
        let conn = Connection::open(&path).unwrap();
        assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='downloads'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        conn.execute_batch("DROP INDEX quarantined_events;").unwrap();
    }
    assert_history(&Store::open(&path).unwrap());
}

#[test]
fn future_database_version_is_preserved_and_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.db");
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", 99).unwrap();
    drop(conn);
    assert!(Store::open(&path).is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 99);
}

#[test]
fn paged_history_has_no_duplicates_at_equal_timestamps_and_scopes_cursors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let store = Store::open(&path).unwrap();
    let mut ids = Vec::new();
    for i in 0..451 {
        let id = uuid::Uuid::new_v4().to_string();
        store.save_message("room-a", &Message {
            id: id.clone(), text: i.to_string(), timestamp: 100, outgoing: true,
            delivery: "unknown".into(), via: None,
        }).unwrap();
        ids.push(id);
    }
    store.remember_history_room("room-a").unwrap();
    assert_eq!(store.history_room().unwrap().as_deref(), Some("room-a"));
    let mut result = Vec::new();
    let mut before = None;
    loop {
        let page = store.message_page("room-a", before.as_deref()).unwrap();
        assert!(!page.messages.is_empty());
        before = Some(page.messages[0].id.clone());
        result.splice(0..0, page.messages.into_iter().map(|m|m.id));
        if !page.has_more { break; }
    }
    assert_eq!(result, ids);
    assert!(store.message_page("room-b", Some(&ids[0])).is_err());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.messages("room-a").unwrap().len(), 200);
    assert_eq!(store.history_room().unwrap().as_deref(), Some("room-a"));
}

#[test]
fn failed_outbox_insert_rolls_back_message_and_receipt_clears_pending_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("history.db")).unwrap();
    let message = Message { id: uuid::Uuid::new_v4().to_string(), text: "durability".into(), timestamp: 1,
        outgoing: true, delivery: "pending".into(), via: None };
    store.0.lock().unwrap().execute_batch("CREATE TRIGGER deny_outbox BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    assert!(store.enqueue_message("room", &message, &message.id, &message.id, "join", b"sealed fixture", 0).is_err());
    assert!(store.messages("room").unwrap().is_empty());
    store.0.lock().unwrap().execute_batch("DROP TRIGGER deny_outbox;").unwrap();
    store.enqueue_message("room", &message, &message.id, &message.id, "join", b"sealed fixture", 0).unwrap();
    assert_eq!(store.pending_outbox("room").unwrap(), 1);
    store.apply_receipt("room", &message.id, None, "direct").unwrap();
    assert_eq!(store.pending_outbox("room").unwrap(), 0);
    assert!(store.set_message_delivery("room", &message.id, "stored", Some("mailbox")).unwrap().is_none());
    assert_eq!(store.messages("room").unwrap()[0].delivery, "delivered");
}
