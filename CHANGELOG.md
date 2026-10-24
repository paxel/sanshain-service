# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.11.0]

### Added
- Producers can be marked **external**: a provide on the admin page marks a new Producer, a switch in the admin list corrects it, and the graph draws it light grey.
- Directory groups are found by searching LDAP for part of their name, in Groups and in the Maintainers picker.
- People get an initials avatar, and optionally their Gravatar picture behind a **Gravatar pictures** switch that is off by default, from their directory address or one they choose on the account page.
- People have a **display name**, taken from LDAP or OIDC, set by local users themselves and by user managers for service accounts.
- The admin user list tags each account as **service account**, **local**, **LDAP**, **OIDC** or **unknown**.
- The landing page is a dashboard of counts and provides per day, with the 10 latest provides for signed-in users.

### Changed
- The admin page is split into the tabs **General**, **Sign-in** and **Users**.
- One picker for services, users and groups applies a choice at once, and ticks and unticks multiple choices without an accept step.
- The admin user list shows the accounts waiting for approval, and finds the others by search and chips.
- **Change Password** is offered only after a sign-in with a Sanshain password.
- Signing in lands on the dashboard instead of the account page.
- The graph's **Periphery** switch is remembered in the browser.

### Fixed
- An LDAP user signing in with both short name and e-mail address keeps one account, named by the new **Username attribute**, and existing duplicates are merged at the next sign-in.
- A version's **build** link is shown only when it came from the trunk build, so it no longer points to a deleted branch build.
- An endpoint's summary on a Producer's page sits under its path.

### Security
- Updated `ctutils` to 0.4.5 (RUSTSEC-2026-0385), `tokio` to 1.54.1 (RUSTSEC-2026-0386, unsound `AsyncFd` constructors that Sanshain does not call) and `chacha20` to 0.10.2 (0.10.0 was yanked).
- `cargo audit` ignores RUSTSEC-2026-0387 (timing variability in `crypto-bigint` 0.5.5): it comes in through `openidconnect` 4.0.1, whose newest release still needs it, and Sanshain uses it only to verify the OIDC provider's ID token signatures with a public key, so no secret can leak; the ignore goes once `openidconnect` moves on ([#74](https://github.com/paxel/sanshain-service/issues/74)).
- `cargo audit` still ignores RUSTSEC-2023-0071 (Marvin attack in `rsa`): no fix exists, and `rsa` comes in through `openidconnect`, which only verifies RS256 ID token signatures with the provider's public key, while the attack needs RSA decryption with a private key that Sanshain does not have.
- Two dependencies are unmaintained without a known vulnerability: `unsafe-libyaml` (RUSTSEC-2024-0449), which parses every received spec through `serde_yaml_ng`, and `basic-toml` (RUSTSEC-2026-0374), used only at build time by the page templates; replacing them is part of [#74](https://github.com/paxel/sanshain-service/issues/74).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
