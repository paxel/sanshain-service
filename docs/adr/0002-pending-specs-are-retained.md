---
status: superseded by ADR-0003
---

# A refused Provide is retained in full, and approving it applies what was submitted

Outside Onboarding, a Provide carrying a breaking change on a protected branch is **held**, not
discarded. Sanshain stores the submitted spec as a Pending spec — one per Producer, branch and API
type, latest replacing previous — and an approver reviews the actual content and accepts it, which
applies exactly that spec. A successful Provide for the same key discards whatever was held.

## Why

Refusal used to destroy the thing a human would need in order to overrule it. The audit log recorded
that *something* was rejected and a sentence about why, but the spec itself was gone. So there was
no way to look at a refused change and decide it was fine: the only route through was to delete the
branch, which threw away that branch's entire version history as a side effect, or to make the
Producer deprecate first and push again.

Retaining the spec is what makes review a real decision rather than a rubber stamp on a one-line
reason. It also costs nothing in confidentiality: distributing API descriptions is what this service
is *for*, so a stored spec is not a secret being accumulated.

Holding is universal outside Onboarding rather than opt-in. A gate would have to be configured per
Producer, and the pushes in question come from CI with no special handling — an opt-in nobody sets
leaves the operator seeing bare refusals, which is the situation being fixed.

## Considered and rejected

- **A one-shot waiver.** Approval grants an allowance and the Producer re-pushes against it. Stores
  no payload and needs no retention policy, but the approver decides without seeing what they are
  approving, and the change lands only when someone else's build runs again.
- **A queue of every refused attempt.** Full forensic history, but CI pushing on each commit files
  the same problem repeatedly, and the entry eventually reviewed is stale by many pushes. One entry
  per key, latest wins, keeps what is held at most one push old.
- **Answering 202 instead of 409.** Honest about "received, not yet applied", and it turns the
  Producer's build green. Rejected because the spec is *not* live: the build would pass while every
  Consumer still resolves the old API, silently, until someone acts — possibly never.

## Consequences

- Sanshain retains spec bodies indefinitely for held entries. The bound is structural rather than a
  cleanup job: one row per Producer/branch/API type, replaced on each new refusal and deleted when a
  Provide succeeds or an approver decides.
- Replay skips the stale-base-version check. That check stops a Producer overwriting work it had not
  seen; an approver applying a spec they have just read is a different act, and failing after review
  would make approval unreliable for reasons the approver cannot see.
- The refusal audit action is replaced by one recording that a spec was quarantined, plus actions for
  the accept and reject decisions. A flat refusal no longer occurs outside the cases where nothing
  was submitted to hold.
- Approval is Producer-scoped: an admin may act anywhere, a Maintainer only on Producers they
  maintain. There is deliberately no second-pair-of-eyes rule, so a Maintainer may approve a change
  they pushed themselves.
