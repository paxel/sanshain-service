---
status: accepted
---

# GA content is immutable apart from line endings and BOM

A Provide's document is normalized before Sanshain does anything with it: a leading UTF-8 BOM is
removed, and CRLF and lone CR line endings become LF. Hashing, the idempotent no-op, GA
immutability, the compatibility checks, splitting and storage all see only the normalized text.
GA content stays immutable byte-for-byte, but the bytes are those of the normalized text. A
one-time step at startup rewrites everything stored before this rule existed.

## Why

GA immutability is there to catch the forgot-to-bump mistake: same number, different API. Line
endings and a BOM are not part of the API. They depend on which platform checked the file out. A
Producer whose Windows build re-provides a GA it first published from Linux changed nothing, yet
was refused with a `409` and told to bump a version it had no reason to bump. The `409` named the
likely cause, and `.gitattributes` could prevent it. But the rule was still wrong: the only
correct answer to that re-provide is the no-op.

The BOM is the same problem with a worse symptom. The YAML parser reads a leading BOM as content,
so a document saved by an editor that adds one was refused as unparsable, while the same file
without the BOM was fine.

Normalizing on the server, and not only in the official clients, means every upload path agrees:
raw HTTP, the clients, the admin upload and `/validate`.

## Considered and rejected

- **Compare normalized, store raw.** This keeps the stored bytes exactly as submitted. But the
  hash is part of the provide answer and the stored row, and two hashes for one document (raw
  for storage, normalized for comparison) is a contract nobody can predict. Snippets cut from raw
  proto text would also keep their carriage returns.
- **Leave stored data alone and normalize only new provides.** A GA stored with CRLF would then
  hash differently from its normalized re-provide, forever. The bug would stay for exactly the
  versions that caused it.
- **A SQL-only migration.** Rewriting the text is possible in SQL on both backends, but the
  content hash has to be recomputed too, and SQLite has no SHA-256 function. The rewrite therefore
  runs once in Rust at startup, the same way for both backends.

## Consequences

- Line endings and a BOM are no longer content. Whitespace, indentation and comments still are.
- `content_hash` is the SHA-256 of the normalized document. A client that wants to predict it
  has to normalize the same way first.
- **Existing GA bytes are rewritten once.** On the first start of a release with this rule, every
  stored document, endpoint snippet and channel-message contract payload is normalized, and every
  `content_hash` is recomputed. Row identity, stability, attribution and timestamps stay the same.
  OpenAPI and AsyncAPI snippets were always re-serialized with LF, so they don't change. gRPC
  snippets and full-document downloads cut from CRLF text now come with LF, so a Consumer's
  ETag or byte-comparing cache sees one change for them. The settings key `spec_text_normalized_at` records the run, and
  it never repeats. A run that fails stops startup, like a failed SQL migration, because a store
  left half normalized would refuse CRLF re-provides of old GA versions.
- The audit log is history and is not rewritten.
