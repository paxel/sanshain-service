# AI Migration Guide & Skill

Migrating your microservices to Sanshain can be significantly accelerated by using AI agents (like GitHub Copilot, Cursor, or Junie). This guide provides the prompts and "skills" you can give to your AI to automate the migration.

### TL;DR
1. **Identify**: Find where your service currently publishes its OpenAPI/AsyncAPI/Proto spec.
2. **Configure**: Create a `sanshain.yaml` in the project root.
3. **Automate**: Add a step to your CI pipeline to call `POST /provide`.
4. **Consume**: Update client code to use `GET /require` or the [official clients](clients.md).

---

## Junie Skill: Service Migration

If you are using Junie, you can copy the following block into a new skill file (e.g., `.junie/skills/sanshain-migration.md`) in your **target project** to help it migrate.

```markdown
# Sanshain Migration Skill

## Purpose
Automate the integration of Sanshain Service into this project for API contract management and dependency tracking.

## Procedures

### 1. Setup `sanshain.yaml`
- Detect the project name and use it as `serviceName`.
- Find existing API specifications (OpenAPI `openapi.yaml`, AsyncAPI, or `.proto` files).
- Create a `sanshain.yaml` file in the root. **Note**: Do not hardcode the `sanshainUrl` if possible; assume it's provided by the environment (`SANSHAIN_URL`).
  ```yaml
  serviceName: <project-name>
  provides:
    - file: <path-to-spec>
      apiType: <openapi|asyncapi|proto>
  ```

### 2. Provider Integration (CI/CD)
- Identify the CI provider (GitHub Actions, GitLab CI, Jenkins).
- Add a "Publish API Contract" step that runs after successful build/test.
- Use `curl` to call the Sanshain `/provide` endpoint with the spec content.
- Ensure the `SANSHAIN_TOKEN` secret is used for authentication.

### 3. Consumer Integration
- Identify external API dependencies (where does this project get its client stubs?).
- Add a `requires` section to `sanshain.yaml` for each dependency.
- Configure the Sanshain [official client](clients.md) to fetch these snippets during the build process.
```

---

## Migration Prompts for LLMs

You can use these prompts in ChatGPT, Claude, or Copilot to help with the manual parts of the migration.

### For a Provider Service
> "I want to migrate this [Java/Rust/Python] service to use Sanshain for API contract management. Can you help me create a `sanshain.yaml` file that points to my `src/main/resources/api.yaml`? Please follow corporate best practices: don't hardcode the URL in the YAML file, assume it comes from an environment variable. Also, write a GitHub Action step that uploads this spec using the `SANSHAIN_URL` and `SANSHAIN_TOKEN` secrets."

### For a Consumer Service
> "This project consumes the `UserService` and `OrderService`. I want to use Sanshain to fetch only the specific endpoints I need. Please create a `sanshain.yaml` (without hardcoded URL) with a `requires` section for `UserService` (GET /users/{id}) and `OrderService` (POST /orders). Then, show me how to configure the Sanshain [official client](clients.md) to download these."

---

## Benefits of AI-Assisted Migration
- **Consistency**: AI ensures that `serviceName` matches across your ecosystem.
- **Speed**: Automates the repetitive task of writing CI YAML and build plugin configurations.
- **Validation**: AI can check your existing OpenAPI files for compatibility with Sanshain's splitting logic before you even push.

---

## Related Links
- [**`sanshain.yaml` Reference**](sanshain-yaml.md) — Full configuration schema.
- [**Client Ecosystem**](clients.md) — Official Maven, Cargo, Go, JS, and Conan clients.
- [**CI Integration**](ci-integration.md) — Detailed pipeline examples.
