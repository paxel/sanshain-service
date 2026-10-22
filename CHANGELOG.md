# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.10.0]

### Added
- A **Structure** view in the spec viewer, next to the YAML and the patch: an OpenAPI or AsyncAPI version as a foldable tree of endpoints with their parameters, request body, responses or messages, every `$ref` opened in place, `allOf` merged (each member naming the part it came from), `oneOf`/`anyOf` as variants with the discriminator, and per member its type, required and other flags, description, default, example, allowed values and constraints. Every node names the version that last changed it, and a folded node says when something below it changed later. Each body has a copyable JSON example, the spec's own or one built from the schema. The choice between YAML and Structure is remembered in the browser.
- The MCP tool `endpoint_structure` answers the same tree for one endpoint, with `with_examples` for a ready JSON body per request and response.

### Removed
- The browser's hidden WebSocket mode for live updates (the `sanshain_use_ws` switch in local storage), which sent the session token in the URL and was always refused; the pages use the update stream with the `Authorization` header. `/api/ws/updates` stays for clients that send the header.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
