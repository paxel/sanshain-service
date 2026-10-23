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

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
