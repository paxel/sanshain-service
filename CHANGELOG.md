# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.6.0]

### Fixed
- Viewing an endpoint's spec works again. Previously (1.5.2) `yaml.html` and `edit.html` requested `/admin/endpoints/yaml`, but the route is `/admin/endpoint-yaml`, so that fetch always returned 404. A branch with no recorded version history — which includes every freshly created service, since history is only kept on protected branches — therefore reported *"This endpoint isn't available on the server"* even though the spec had been published successfully. Master appeared unaffected only because it renders from version history rather than that fetch. The endpoint editor was loading an empty document for the same reason.
- Deleting an endpoint on a branch is no longer undone by inheritance. Previously (1.5.2) a request for an endpoint a branch did not carry was answered from a protected branch whenever the requested branch was not itself protected — without checking whether that branch had published an API of its own. A producer that deprecated an endpoint on `master` and removed it on a feature branch still served `master`'s copy to consumers of that branch, in the UI *and in their builds*, so the removal silently did not take effect. See [ADR 0001](docs/adr/0001-endpoint-resolution-model.md).
- A branch's own published spec is now shown in preference to an ancestor's recorded history. Previously (1.5.2) a feature branch that had published its own spec but had no version rows of its own (only protected branches record history) displayed `master`'s history under its own name — an API that branch did not serve.
- Browsing no longer creates services or branches. Previously (1.5.2) opening a branch's endpoint list called `ensure_service`/`ensure_branch`, so a mistyped or stale URL permanently created an empty branch that then appeared in the services overview and in stale-branch cleanup.
- The Admin page's **Database Configuration** panel now shows the backend and connection URL. Previously (1.5.2) it requested `/admin/settings/database`, which was never implemented, and both fields showed `—`. The URL is credential-stripped: the password is removed from the connection string and from any `password`-style query parameter, so it is never sent to the browser.

### Changed
- Every endpoint lookup now reports **how** it was resolved, and which branch answered. A branch that has published a spec is *authoritative*: that spec is the complete statement of its API, so an endpoint missing from it is missing deliberately. Only a branch that has never published inherits from another branch.
  - `/require`, `/require/asyncapi`, `/require/grpc` and `/require-bundle` return `X-Sanshain-Resolution` (`published` or `inherited`) and `X-Sanshain-Served-Branch` alongside the unchanged YAML body.
  - Requesting an endpoint that an authoritative branch deliberately does not publish returns **`410 Gone`**, immediately, even when `timeout` is set — no later publish of the current spec can change the answer. `404` continues to mean no branch has the endpoint, and remains the only state a long-poll waits on. A protected branch that has not published yet also answers `404`, so a consumer building before the producer's first push still waits rather than failing.
  - The unmet dependency is still recorded for a `410`, so a consumer requiring an endpoint its producer has dropped still appears in the dependency views.
  - `GET /admin/endpoint-yaml` now returns a JSON object (`state`, `served_branch`, `version`, `yaml`, `deprecated`, `external`) instead of a bare YAML body, so the UI is told what happened instead of inferring it from an empty response.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
