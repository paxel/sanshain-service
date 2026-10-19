# Sanshain Service — Documentation

The user documentation is the site: **https://paxel.github.io/sanshain-service/** (Start,
Developer, Operator and Reference tracks; source under `site/`, built with `just site`; `just site-all`
builds it the way Pages publishes it, with every release since 2.6.1 under `vX.Y.Z/`).

What remains in `docs/` is not for the site:

- [`adr/`](adr/) — architecture decision records, one file per decision, numbered.
- [`arc42/`](arc42/) and [`architecture.yaml`](architecture.yaml) — the arc42 model and its
  whiteboxed export; the site's Architecture page is generated from them.
- [`agents/`](agents/) — instructions for coding agents: the issue tracker, triage labels, the
  domain docs layout.
- [`ai/`](ai/) — the longer-term roadmap (`plan.md`) and the client 2.2 rollout notes.
- [`ai-migration.md`](ai-migration.md) — using an AI assistant to migrate a service to Sanshain.
- [`developer-guide.md`](developer-guide.md) — service internals for contributors and auditors.
- [`reports/demo.md`](reports/demo.md) — a sample dependency report.

The two API contracts are at the repository root: `api.yaml` (Producers and Consumers) and
`maintenance.yaml` (the `/admin/*` surface).
