# Client Ecosystem

Sanshain is designed to be integrated directly into your build process. We provide official clients and plugins for major languages and build tools.

### TL;DR
- **Java/Kotlin**: Use the [Maven Plugin](https://github.com/paxel/sanshain-maven-plugin).
- **Rust**: Use the [Cargo Plugin](https://github.com/paxel/sanshain).
- **Go**: Use the [Go CLI](https://github.com/paxel/sanshain-go).
- **JS/TS**: Use the [NPM Client](https://github.com/paxel/sanshain-js).
- **C/C++**: Use the [Conan Plugin](https://github.com/paxel/sanshain-conan).
- **CI/CD**: Most clients have built-in support for GitHub Actions and GitLab CI.

---

## Official Clients

### Maven Plugin (Java / Kotlin)
The [**sanshain-maven-plugin**](https://github.com/paxel/sanshain-maven-plugin) is the recommended way to integrate Sanshain into Java or Kotlin projects.
- **Goals**: `sanshain:provide` and `sanshain:require`.
- **Integration**: Typically bound to `generate-sources` and `deploy` phases.
- **Corporate Usage**: See the [**Maven Corporate Guide**](../../SanshainMaven/docs/corporate-usage.md) for `settings.xml` and CI/CD patterns.
- **Config**: Defined in your `pom.xml`.

### Rust / Cargo
The [**cargo-sanshain**](https://github.com/paxel/sanshain-rs) subcommand integrates Sanshain into the Rust ecosystem, built on the [**sanshain**](https://github.com/paxel/sanshain-rs) client library.
- **Installation**: `cargo install cargo-sanshain`.
- **Usage**: `cargo sanshain provide` and `cargo sanshain require`.
- **Config**: Uses `sanshain.yaml` in the crate root.

### Go CLI
The [**sanshain-go**](https://github.com/paxel/sanshain-go) tool provides a lightweight Go-based CLI.
- **Installation**: `go install github.com/paxel/sanshain-go/cmd/sanshain-go@latest`.
- **Features**: Native Go integration. Stability is `snapshot` unless the pipeline sets the GA switch (`SANSHAIN_GA=true` or `--ga`) — there is no branch detection.
- **Config**: Uses `sanshain.yaml`.

### JavaScript / TypeScript
The [**sanshain**](https://github.com/paxel/sanshain-js) npm package provides a Node.js client, a CLI, and a GitHub Action.
- **GitHub Action**: Use `paxel/sanshain-js@main` in your workflows.
- **Installation**: `npm install --save-dev sanshain`.
- **CLI**: `npx sanshain provide` (requires `sanshain.yaml`).

### Conan Plugin (C / C++)
The [**sanshain-conan**](https://github.com/paxel/sanshain-conan) PyPI package extends Conan to manage API specifications as dependencies.
- **Installation**: `pip install sanshain-conan` into the environment Conan runs in.
- **Integration**: `from sanshainconan import Sanshain` in `conanfile.py` — the former `python_requires` pattern is gone.
- **Workflow**: Runs during `conan install` to prepare build inputs.

---

## Configuration (`sanshain.yaml`)
Every official client reads the same `sanshain.yaml` from the project root — the Maven plugin accepts `pom.xml` parameters as overrides, but the file is the shared format:

```yaml
serviceName: "my-service"
# sanshainUrl is omitted: provide via SANSHAIN_URL environment variable

provides:
  - file: "api/openapi.yaml" # version is read from its info.version

requires:
  - serviceName: "auth-service"
    version: "1.2.0" # exact pin, MAJOR.MINOR.PATCH
    outputDirectory: "generated/api/auth"
    endpoints:
      - method: GET
        path: /api/v1/user
```

> ⚠️ **AsyncAPI 2.x perspective convention.** Sanshain reads `publish`/`subscribe` from the
> **application's** perspective: `publish` means *the providing service publishes to the channel*,
> `subscribe` means *it consumes the channel*. The official AsyncAPI 2.x specification defines
> those keywords from the client's perspective — exactly inverted. A 2.x document authored with
> the spec-literal reading registers its contracts, and harvests its subscriptions, exactly
> backwards. Details and the 3.x `send`/`receive` mapping:
> [sanshain-yaml.md](sanshain-yaml.md#asyncapi-version-compatibility-v2--v3).

---

## Community & Custom Clients
If an official client doesn't exist for your language, you can:
1. Use the [**REST API**](api-usage.md) directly with `curl` or any HTTP client.
2. Build your own using the [`api.yaml`](../api.yaml) specification.
