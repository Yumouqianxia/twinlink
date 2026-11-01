CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS messages (
  room TEXT NOT NULL, id TEXT NOT NULL, timestamp INTEGER NOT NULL,
  value TEXT NOT NULL, PRIMARY KEY(room,id)
);
CREATE INDEX IF NOT EXISTS messages_room_time ON messages(room,timestamp);
CREATE TABLE IF NOT EXISTS transfers (
  room TEXT NOT NULL, id TEXT NOT NULL, value TEXT NOT NULL,
  PRIMARY KEY(room,id)
);


-- Phase 2 durable offline mailbox state. Every row is scoped by room ID so
-- switching rooms can never mix unrelated encrypted queues.
CREATE TABLE IF NOT EXISTS outbox (
  room TEXT NOT NULL,
  event_id TEXT NOT NULL,
  dedupe TEXT NOT NULL,
  kind TEXT NOT NULL,
  recipient TEXT NOT NULL,
  envelope BLOB NOT NULL,
  created_at INTEGER NOT NULL,
  next_attempt INTEGER NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL DEFAULT 'pending',
  last_error TEXT,
  PRIMARY KEY(room,event_id),
  UNIQUE(room,dedupe)
);
CREATE INDEX IF NOT EXISTS outbox_due ON outbox(room,state,next_attempt);

CREATE TABLE IF NOT EXISTS seen_events (
  room TEXT NOT NULL,
  event_id TEXT NOT NULL,
  processed_at INTEGER NOT NULL,
  PRIMARY KEY(room,event_id)
);

CREATE TABLE IF NOT EXISTS uploads (
  room TEXT NOT NULL,
  file_id TEXT NOT NULL,
  source_path TEXT NOT NULL,
  name TEXT NOT NULL,
  size INTEGER NOT NULL,
  hash TEXT NOT NULL,
  encrypted_size INTEGER NOT NULL,
  chunk_count INTEGER NOT NULL,
  status TEXT NOT NULL,
  error TEXT,
  created_at INTEGER NOT NULL,
  final_path TEXT,
  PRIMARY KEY(room,file_id)
);

CREATE TABLE IF NOT EXISTS downloads (
  room TEXT NOT NULL,
  file_id TEXT NOT NULL,
  sender TEXT NOT NULL,
  name TEXT NOT NULL,
  size INTEGER NOT NULL,
  hash TEXT NOT NULL,
  status TEXT NOT NULL,
  target_dir TEXT,
  temp_path TEXT,
  next_index INTEGER NOT NULL DEFAULT 0,
  error TEXT,
  created_at INTEGER NOT NULL,
  final_path TEXT,
  PRIMARY KEY(room,file_id)
);


CREATE TABLE IF NOT EXISTS quarantined_events (
  room TEXT NOT NULL,
  event_id TEXT NOT NULL,
  sender TEXT NOT NULL,
  recipient TEXT NOT NULL,
  payload TEXT NOT NULL,
  error TEXT NOT NULL,
  quarantined_at INTEGER NOT NULL,
  PRIMARY KEY(room,event_id)
);
PRAGMA user_version = 3;
