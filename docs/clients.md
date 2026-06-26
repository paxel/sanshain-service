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
- **Config**: Defined in your `pom.xml`.

### Rust / Cargo
The [**cargo-sanshain**](https://github.com/paxel/sanshain) subcommand integrates Sanshain into the Rust ecosystem.
- **Installation**: `cargo install --git https://github.com/paxel/sanshain --path cargo-sanshain`.
- **Usage**: `cargo sanshain provide` and `cargo sanshain require`.
- **Config**: Uses `sanshain.yaml` in the crate root.

### Go CLI
The [**sanshain-go**](https://github.com/paxel/sanshain-go) tool provides a lightweight Go-based CLI.
- **Installation**: `go install github.com/paxel/sanshain-go/cmd/sanshain-go@latest`.
- **Features**: Automatic branch detection and native Go integration.
- **Config**: Uses `sanshain.yaml`.

### JavaScript / TypeScript
The [**sanshain-js**](https://github.com/paxel/sanshain-js) package provides a Node.js client, a CLI, and a GitHub Action.
- **GitHub Action**: Use `paxel/sanshain-js@main` in your workflows.
- **Installation**: Install directly from GitHub (not yet on NPM):
  ```bash
  npm install --save-dev github:paxel/sanshain-js
  ```
- **CLI**: `npx sanshain provide` (requires `sanshain.yaml`).

### Conan Plugin (C / C++)
The [**sanshain-conan**](https://github.com/paxel/sanshain-conan) plugin extends Conan to manage API specifications as dependencies.
- **Integration**: Uses `python_requires` in `conanfile.py`.
- **Workflow**: Runs during `conan install` to prepare build inputs.

---

## Configuration (`sanshain.yaml`)
Most clients (except Maven) share a common configuration format:

```yaml
serviceName: "my-service"
# sanshainUrl is omitted: provide via SANSHAIN_URL environment variable

provides:
  - file: "api/openapi.yaml"
    baseVersion: 1 # Optional: optimistic concurrency

requires:
  - serviceName: "auth-service"
    outputDirectory: "generated/api/auth"
    endpoints:
      - method: GET
        path: /api/v1/user
```

---

## Community & Custom Clients
If an official client doesn't exist for your language, you can:
1. Use the [**REST API**](api-usage.md) directly with `curl` or any HTTP client.
2. Build your own using the [`api.yaml`](../api.yaml) specification.
