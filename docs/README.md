# Sanshain Service — Documentation

Detailed guides for users, administrators, and developers.

### TL;DR
- **New Users**: Start with [**Getting Started**](getting-started.md).
- **Producers**: Learn how to [**Provide Specs**](user-guide.md#providing-a-spec).
- **Consumers**: Learn how to [**Require Endpoints**](user-guide.md#requiring-an-endpoint).
- **Automation**: Use [**CI Integration**](ci-integration.md) and [**`sanshain.yaml`**](sanshain-yaml.md).

---

## Guide Index

### Fundamentals
- [**Getting Started**](getting-started.md) — Installation, first launch, and initial setup.
- [**User Guide**](user-guide.md) — Day-to-day usage: providing and requiring APIs.
- [**Client Ecosystem**](clients.md) — Detailed guide for Maven, Cargo, Go, JS, and Conan.
- [**API Lifecycle**](api-lifecycle.md) — Handling breaking changes and versioning.
- [**API Usage**](api-usage.md) — Authentication and detailed endpoint reference.
- [**Troubleshooting**](troubleshooting.md) — Solutions to common issues.
- [**Corporate Best Practices**](corporate-best-practices.md) — Recommendations for enterprise usage and [Maven-specific patterns](../../SanshainMaven/docs/corporate-usage.md).

### Integration & Configuration
- [**CI Integration**](ci-integration.md) — Automating contract validation in pipelines.
- [**`sanshain.yaml`**](sanshain-yaml.md) — Standard client configuration format.
- [**AI Migration**](ai-migration.md) — Using AI to migrate existing services to Sanshain.

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
