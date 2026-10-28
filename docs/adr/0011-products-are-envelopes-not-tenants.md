---
status: accepted
---

# Products are envelopes over a shared namespace, not tenants

The roadmap carried a *Top-Layer System Switch*: partition one Sanshain instance into isolated
systems or organisations, which users would log in to. The need behind it is real — people want to
see and manage the services that make up one thing their company ships — but isolation answers the
wrong question. Which services belong together is usually already written down outside Sanshain,
in a Helm umbrella chart for example, and a microservice shared by several such things is the
normal case, not an exception. Divisions of a company that never touch each other already use
distinct names, so walls between them would protect nothing.

## Decision

1. **No isolation.** The Top-Layer System Switch is dropped. There is one namespace; nobody logs
   in to a part of the instance.
2. **A Product is an envelope.** A named set of systems, defined from outside and above. A system
   never declares, and never knows, which Products it is in, and one system may be in many. A
   member is a system by name and counts with whichever of its Producer and Consumer roles it has.
   The Producer's `domain` stays the graph cluster it is, and Backstage `spec.system` does not feed
   Products: both are the system describing itself, which is the direction this rules out.
3. **The envelope collects letters, it does not change them.** A Product passes no power over its
   members. A *Product Maintainer* may add and remove members of that Product and nothing else.
   Because membership grants nothing, letting a Product Maintainer edit it is no escalation, and
   overlapping Products cannot conflict. The Producer-scoped Maintainer and the global `releaser`
   role are unchanged.
4. **Administration.** Creating and deleting a Product needs global `ManageProducers`; Product
   Maintainers are assigned via `ManageRoles`, like Maintainers today. Product names are unique
   among Products only — the `product:` prefix and separate pages keep them apart from systems,
   and Sanshain could not refuse a first Provide because a Product already has that name.
5. **Two membership sources.** By hand, a multiselect over known systems; or by pasting a Helm
   umbrella `Chart.yaml` on an existing Product's page, whose `dependencies[].alias` (else `name`)
   are read as system names. The import applies directly — no preview — and reports what it added,
   removed and skipped.
6. **The current chart is the truth for what it lists.** Each member has exactly one origin, Helm
   or manual. A re-import adds every listed system Sanshain knows, including one removed by hand
   earlier and claiming one added by hand as Helm-sourced, and removes the Helm-sourced members the
   chart no longer lists. Manual members survive imports. Chart names that match no Producer and no
   Consumer are skipped and reported: umbrella charts are full of infrastructure subcharts that
   would otherwise sit in the Product as permanent noise; a service that builds later is picked up
   by the next import.
7. **Membership by name, without a foreign key.** As with the periphery (ADR-0007), a member whose
   system loses every role stays a member, shown as "not in Sanshain", and is live again when a
   system of that name returns. Deleting something elsewhere does not silently shrink a Product.
8. **First version is UI only.** A Product page lists its members, and a graph focus
   `product:<name>` narrows the drawing to the members and their neighbours, like `tag:<name>`.
   `api.yaml`, reports and release cuts are not touched; the Product routes belong to
   `maintenance.yaml`.

## Considered and rejected

- **Isolation in addition to Products.** Adds scoped queries and a login context everywhere for a
  separation that distinct names already give.
- **Product grants that cover the members.** Maintainer or releaser power over every member of a
  Product makes editing membership a way to grant oneself power, and two Products sharing a system
  would need conflict rules.
- **Pushed Product definitions from CI.** A pasted chart covers the need without an endpoint or
  pipeline wiring; a push can be added later as another source without changing the model.

## Not decided here

A branded banner or a dashboard on the Product page; Products in the Producer/Consumer API, in
reports or in release cuts; other envelope formats than a Helm `Chart.yaml`.
