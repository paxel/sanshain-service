# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.10.0]

### Added
- **Off** is maintenance with a way in: the API answers 503, and only holders of `manage_auth_config` sign in, through the mode Off interrupted (LDAP or OIDC) or a local password. Everyone else is refused with the reason, and their open sessions pause until Off ends.
- `INITIAL_AUTH_MODE` (Helm: `config.initialAuthMode`) picks the auth mode of the very first start.
- A Registration switch for Local mode, next to Auto-acknowledge; registering answers whether the account can sign in at once.
- An OIDC form and mode on the admin page.
- Service accounts for automation: an administrator creates one (e.g. `jenkins`) without a password, and every holder of `manage_users` issues, sees and revokes its API tokens on the account page, each marked with who issued it.
- A **Structure** view in the spec viewer, next to the YAML and the patch: an OpenAPI or AsyncAPI version as a foldable tree of endpoints with their parameters, request body, responses or messages, every `$ref` opened in place, `allOf` merged (each member naming the part it came from), `oneOf`/`anyOf` as variants with the discriminator, and per member its type, required and other flags, description, default, example, allowed values and constraints. Every node names the version that last changed it, and a folded node says when something below it changed later. Each body has a copyable JSON example, the spec's own or one built from the schema. The choice between YAML and Structure is remembered in the browser.
- A filter over the endpoints of a Producer's version and of a Consumer: every word typed matches the path, the method or the summary, chips narrow to used, unused or deprecated endpoints (only deprecated for a Consumer), and the filter stays in the address so Back and shared links keep it. A Consumer's endpoints now show their summaries.
- The MCP tool `endpoint_structure` answers the same tree for one endpoint, with `with_examples` for a ready JSON body per request and response.

### Changed
- **Self-registration is closed after this update**: Local mode has a Registration switch, off by default, also on instances updated from 2.9.
- A Producer's version lists its endpoints by path, then method, instead of in the order the store happened to return them.
- A fresh instance starts in Off, where only the admin signs in and picks the mode, instead of in Local with open registration.
- An instance whose stored mode is Dev but whose server runs without `ALLOW_INSECURE_DEV_MODE` is switched to Local on startup, which is how it has been working.
- Dev mode opens provide, require, catalog and MCP only: the admin pages need a signed-in administrator, and only administrators sign in. On a server without `ALLOW_INSECURE_DEV_MODE` the Dev option is greyed out and switching to it is refused instead of silently doing nothing.
- Rainbow colours the graph by position, so the callers in view get colours as far apart as possible instead of mostly blue-greens, and it colours the catalog's dependsOn lines too.
- The sign-in page shows the Register tab only while registration is open, does not promise a wait when new accounts are acknowledged automatically, shows the SSO button only while OIDC is the mode, and says when only administrators can sign in.

### Removed
- The browser's hidden WebSocket mode for live updates (the `sanshain_use_ws` switch in local storage), which sent the session token in the URL and was always refused; the pages use the update stream with the `Authorization` header. `/api/ws/updates` stays for clients that send the header.

### Fixed
- Domain frames in the graph no longer overlap: each domain's nodes are laid out as a block of their own.
- The graph's Topics switch draws the channels a producer publishes that nobody requires yet.
- Under LDAP, a local password is checked before the directory, so a user whose local and directory accounts share a name no longer locks their directory account by typing the local password.

### Security
- The `dev_user` account that dev mode creates no longer has a password: it carried a fixed one written in the source and signed in as an admin on any instance that had ever been in dev mode. An existing `dev_user` is locked on startup.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
