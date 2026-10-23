# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.11.0]

### Added
- Directory groups are added by searching LDAP for a part of their name, in the Groups section or right in the Maintainers picker, so a team like `carla` can maintain a Producer without knowing its DN (`GET /admin/directory-groups`, `POST /admin/groups/directory`).
- Every person has an avatar, a circle with their initials, in the header and the user list. A **Gravatar pictures** switch (`manage_settings`, off by default) shows their picture from gravatar.com instead, keyed by the SHA-256 of the e-mail address from LDAP `mail` or the OIDC `email` claim; the address itself is not stored.
- People have a **display name**, shown with the username in the user list, groups, maintainers, the token **For** choice, the audit log and the header: from LDAP `displayName` or the OIDC `name` claim at every sign-in, set by local users on their account page, and by user managers for service accounts.
- The admin user list tags every account with its kind: **service account**, or how the person last signed in (**local**, **LDAP**, **OIDC**); users who have not signed in since 2.11 show **unknown** (`kind` in `GET /admin/users`).
- The landing page is a dashboard: the number of Producers, Consumers and versions, the provides of the last 7 days and a chart of provides per day over 30 days, for everyone; a signed-in user also sees the 10 latest provides with their names.

### Changed
- The admin page is split into the tabs **General**, **Sign-in** and **Users**; the open tab is in the address (`#users`), the last one opens by default, and a tab without any of its permissions is greyed out with a hint.
- One picker for services, users and groups, used by the graph's Focus, the maintainers and group members on the admin page, and the token **For** choice: type to narrow, a single choice takes effect when picked, and multiple choices are ticked and unticked with immediate effect, without an accept step.
- The admin user list shows the accounts waiting for approval and a count of the others; a search by username or display name and chips for kind, status and role find the rest.
- The account page offers **Change Password** only after a sign-in with a Sanshain password, not after an LDAP or OIDC sign-in; `/auth/me` reports how the session signed in (`sign_in`).
- Signing in, locally or through OIDC, lands on the dashboard instead of the account page.
- The graph's **Periphery** switch is remembered in the browser like direction, Compact, Rainbow and Topics; a link that hides protocols still decides it.

### Fixed
- An endpoint's summary on a Producer's page sits under its path instead of floating between the path and the clients badge.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
