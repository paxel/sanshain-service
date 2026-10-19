# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.8.0]

### Added
- **Test Connection** appends a network diagnosis when the directory could not be reached: the addresses the name resolves to, the container's resolvers, and per address an ICMP echo, the kernel's route lookup and a TCP probe of the port (`diagnostics` in the `400` body, shown under the message on the admin page). On Kubernetes the echo line says when unprivileged ping is not permitted in the pod.
- The documentation site keeps every release: the version chip in the header is a menu that opens the same chapter in any release since 2.6.1 (under `/vX.Y.Z/`, the latest also at the root), pages of an older release carry a pointer to the latest one, and the changelog link of a release points at that release's notes.

### Changed
- An LDAP server that cannot be reached is explained in plain words with where to look: name not resolved, no route from this container, network unreachable, port closed, or no answer within 5 s, instead of the kernel's `No route to host (os error 113)` alone. TLS failures name the cause the same way (untrusted certificate with the `EXTRA_CA_CERTS_DIR` hint, wrong hostname, expired, plain port behind `ldaps://`, refused handshake).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
