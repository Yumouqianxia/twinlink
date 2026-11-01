#!/usr/bin/env python3
"""Private, opaque store-and-forward for two TwinLink devices (Python stdlib).

This service does not implement networking, decrypt content, or execute uploads.
Run behind its own TLS listener, with an operator token supplied out of band.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import logging
import math
import os
import re
import secrets
import shutil
import sqlite3
import ssl
import threading
import time
import uuid
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

VERSION = 1
CHUNK_BYTES = 1024 * 1024 + 16
MAX_EVENT_BYTES = 96 * 1024
MAX_JSON_BYTES = 160 * 1024
HEX64 = re.compile(r"^[0-9a-f]{64}$")
ROLES = {"host", "join"}
LOG = logging.getLogger("twinlink.mailbox")


class ApiError(Exception):
    def __init__(self, status: int, code: str):
        self.status, self.code = status, code


def require(condition, status=400, code="invalid_request"):
    if not condition:
        raise ApiError(status, code)


def opaque_id(value: str) -> str:
    try:
        parsed = uuid.UUID(value)
    except (ValueError, TypeError, AttributeError):
        raise ApiError(400, "invalid_id") from None
    require(str(parsed) == value, code="invalid_id")
    return value


def timestamp() -> int:
    return int(time.time() * 1000)


def token_digest(token: str) -> str:
    return hashlib.sha256(token.encode("ascii")).hexdigest()


@dataclass
class Limits:
    ttl_seconds: int = 7 * 86400
    max_file_bytes: int = 1024 * 1024 * 1024
    max_storage_bytes: int = 4 * 1024 * 1024 * 1024
    max_queue_bytes: int = 32 * 1024 * 1024
    max_events: int = 4096
    max_rooms: int = 128
    min_free_bytes: int = 256 * 1024 * 1024


class Mailbox:
    def __init__(self, root: Path, operator_token: str, limits: Limits | None = None):
        require(len(operator_token) >= 32, code="operator_token_too_short")
        self.root = root
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(self.root, 0o700)
        self.objects = root / "objects"
        self.objects.mkdir(exist_ok=True, mode=0o700)
        self.operator = operator_token
        self.limits = limits or Limits()
        self.lock = threading.RLock()
        self.db = sqlite3.connect(root / "mailbox.db", check_same_thread=False)
        self.db.row_factory = sqlite3.Row
        os.chmod(root / "mailbox.db", 0o600)
        self.db.executescript("""
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=FULL;
            PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS rooms(
                id TEXT PRIMARY KEY, token_hash TEXT NOT NULL, created INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS events(
                room TEXT NOT NULL REFERENCES rooms(id), id TEXT NOT NULL,
                sender TEXT NOT NULL, recipient TEXT NOT NULL, payload TEXT NOT NULL,
                bytes INTEGER NOT NULL, created INTEGER NOT NULL, expires INTEGER NOT NULL,
                PRIMARY KEY(room,sender,id));
            CREATE INDEX IF NOT EXISTS inbox ON events(room,recipient,created);
            CREATE TABLE IF NOT EXISTS files(
                room TEXT NOT NULL REFERENCES rooms(id), id TEXT NOT NULL,
                sender TEXT NOT NULL, size INTEGER NOT NULL, chunks INTEGER NOT NULL,
                complete INTEGER NOT NULL DEFAULT 0, created INTEGER NOT NULL,
                expires INTEGER NOT NULL, PRIMARY KEY(room,id));
            CREATE TABLE IF NOT EXISTS parts(
                room TEXT NOT NULL,id TEXT NOT NULL,idx INTEGER NOT NULL,
                digest TEXT NOT NULL,bytes INTEGER NOT NULL,PRIMARY KEY(room,id,idx),
                FOREIGN KEY(room,id) REFERENCES files(room,id) ON DELETE CASCADE);
        """)
        self.last_cleanup = 0
        self.cleanup(force=True)

    def close(self):
        with self.lock:
            self.db.close()

    def authenticate(self, authorization: str):
        require(hmac.compare_digest(authorization.encode("utf-8"), ("Bearer " + self.operator).encode("utf-8")), 401, "unauthorized")

    def check_room(self, room: str, token: str):
        require(bool(HEX64.fullmatch(room)), code="invalid_room")
        require(bool(HEX64.fullmatch(token)), 403, "room_forbidden")
        row = self.db.execute("SELECT token_hash,created FROM rooms WHERE id=?", (room,)).fetchone()
        require(row is not None and hmac.compare_digest(row[0], token_digest(token)), 403, "room_forbidden")
        # `created` is also the last coarse activity timestamp (not exposed on
        # the wire). Do not fsync every inbox poll, but keep active empty rooms
        # alive so the janitor can reclaim abandoned registrations safely.
        now = timestamp()
        if now - row[1] >= min(3600_000, self.limits.ttl_seconds * 500):
            self.db.execute("UPDATE rooms SET created=? WHERE id=?", (now, room))

    def register(self, body):
        room, token = body.get("roomId", ""), body.get("roomToken", "")
        require(isinstance(room, str) and bool(HEX64.fullmatch(room)), code="invalid_room")
        require(isinstance(token, str) and bool(HEX64.fullmatch(token)), code="invalid_room_token")
        with self.lock, self.db:
            old = self.db.execute("SELECT token_hash FROM rooms WHERE id=?", (room,)).fetchone()
            if old:
                require(hmac.compare_digest(old[0], token_digest(token)), 409, "room_exists")
                self.db.execute("UPDATE rooms SET created=? WHERE id=?", (timestamp(), room))
            else:
                require(self.db.execute("SELECT count(*) FROM rooms").fetchone()[0] < self.limits.max_rooms,
                        507, "room_limit")
                self.db.execute("INSERT INTO rooms VALUES(?,?,?)", (room, token_digest(token), timestamp()))
        return {"roomId": room, "ttlSeconds": self.limits.ttl_seconds,
                "chunkBytes": CHUNK_BYTES, "maxFileBytes": self.limits.max_file_bytes}

    def file_row(self, room, file_id):
        opaque_id(file_id)
        row = self.db.execute("SELECT * FROM files WHERE room=? AND id=?", (room, file_id)).fetchone()
        require(row is not None and row["expires"] > timestamp(), 404, "file_missing_or_expired")
        return row

    def object_dir(self, room, file_id):
        # Both identifiers have already passed strict fixed-format validation.
        return self.objects / room / file_id

    def delete_file(self, room, file_id):
        self.db.execute("DELETE FROM files WHERE room=? AND id=?", (room, file_id))
        shutil.rmtree(self.object_dir(room, file_id), ignore_errors=True)

    def cleanup(self, force=False):
        with self.lock:
            now = timestamp()
            if not force and now - self.last_cleanup < 60_000:
                return
            with self.db:
                self.db.execute("DELETE FROM events WHERE expires<=?", (now,))
                for row in self.db.execute("SELECT room,id FROM files WHERE expires<=?", (now,)).fetchall():
                    self.delete_file(row[0], row[1])
                self.db.execute("""DELETE FROM rooms WHERE created<=?
                    AND NOT EXISTS(SELECT 1 FROM events WHERE events.room=rooms.id)
                    AND NOT EXISTS(SELECT 1 FROM files WHERE files.room=rooms.id)""",
                    (now - self.limits.ttl_seconds * 1000,))
            # Remove crash leftovers without following symlinks or accepting remote paths.
            known = {(r[0], r[1]) for r in self.db.execute("SELECT room,id FROM files")}
            for room_dir in self.objects.iterdir():
                if room_dir.is_symlink() or not room_dir.is_dir():
                    continue
                for file_dir in room_dir.iterdir():
                    if file_dir.is_symlink() or not file_dir.is_dir():
                        continue
                    if (room_dir.name, file_dir.name) not in known:
                        shutil.rmtree(file_dir)
                    else:
                        for tmp in file_dir.glob("*.tmp"):
                            if not tmp.is_symlink():
                                tmp.unlink(missing_ok=True)
            self.db.execute("PRAGMA wal_checkpoint(PASSIVE)")
            self.last_cleanup = now

    def event_post(self, room, body):
        event_id = opaque_id(body.get("id"))
        sender, recipient, payload = body.get("sender"), body.get("recipient"), body.get("payload")
        require(isinstance(sender, str) and isinstance(recipient, str) and sender in ROLES and recipient in ROLES and sender != recipient)
        require(isinstance(payload, str) and len(payload) <= MAX_EVENT_BYTES * 2)
        try:
            raw = base64.b64decode(payload + "=" * (-len(payload) % 4), altchars=b"-_", validate=True)
        except (ValueError, TypeError):
            raise ApiError(400, "invalid_payload") from None
        require(40 <= len(raw) <= MAX_EVENT_BYTES, 413, "payload_size")
        canonical = base64.urlsafe_b64encode(raw).decode().rstrip("=")
        require(canonical == payload, code="noncanonical_payload")
        old = self.db.execute("SELECT payload,recipient,created,expires FROM events WHERE room=? AND sender=? AND id=?",
                              (room, sender, event_id)).fetchone()
        if old:
            require(old[0] == payload and old[1] == recipient, 409, "event_conflict")
            return {"id": event_id, "createdAt": old[2], "expiresAt": old[3]}
        count, used = self.db.execute("SELECT count(*),coalesce(sum(bytes),0) FROM events").fetchone()
        require(count < self.limits.max_events and used + len(payload) <= self.limits.max_queue_bytes,
                507, "queue_full")
        now = timestamp()
        expires = now + self.limits.ttl_seconds * 1000
        self.db.execute("INSERT INTO events VALUES(?,?,?,?,?,?,?,?)",
                        (room, event_id, sender, recipient, payload, len(payload), now, expires))
        return {"id": event_id, "createdAt": now, "expiresAt": expires}

    def inbox(self, room, recipient):
        require(recipient in ROLES)
        events = []
        size = 0
        for row in self.db.execute("SELECT * FROM events WHERE room=? AND recipient=? AND expires>? ORDER BY created,id LIMIT 64",
                                   (room, recipient, timestamp())):
            if size + row["bytes"] > 1024 * 1024:
                break
            size += row["bytes"]
            events.append({"id": row["id"], "sender": row["sender"], "recipient": row["recipient"],
                           "payload": row["payload"], "createdAt": row["created"], "expiresAt": row["expires"]})
        return {"events": events}

    def begin_file(self, room, body):
        file_id = opaque_id(body.get("id"))
        sender, size, chunks = body.get("sender"), body.get("encryptedSize"), body.get("chunkCount")
        require(isinstance(sender, str) and sender in ROLES and type(size) is int and type(chunks) is int)
        max_size = self.limits.max_file_bytes + 16 * max(1, math.ceil(self.limits.max_file_bytes / (CHUNK_BYTES - 16)))
        require(16 <= size <= max_size and chunks == math.ceil(size / CHUNK_BYTES), 413, "file_size")
        # Every encrypted part must contain an authentication tag, including an empty file.
        require(size - (chunks - 1) * CHUNK_BYTES >= 16, code="invalid_last_chunk")
        old = self.db.execute("SELECT * FROM files WHERE room=? AND id=?", (room, file_id)).fetchone()
        if old:
            require((old["sender"], old["size"], old["chunks"]) == (sender, size, chunks), 409, "file_conflict")
        else:
            reserved = self.db.execute("SELECT coalesce(sum(size),0) FROM files").fetchone()[0]
            require(reserved + size <= self.limits.max_storage_bytes, 507, "storage_full")
            # Existing reservations also need disk space for their unwritten parts.
            written = self.db.execute("SELECT coalesce(sum(bytes),0) FROM parts").fetchone()[0]
            require(shutil.disk_usage(self.root).free - (reserved - written) - size >= self.limits.min_free_bytes,
                    507, "disk_space")
            now = timestamp()
            self.db.execute("INSERT INTO files VALUES(?,?,?,?,?,0,?,?)",
                            (room, file_id, sender, size, chunks, now, now + self.limits.ttl_seconds * 1000))
            self.object_dir(room, file_id).mkdir(parents=True, exist_ok=True, mode=0o700)
        return self.file_info(room, file_id)

    def file_info(self, room, file_id):
        row = self.file_row(room, file_id)
        return {"id": file_id, "sender": row["sender"], "encryptedSize": row["size"],
                "chunkCount": row["chunks"], "chunkBytes": CHUNK_BYTES,
                "received": [r[0] for r in self.db.execute("SELECT idx FROM parts WHERE room=? AND id=? ORDER BY idx", (room, file_id))],
                "complete": bool(row["complete"]), "expiresAt": row["expires"]}

    def put_part(self, room, file_id, index, data):
        row = self.file_row(room, file_id)
        require(0 <= index < row["chunks"], code="invalid_chunk_index")
        expected = min(CHUNK_BYTES, row["size"] - index * CHUNK_BYTES)
        require(len(data) == expected, code="chunk_size")
        digest = hashlib.sha256(data).hexdigest()
        old = self.db.execute("SELECT digest FROM parts WHERE room=? AND id=? AND idx=?", (room, file_id, index)).fetchone()
        if old:
            require(hmac.compare_digest(old[0], digest), 409, "chunk_conflict")
            return {"index": index, "sha256": digest}
        require(not row["complete"], 409, "already_complete")
        directory = self.object_dir(room, file_id)
        target = directory / str(index)
        temp = directory / (secrets.token_hex(16) + ".tmp")
        try:
            fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
            with os.fdopen(fd, "wb") as f:
                f.write(data)
                f.flush()
                os.fsync(f.fileno())
            os.replace(temp, target)
            directory_fd = os.open(directory, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
            try:
                os.fsync(directory_fd)
            finally:
                os.close(directory_fd)
            # File before metadata: a crash may leave an uncommitted part, safely overwritten on retry.
            self.db.execute("INSERT INTO parts VALUES(?,?,?,?,?)", (room, file_id, index, digest, len(data)))
        finally:
            temp.unlink(missing_ok=True)
        return {"index": index, "sha256": digest}

    def complete_file(self, room, file_id):
        row = self.file_row(room, file_id)
        count, size = self.db.execute("SELECT count(*),coalesce(sum(bytes),0) FROM parts WHERE room=? AND id=?", (room, file_id)).fetchone()
        require(count == row["chunks"] and size == row["size"], 409, "upload_incomplete")
        self.db.execute("UPDATE files SET complete=1 WHERE room=? AND id=?", (room, file_id))
        return self.file_info(room, file_id)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "TwinLinkMailbox/1"
    sys_version = ""

    def setup(self):
        self.request.settimeout(20)
        if isinstance(self.request, ssl.SSLSocket):
            self.request.do_handshake()
        super().setup()

    def log_message(self, fmt, *args):
        # Do not log headers, bodies, tokens, room IDs or filenames.
        LOG.info("request peer=%s method=%s", self.client_address[0], self.command)

    def parse_request(self):
        if not super().parse_request():
            return False
        if sum(len(k) + len(v) for k, v in self.headers.items()) > 16 * 1024:
            self.send_error(431)
            return False
        return True

    @property
    def mailbox(self):
        return self.server.mailbox

    def reply(self, status, body, content_type="application/json"):
        # A successful response means its metadata has reached SQLite's durable
        # transaction boundary, even if the client disconnects while reading it.
        with self.mailbox.lock:
            if self.mailbox.db.in_transaction and status < 400:
                self.mailbox.db.commit()
        if not isinstance(body, bytes):
            body = json.dumps(body, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)
        self.close_connection = True

    def read_body(self, maximum):
        require(self.headers.get("Transfer-Encoding") is None, 411, "content_length_required")
        try:
            size = int(self.headers.get("Content-Length", "-1"))
        except ValueError:
            raise ApiError(400, "invalid_content_length") from None
        require(0 <= size <= maximum, 413, "body_size")
        body = self.rfile.read(size)
        require(len(body) == size, code="incomplete_body")
        return body

    def read_json(self):
        require(self.headers.get("Content-Type", "").split(";")[0] == "application/json", 415, "json_required")
        try:
            body = json.loads(self.read_body(MAX_JSON_BYTES))
        except (ValueError, UnicodeError):
            raise ApiError(400, "invalid_json") from None
        require(isinstance(body, dict))
        return body

    def dispatch(self):
        try:
            parsed = urlsplit(self.path)
            require(not parsed.scheme and not parsed.netloc and len(self.path) <= 1024)
            parts = parsed.path.strip("/").split("/")
            query = parse_qs(parsed.query)
            if self.command == "GET" and parts == ["health"]:
                return self.reply(200, {"service": "twinlink-mailbox", "version": VERSION})
            self.mailbox.authenticate(self.headers.get("Authorization", ""))
            self.mailbox.cleanup()
            if self.command == "POST" and parts == ["v1", "rooms"]:
                return self.reply(200, self.mailbox.register(self.read_json()))
            require(len(parts) >= 4 and parts[:2] == ["v1", "rooms"], 404, "not_found")
            room = parts[2]
            # Authentication happens before reading large bodies or touching object files.
            with self.mailbox.lock, self.mailbox.db:
                self.mailbox.check_room(room, self.headers.get("X-Room-Token", ""))
                resource = parts[3]
                if resource == "events":
                    if len(parts) == 4 and self.command == "POST":
                        return self.reply(201, self.mailbox.event_post(room, self.read_json()))
                    if len(parts) == 4 and self.command == "GET":
                        return self.reply(200, self.mailbox.inbox(room, query.get("recipient", [""])[0]))
                    if len(parts) == 5 and self.command == "DELETE":
                        event_id = opaque_id(parts[4])
                        recipient = query.get("recipient", [""])[0]
                        require(recipient in ROLES)
                        self.mailbox.db.execute("DELETE FROM events WHERE room=? AND id=? AND recipient=?", (room, event_id, recipient))
                        return self.reply(200, {"deleted": True})
                if resource == "files":
                    if len(parts) == 4 and self.command == "POST":
                        return self.reply(201, self.mailbox.begin_file(room, self.read_json()))
                    require(len(parts) >= 5, 404, "not_found")
                    file_id = opaque_id(parts[4])
                    if len(parts) == 5:
                        if self.command == "GET":
                            return self.reply(200, self.mailbox.file_info(room, file_id))
                        if self.command == "DELETE":
                            self.mailbox.delete_file(room, file_id)
                            return self.reply(200, {"deleted": True})
                    if len(parts) == 6 and parts[5] == "complete" and self.command == "POST":
                        return self.reply(200, self.mailbox.complete_file(room, file_id))
                    if len(parts) == 7 and parts[5] == "chunks":
                        require(parts[6].isdigit() and len(parts[6]) <= 6, code="invalid_chunk_index")
                        index = int(parts[6])
                        if self.command == "PUT":
                            row = self.mailbox.file_row(room, file_id)
                            require(0 <= index < row["chunks"], code="invalid_chunk_index")
                            require(self.headers.get("X-Sender-Role") == row["sender"], 403, "sender_required")
                            return self.reply(200, self.mailbox.put_part(room, file_id, index, self.read_body(CHUNK_BYTES)))
                        if self.command == "GET":
                            row = self.mailbox.file_row(room, file_id)
                            require(row["complete"], 409, "upload_incomplete")
                            require(0 <= index < row["chunks"], code="invalid_chunk_index")
                            part = self.mailbox.db.execute("SELECT digest,bytes FROM parts WHERE room=? AND id=? AND idx=?", (room, file_id, index)).fetchone()
                            require(part is not None, 404, "chunk_missing")
                            path = self.mailbox.object_dir(room, file_id) / str(index)
                            fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
                            with os.fdopen(fd, "rb") as f:
                                data = f.read(CHUNK_BYTES + 1)
                            require(len(data) == part["bytes"] and hmac.compare_digest(hashlib.sha256(data).hexdigest(), part["digest"]), 500, "stored_chunk_corrupt")
                            return self.reply(200, data, "application/octet-stream")
                raise ApiError(404, "not_found")
        except ApiError as error:
            self.reply(error.status, {"error": error.code})
        except (BrokenPipeError, ConnectionResetError, TimeoutError, ssl.SSLError):
            self.close_connection = True
        except Exception:
            # Never include arbitrary exception text, which could reveal local paths/data.
            LOG.error("request failed (internal error)")
            self.reply(500, {"error": "internal_error"})

    do_GET = dispatch
    do_POST = dispatch
    do_PUT = dispatch
    do_DELETE = dispatch


class Server(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, address, mailbox, tls=None):
        self.mailbox = mailbox
        self.tls = tls
        self.slots = threading.BoundedSemaphore(16)
        super().__init__(address, Handler)

    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(20)
        if self.tls:
            connection = self.tls.wrap_socket(connection, server_side=True, do_handshake_on_connect=False)
        return connection, address

    def process_request(self, request, client_address):
        if not self.slots.acquire(blocking=False):
            request.close()
            return
        try:
            super().process_request(request, client_address)
        except Exception:
            self.slots.release()
            raise

    def process_request_thread(self, request, client_address):
        try:
            super().process_request_thread(request, client_address)
        finally:
            self.slots.release()

    def handle_error(self, request, client_address):
        LOG.warning("connection closed before completing a request")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=18443)
    parser.add_argument("--data", type=Path, default=Path("/var/lib/twinlink-mailbox"))
    parser.add_argument("--cert", type=Path, required=True)
    parser.add_argument("--key", type=Path, required=True)
    parser.add_argument("--max-storage-mib", type=int, default=4096)
    parser.add_argument("--max-file-mib", type=int, default=1024)
    parser.add_argument("--ttl-hours", type=int, default=168)
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    token = os.environ.get("TWINLINK_OPERATOR_TOKEN", "")
    if len(token) < 32:
        parser.error("set a strong TWINLINK_OPERATOR_TOKEN outside the source/config command line")
    if not (1 <= args.max_file_mib <= args.max_storage_mib <= 16384 and 1 <= args.ttl_hours <= 720):
        parser.error("invalid capacity or expiry limits")
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.minimum_version = ssl.TLSVersion.TLSv1_2
    tls.load_cert_chain(args.cert, args.key)
    mailbox = Mailbox(args.data, token, Limits(ttl_seconds=args.ttl_hours * 3600,
                      max_file_bytes=args.max_file_mib * 1024**2,
                      max_storage_bytes=args.max_storage_mib * 1024**2))
    server = Server((args.host, args.port), mailbox, tls)
    stop = threading.Event()

    def janitor():
        while not stop.wait(60):
            try:
                mailbox.cleanup(force=True)
            except Exception:
                LOG.error("scheduled cleanup failed")

    threading.Thread(target=janitor, daemon=True).start()
    LOG.info("mailbox listening on port %s; encrypted file quota=%s MiB; expiry=%s h", args.port, args.max_storage_mib, args.ttl_hours)
    try:
        server.serve_forever()
    finally:
        stop.set()
        server.server_close()
        # Daemon handlers are stopped when the process exits; do not close their shared DB underneath them.


if __name__ == "__main__":
    main()
