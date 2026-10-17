---
status: accepted
---

# Webhooks: facts and rejections, written with the fact, sent to allowed hosts only

Sanshain emits events over generic webhooks (#47). Four decisions shape them:

1. **Events are graph facts and rejections only.** `version.published`, `provide.rejected`,
   `version.deprecated`, `dependency.recorded`, `branch.cut`, `catalog.recorded` — nothing else.
2. **The event row is written in the same database transaction as the fact it describes**, with its
   fan-out already decided; a worker delivers it afterwards with retries.
3. **Webhooks are generic.** A hook is a URL and a secret; `json`, `slack` and `teams` are body
   shapes, not connectors.
4. **Receivers must be on an admin-managed egress allow-list.**

## Why

**Facts and rejections only.** The people who asked for webhooks want to know when what they
depend on changes, and when their own Provide was refused — the two things a build pipeline cannot
tell them in time. Admin and audit events (a user approved, a role granted, a token created) are a
different audience with a different tool already: the audit log. Keeping the catalogue to the
things the require graph and the version rules say keeps the event contract small enough to
document every field, and keeps a hook from becoming a second audit feed with a weaker access
check.

**Same transaction.** A webhook that fires before the fact is committed describes something that may
never exist; one that fires after the commit, from the request handler, is lost when the handler
dies, the receiver is down or the process restarts — exactly the cases a notification exists for.
The outbox pattern closes both gaps: the fact and its event commit together or not at all, and
delivery is a separate concern with its own retries. Fan-out is decided when the event is written,
not when it is delivered, so "who was requiring this Producer at the time" is the answer at that
time, and a hook registered later does not receive history.

How that lands on the repository: every write that is a fact takes the outbox rows as a parameter
and inserts them before its commit — `upsert_spec_version` (a field of `UpsertSpecVersion`),
`record_dependencies_bulk`, `insert_branch` and `reconcile_periphery` (an argument each). Two of
those writes were single statements before and are transactions now. This keeps the one
repository trait the application layer programs against; a transaction object handed across the
port would have exposed the database to every use case for the sake of four call sites. Two
events have no fact transaction to join and go to the outbox on their own: `provide.rejected` (a
rejection writes nothing but a best-effort audit row) and `catalog.recorded` for a file Sanshain
could not read (nothing is recorded, by design). A catalog push that is recorded writes the
periphery and the context in two statements already; its event rides the periphery's transaction.

The worker is a plain function with an injectable clock (`deliver_due(repo, sender, config, now)`),
so retries, backoff and the 24-hour cutoff are tested without sleeping, and `main` only loops over
it. On PostgreSQL the claim is `UPDATE … WHERE id IN (SELECT … FOR UPDATE SKIP LOCKED)`, which is
what makes two replicas safe; SQLite has one writer. A claim moves the row's next attempt forward
as a lease, so a pass that dies mid-send loses nothing for good.

**Generic webhooks.** A Slack app token, a Jira REST credential or a Confluence page would each be a
connector with its own secret, its own API surface and its own failure modes to carry in the
registry. A receiver — the chat tool's incoming webhook, n8n, Jenkins, a few lines of code — sits
behind a URL anyway. Offering the Slack and Teams *message shapes* removes the adapter most teams
would otherwise write, without taking on the connectors. Signing with HMAC-SHA256 over timestamp
and body, with the secret shown once like an API token, is what the receivers can verify and what
they already know from other systems.

**Egress allow-list.** Sanshain runs inside the cluster. A hook URL is caller-supplied, and a
`POST` from inside the cluster reaches the Kubernetes API, the metadata service and every internal
endpoint a Maintainer could not otherwise touch. Validating the URL's shape does not help; only an
explicit list of approved hosts does. An empty list refuses system hooks outright, so a fresh
install is safe before anyone has thought about it. Administrators with `manage_settings` may
register any host — they could reconfigure the list anyway — and such hooks stay exempt at
delivery, because the list is checked again at every delivery so that removing a host stops the
traffic to it.

## Considered and rejected

- **Firing from the request handler after the commit.** Simpler, and wrong under every failure that
  matters; see above.
- **A transaction handle on the port.** Would let any use case open a transaction and compose
  writes, but leaks the storage model into the application layer for four call sites and makes
  every mock carry transaction state.
- **Computing fan-out at delivery time.** Would make a late-registered hook receive events from
  before its time, and the answer to "who required this" drift with the graph.
- **Native connectors (Slack app, Jira, Confluence).** Credentials, rate limits and API drift of
  three products in the registry, for what a URL behind an automation tool does already.
- **Custom headers or bearer tokens on a hook.** Every one is a place to put a credential the
  registry then stores in clear; the signature covers authentication.
- **A URL deny-list (link-local, private ranges).** Lists what must not be reached and misses what
  nobody thought of; an allow-list lists what may be reached.
- **Admin and audit events.** The audit log is the record of those; a hook for them is a second
  copy with a weaker access check.
