# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.6.1]

### Added
- A documentation site with search, index and light and dark themes, built with `just site` and published to GitHub Pages on every release; the Markdown guides moved into it, re-checked against this release, and were deleted. Its REST and maintenance references are generated from the two contracts at every build, so they cannot drift; the arc42 architecture view is generated from the whiteboxed export the same way, every box but the User a link; two recorded walkthroughs (a team's story and an operator's story) show the real answers of this release step by step, re-recordable with `just record-walkthroughs`.

### Fixed
- `CLEANUP_INTERVAL_SECS=0` is refused at startup with a message naming the variable, instead of crashing the service right after it logged its start.
- The Helm chart and the Kustomize manifests no longer set `PROMETHEUS_ENDPOINT` and `LOGIN_SESSION_DURATION_HOURS`, two variables the service never read.
- `autoscaling.enabled` in the Helm chart now renders a HorizontalPodAutoscaler from `minReplicas`, `maxReplicas` and the CPU and memory targets; before, those values existed but no autoscaler was created.
- The Helm chart gained `extraEnv` for every variable its ConfigMap does not render (`PUBLIC_BASE_URL`, the webhook, MCP, OTEL and LDAP timeout settings and more), as `name`/`value` or `valueFrom` entries.
- The startup log line for a claimed `SANSHAIN_ROOT_USERS` name no longer tells the operator to reset a password that does not exist; it says the placeholder is taken over by the first LDAP or OIDC login under that name.
- `maintenance.yaml` documents `201` for granting a role, which is what the route has always answered; the generated reference follows.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
