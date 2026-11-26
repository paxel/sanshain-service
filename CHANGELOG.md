# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.18.0]

### Added
- A version shows the git branch it was built from, and where it merges to (`feature/master/login → master`): provides take `git_branch` and `git_target_branch` in the origin, a version keeps the branch it was born on, and the new Branch Names setting reads the target and the kind of branch from the company's naming patterns, with a test field ([san-1029](https://github.com/paxel/sanshain-service/issues/143)).

### Changed
- A refused provide from a release, bugfix or hotfix branch, or with a stream tag, is proposed a patch in its own minor (`2.6.0` gives `2.6.1`, not `2.24.0`); the maintenance kinds and an extra maintenance pattern are part of the Branch Names setting ([san-1030](https://github.com/paxel/sanshain-service/issues/144)).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
