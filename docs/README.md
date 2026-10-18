# Sanshain Service — Documentation

Detailed guides for users, administrators, and developers.

### TL;DR
- **New Users**: Start with [**Getting Started**](https://paxel.github.io/sanshain-service/start/getting-started.html).
- **Producers**: Learn how to [**Provide Specs**](https://paxel.github.io/sanshain-service/start/getting-started.html#the-first-provide-by-hand).
- **Consumers**: Learn how to [**Require Endpoints**](https://paxel.github.io/sanshain-service/start/getting-started.html#the-first-require-by-hand).
- **Automation**: Use [**CI Integration**](ci-integration.md) and [**`sanshain.yaml`**](https://paxel.github.io/sanshain-service/developer/sanshain-yaml.html).

---

## Guide Index

### Fundamentals
- [**Getting Started**](https://paxel.github.io/sanshain-service/start/getting-started.html) — Installation, first launch, and initial setup.
- [**User Guide**](https://paxel.github.io/sanshain-service/developer/web-ui.html) — Day-to-day usage: providing and requiring APIs.
- [**Client Ecosystem**](https://paxel.github.io/sanshain-service/developer/clients.html) — Detailed guide for Maven, Cargo, Go, JS, and Conan.
- [**API Lifecycle**](https://paxel.github.io/sanshain-service/developer/api-lifecycle.html) — Handling breaking changes and versioning.
- [**API Usage**](api-usage.md) — Authentication and detailed endpoint reference.
- [**Troubleshooting**](troubleshooting.md) — Solutions to common issues.
- [**Corporate Best Practices**](corporate-best-practices.md) — Recommendations for enterprise usage and [Maven-specific patterns](../../SanshainMaven/docs/corporate-usage.md).

### Integration & Configuration
- [**CI Integration**](ci-integration.md) — Automating contract validation in pipelines.
- [**`sanshain.yaml`**](https://paxel.github.io/sanshain-service/developer/sanshain-yaml.html) — Standard client configuration format.
- [**AI Migration**](ai-migration.md) — Using AI to migrate existing services to Sanshain.
- [**MCP**](https://paxel.github.io/sanshain-service/developer/mcp.html) — Letting AI assistants query the catalog over the Model Context Protocol: token setup, client configuration, the seven tools, limits.

### Administration & Operations
- [**Administration**](administration.md) — Managing users, versions, and LDAP.
- [**Configuration**](configuration.md) — Environment variables and system settings.
- [**Benchmarking**](benchmarking.md) — Performance results and how to run benchmarks.

### For Contributors
- [**Developer Guide**](developer-guide.md) — Internal architecture, splitting logic, and contributing.

---

## Client Ecosystem
Integrate Sanshain directly into your build tools:
- [**Maven Plugin**](https://github.com/paxel/sanshain-maven-plugin)
- [**Gradle Plugin**](https://github.com/paxel/sanshain-gradle-plugin)
- [**Cargo Plugin**](https://github.com/paxel/sanshain-cargo-plugin)
- [**CLI**](https://github.com/paxel/sanshain-cli)

## API contracts

- **`api.yaml`** — the contract Producers and Consumers speak: providing specifications and requiring
  endpoint snippets. This is what build plugins and CLIs are written against.
- **`maintenance.yaml`** — the `/admin/*` administrative surface, including the permission each
  endpoint requires. See [Administration](administration.md).
