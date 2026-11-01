import base64
import hashlib
import http.client
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
import unittest
import uuid

sys.path.insert(0, str(Path(__file__).parent))
from mailbox import CHUNK_BYTES, Mailbox, Limits, Server


class MailboxTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.token = "operator-test-only-" + "a" * 40
        self.room = "b" * 64
        self.room_token = "c" * 64
        self.limits = Limits(min_free_bytes=0)
        self.box = Mailbox(self.root, self.token, self.limits)
        self.server = Server(("127.0.0.1", 0), self.box)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.base = "/v1/rooms/" + self.room
        self.assertEqual(self.request("POST", "/v1/rooms", {"roomId": self.room, "roomToken": self.room_token})[0], 200)

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
        self.box.close()
        self.tmp.cleanup()

    def request(self, method, path, body=None, token=None, room_token=None, extra=None):
        headers = {"Authorization": "Bearer " + (self.token if token is None else token),
                   "X-Room-Token": self.room_token if room_token is None else room_token}
        if isinstance(body, dict):
            body = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        if extra:
            headers.update(extra)
        c = http.client.HTTPConnection("127.0.0.1", self.server.server_port, timeout=5)
        c.request(method, path, body=body, headers=headers)
        r = c.getresponse()
        content = r.read()
        result = json.loads(content) if r.getheader("Content-Type", "").startswith("application/json") else content
        c.close()
        return r.status, result

    def event(self):
        return {"id": str(uuid.uuid4()), "sender": "host", "recipient": "join",
                "payload": base64.urlsafe_b64encode(os.urandom(80)).decode().rstrip("=")}

    def test_authentication_and_room_isolation(self):
        self.assertEqual(self.request("GET", "/health", token="")[0], 200)
        self.assertEqual(self.request("GET", self.base + "/events?recipient=join", token="bad")[0], 401)
        self.assertEqual(self.request("GET", self.base + "/events?recipient=join", room_token="d" * 64)[0], 403)
        self.assertEqual(self.request("GET", "/v1/rooms/" + "d" * 64 + "/events?recipient=join")[0], 403)
        self.assertEqual(self.request("POST", "/v1/rooms", {"roomId": self.room, "roomToken": "d" * 64})[0], 409)

    def test_event_durable_idempotent_and_explicit_ack(self):
        event = self.event()
        first = self.request("POST", self.base + "/events", event)
        self.assertEqual(first[0], 201)
        self.assertEqual(self.request("POST", self.base + "/events", event), first)
        self.assertEqual(self.request("GET", self.base + "/events?recipient=host")[1]["events"], [])
        self.assertEqual(len(self.request("GET", self.base + "/events?recipient=join")[1]["events"]), 1)
        # Verify committed data through a separate connection, not the handler's connection.
        with sqlite3.connect(self.root / "mailbox.db") as other:
            self.assertEqual(other.execute("SELECT count(*) FROM events").fetchone()[0], 1)
        changed = dict(event, payload=self.event()["payload"])
        self.assertEqual(self.request("POST", self.base + "/events", changed)[0], 409)
        self.request("DELETE", self.base + "/events/" + event["id"] + "?recipient=join")
        self.assertEqual(self.request("GET", self.base + "/events?recipient=join")[1]["events"], [])

    def test_file_resume_integrity_and_no_remote_paths(self):
        file_id = str(uuid.uuid4())
        data = [os.urandom(CHUNK_BYTES), os.urandom(48)]
        create = {"id": file_id, "sender": "host", "encryptedSize": sum(map(len, data)), "chunkCount": 2}
        self.assertEqual(self.request("POST", self.base + "/files", create)[0], 201)
        path = self.base + "/files/" + file_id
        self.assertEqual(self.request("POST", path + "/complete")[0], 409)
        self.assertEqual(self.request("GET", path + "/chunks/0")[0], 409)
        self.assertEqual(self.request("PUT", path + "/chunks/0", data[1], extra={"X-Sender-Role": "join"})[0], 403)
        for index, content in enumerate(data):
            expected = self.request("PUT", path + f"/chunks/{index}", content, extra={"X-Sender-Role": "host"})
            self.assertEqual(expected[0], 200)
            self.assertEqual(expected[1]["sha256"], hashlib.sha256(content).hexdigest())
            self.assertEqual(self.request("PUT", path + f"/chunks/{index}", content, extra={"X-Sender-Role": "host"}), expected)
        info = self.request("POST", self.base + "/files", create)[1]
        self.assertEqual(info["received"], [0, 1])
        self.assertEqual(self.request("PUT", path + "/chunks/1", b"x" * 48, extra={"X-Sender-Role": "host"})[0], 409)
        self.assertTrue(self.request("POST", path + "/complete")[1]["complete"])
        for index, content in enumerate(data):
            self.assertEqual(self.request("GET", path + f"/chunks/{index}"), (200, content))
        self.assertEqual(self.request("GET", path + "/chunks/../../password")[0], 404)
        self.assertEqual(self.request("POST", self.base + "/files", dict(create, id="../../escape"))[0], 400)
        # Server checks stored ciphertext for disk corruption before handing it out.
        (self.box.object_dir(self.room, file_id) / "1").write_bytes(b"z" * 48)
        self.assertEqual(self.request("GET", path + "/chunks/1")[0], 500)
        self.request("DELETE", path)
        self.assertFalse(self.box.object_dir(self.room, file_id).exists())

    def test_quota_reservations_and_expiry_cleanup(self):
        self.box.limits.max_storage_bytes = 100
        file_id = str(uuid.uuid4())
        create = {"id": file_id, "sender": "host", "encryptedSize": 80, "chunkCount": 1}
        self.assertEqual(self.request("POST", self.base + "/files", create)[0], 201)
        self.assertEqual(self.request("POST", self.base + "/files", dict(create, id=str(uuid.uuid4())))[0], 507)
        self.request("POST", self.base + "/events", self.event())
        with self.box.lock, self.box.db:
            self.box.db.execute("UPDATE files SET expires=0")
            self.box.db.execute("UPDATE events SET expires=0")
        self.box.cleanup(force=True)
        self.assertFalse(self.box.object_dir(self.room, file_id).exists())
        self.assertEqual(self.request("GET", self.base + "/events?recipient=join")[1]["events"], [])
        self.assertEqual(self.request("POST", self.base + "/files", dict(create, id=str(uuid.uuid4())))[0], 201)

    def test_oversized_and_malformed_inputs(self):
        self.assertEqual(self.request("POST", self.base + "/events", b"x" * (160 * 1024 + 1), extra={"Content-Type": "application/json"})[0], 413)
        self.assertEqual(self.request("POST", self.base + "/events", {"id": str(uuid.uuid4()), "sender": [], "recipient": "join"})[0], 400)
        self.assertEqual(self.request("POST", self.base + "/files", {"id": str(uuid.uuid4()), "sender": "host", "encryptedSize": True, "chunkCount": 1})[0], 400)
        self.box.limits.max_queue_bytes = 1
        self.assertEqual(self.request("POST", self.base + "/events", self.event())[0], 507)

    def test_idle_empty_rooms_reclaimed_but_active_or_pending_rooms_survive(self):
        # An old room with an unexpired envelope must remain claimable.
        event = self.event()
        self.assertEqual(self.request("POST", self.base + "/events", event)[0], 201)
        with self.box.lock, self.box.db:
            self.box.db.execute("UPDATE rooms SET created=0 WHERE id=?", (self.room,))
        self.box.cleanup(force=True)
        with self.box.lock:
            self.assertEqual(self.box.db.execute("SELECT count(*) FROM rooms").fetchone()[0], 1)
        # Authentication refreshes activity, without exposing a room credential.
        with self.box.lock, self.box.db:
            self.box.check_room(self.room, self.room_token)
            self.box.db.execute("DELETE FROM events WHERE room=?", (self.room,))
        self.box.cleanup(force=True)
        self.assertEqual(self.request("GET", self.base + "/events?recipient=join")[0], 200)
        # Only a genuinely idle, empty registration is removed, freeing quota.
        with self.box.lock, self.box.db:
            self.box.db.execute("UPDATE rooms SET created=0 WHERE id=?", (self.room,))
        self.box.cleanup(force=True)
        self.box.limits.max_rooms = 1
        self.assertEqual(self.request("POST", "/v1/rooms", {
            "roomId": "d" * 64, "roomToken": "e" * 64})[0], 200)
        self.assertEqual(self.request("POST", "/v1/rooms", {
            "roomId": self.room, "roomToken": self.room_token})[0], 507)


if __name__ == "__main__":
    unittest.main()
