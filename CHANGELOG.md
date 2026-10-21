# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.9.0]

### Changed
- A snapshot can only be changed by the user who last provided it. Different content from anyone else is refused `409` (`snapshot_owned`) naming the provider and proposing the next free minor, so two developers who picked the same number no longer break each other's Consumer builds; identical bytes from anyone (CI rebuilding an already provided commit) stay a no-op, and an administrator deletes the snapshot to free the number.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
