# Private mailbox protocol v1

The server stores opaque encrypted envelopes and encrypted file chunks. It is
separate from EasyTier's discovery/relay node. This protocol does not implement
NAT traversal or remove recipient consent. There is no public account sign-up.

## Provisioning and authentication

The coordinated client `ServerConfig` is camelCase JSON:

```text
{relayUrl, mailboxUrl, mailboxCaPem, accessToken, networkPrefix}
```

Deployment supplies the actual values outside the repository. The mailbox is
`https://45.76.70.254:18443`, relay `tcp://45.76.70.254:11010`. The certificate
has that IP as a SAN. Add **only that provisioned certificate** as an additional
trusted root and keep normal TLS hostname/certificate verification enabled.
Never use danger_accept_invalid_certs. Reject HTTP endpoints and HTTP redirects
in production; never forward authorization headers to another origin.

Every `/v1` request requires `Authorization: Bearer <accessToken>`.
All room-specific requests except registration also require
`X-Room-Token: <64 lowercase hex>`. These are header values, never URL params.
Mark secret headers sensitive and do not log request bodies or codes.

`roomId` must be 64 lowercase hex and can equal the client's stable local room
identifier. `roomToken = hex(HMAC-SHA256(seed, "twinlink/mailbox/auth/v1"))`.
Room IDs need not be secrets; knowing an ID alone must confer no authorization.

All non-binary bodies/responses are JSON. Set Content-Type application/json
and a Content-Length for POST bodies. Responses are bounded and connections
may close after a request. Errors use `{ "error": "machine_code" }` and HTTP
400/401/403/404/409/413/415/507. Do not show server errors as successful delivery.

## Room registration

`POST /v1/rooms` body `{roomId, roomToken}`. Idempotent if the token matches;
another token for an existing room gets 409. Response:

```text
{roomId, ttlSeconds:604800, chunkBytes:1048592, maxFileBytes:1073741824}
```

Register on room activation/restart before polling or uploading. The private
deployment caps registration at 128 rooms. Idle registrations with no pending
events/files are reclaimed after the retention period; active authenticated
requests refresh coarse activity. Re-registering a reclaimed room is supported. `GET /health` is unauthenticated
and returns `{service:"twinlink-mailbox",version:1}` without data or settings.

## Opaque event queue

`POST /v1/rooms/{roomId}/events`:

```text
{id:<canonical UUID>, sender:"host"|"join", recipient:<opposite role>,
 payload:<base64url, no padding, encrypted envelope>}
```

Envelope bytes are bounded to 96 KiB; JSON request to 160 KiB. Same sender+id
and exact same payload are idempotent. Changed ciphertext under the same event
ID returns 409. Persist the ciphertext before the first POST and reuse it for
retries. A 201 response is committed to SQLite before it is sent and returns
`{id,createdAt,expiresAt}` in Unix milliseconds.

`GET /v1/rooms/{roomId}/events?recipient=host` (or join) returns `{events:[...]}`.
Each entry has id, sender, recipient, payload, createdAt, expiresAt. A page is
at most 64 events / approximately 1 MiB of base64 payload. Consume/ack then poll
again; do not delete an envelope until it is authenticated, validated and
persisted locally. `DELETE .../events/{eventId}?recipient=host` acknowledges it
idempotently. A malformed/unverifiable envelope must not mark a message delivered.

Event crypto (client): XChaCha20Poly1305, 32-byte event key
`HMAC-SHA256(seed,"twinlink/mailbox/events/v1")`. Bytes:
`[version=1:1 byte][random nonce:24 bytes][ciphertext + 16-byte tag]`.
AAD UTF-8:
`twinlink/mailbox/v1|{roomId}|{sender}|{recipient}|{eventId}`.
Generate a fresh random nonce for each new event, not for retrying a saved event.
`roomId` is always 64 lowercase hex, **not** a UUID. Only event/file IDs are UUIDs.

Suggested authenticated plaintext JSON variants (camelCase, validate strictly):

```text
{kind:"message", message:{id,text,timestamp}}
{kind:"receipt", messageId:<original message UUID>}
{kind:"offer", transfer:{id,name,size,hash:<plaintext SHA256 hex>}}
{kind:"fileComplete", id:<transfer UUID>}
{kind:"cancel", id:<transfer UUID>}
{kind:"reject", id:<transfer UUID>}
```

Incoming message IDs must be de-duplicated across direct and mailbox delivery.
After durable local storage, enqueue a persistent receipt before acking the
incoming envelope. Receiving a duplicate message should re-queue its receipt,
not show a duplicate bubble. `uploaded/stored` means the server accepted it;
`delivered` requires a direct authenticated ACK or a decrypted receipt.
No receipt implies no delivery claim. Local outbox remains on disk across restarts.

## Resumable encrypted files

File IDs are canonical random UUIDs, never filenames or remote paths. Encrypt
only an explicitly selected source. Plain chunk size 1,048,576 bytes; ciphertext
chunk size adds a 16-byte authentication tag. Empty files have one 16-byte chunk.
File key: `HMAC-SHA256(seed,"twinlink/mailbox/file/v1/" + fileId)`.
Nonce: UUID's 16 raw bytes followed by little-endian u64 chunk index (24 bytes).
AAD UTF-8:
`twinlink/file/v1|{roomId}|{fileId}|{index}|{plaintextSize}|{plaintextSha256}`.
`plaintextSize` and `plaintextSha256` above are the **total file plaintext size**
and **full-file plaintext SHA-256**, identical in every chunk's AAD, including
the empty file. They are not per-chunk values. The encrypted offer carries them.
The expected chunk plaintext length is derived separately from total size/index.
Every file ID/key must refer to one immutable source snapshot. **Never encrypt
different plaintext under a previously uploaded file ID/index.** A safe client
builds an immutable encrypted spool, verifies the complete source SHA-256 before
publishing any chunks, then retries/resumes using those exact cached bytes. If
the source changes before the spool is committed, discard it without uploading.
Do not automatically reread a changed file and reuse old nonces.

`POST /v1/rooms/{roomId}/files`:

```text
{id, sender:"host"|"join", encryptedSize:<total ciphertext bytes>, chunkCount}
```

Reserves the full size against the global 4 GiB quota before upload. Maximum
plaintext file is 1 GiB. Repeating an identical declaration returns the existing
manifest; a changed declaration returns 409. Manifest:

```text
{id,sender,encryptedSize,chunkCount,chunkBytes:1048592,
 received:[<indices already committed>],complete:false|true,expiresAt}
```

`GET .../files/{id}` returns that manifest. Resume by uploading missing indices.
`PUT .../files/{id}/chunks/{index}` with raw ciphertext body, Content-Length,
`Content-Type: application/octet-stream`, and `X-Sender-Role: host|join` matching
the owner. Size must equal declared chunk length. Chunks may arrive out of order.
Identical bytes are idempotent, different bytes for the same index return 409.
Response `{index,sha256:<ciphertext digest>}`. Use bounded memory, optionally
2–4 parallel chunk requests; no entire-file frontend IPC or memory buffering.

`POST .../files/{id}/complete` verifies all declared parts and marks the manifest
complete. **Only then enqueue the encrypted offer event**, so an incoming offer
always refers to an uploaded file. No source filenames or plaintext hashes go
into unencrypted file manifests.

`GET .../files/{id}/chunks/{index}` returns ciphertext only for complete uploads.
The receiver must not call this until the user explicitly accepts that offer.
Decrypt each bounded chunk, verify AAD/tag, and stream plaintext to a unique
local temp file; verify full SHA-256/size before no-clobber final persistence.
Discard corrupt partial downloads. Persist prior acceptance/download progress
to resume after interruption; otherwise ask for consent again. Never accept a
remote path, overwrite existing files, or materialize a file merely on receipt
of an offer. Do not claim progress beyond bytes actually uploaded/downloaded.

`DELETE .../files/{id}` idempotently deletes its encrypted parts and reservation.
Use it after recipient's successful persistence/receipt, or sender cancellation.
Retain local pending completion/cancellation events until acknowledged so a crash
does not leave a false peer status. Reject/cancel must not become success.

## Expiry, quotas, retries

Events and files expire 7 days after creation; a janitor removes expired events
and file data at least once per minute. This is server retention, not local chat
history deletion. A file that reaches expiry before upload completes cannot be
offered as available. Return 404 for missing/expired attachments; show expired
in the client instead of retrying that same object forever. Queue cap is 32 MiB
and 4,096 envelopes; storage reservations cap at 4 GiB; disk free space is also
checked. Use retry backoff for network/5xx, but explain terminal 4xx/507 states.
HTTP/TLS failures leave durable local outbox items pending; sending while the
peer is offline must not require an active EasyTier sender/channel.

Local unit verification: `python3 -m unittest discover -s server -v`. Tests use
temporary storage and loopback HTTP only. The deployed service always uses TLS.
