# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.18.0]

### Added
- A version shows the git branch it was built from, and where it merges to (`feature/master/login → master`): provides take `git_branch` and `git_target_branch` in the origin, a version keeps the branch it was born on, and the new Branch Names setting reads the target and the kind of branch from the company's naming patterns, with a test field ([san-1029](https://github.com/paxel/sanshain-service/issues/143)).
- The Producer page lists, per version, the endpoints it added, changed, removed or newly deprecated against the highest GA below it, and the spec viewer folds each version open to the same list ([san-1031](https://github.com/paxel/sanshain-service/issues/145)).
- The Consumer page shows per Pin the newer versions on only the endpoints that Consumer requires, and proposes the newest one where all of them keep their shape; the require answer names it in the new header `X-Sanshain-Upgrade` ([san-1031](https://github.com/paxel/sanshain-service/issues/145)).
- A Consumer's `requires` section is versioned: requires send `requires_hash`, and each new hash from a trunk or tag require gets the Consumer's next number with the Pins it holds; the Consumer page shows them in a strip on top, and the Producer page a strip of who pins each version, in main, dev or a sanshain-branch ([san-1032](https://github.com/paxel/sanshain-service/issues/146)).
- Product releases: a Helm pipeline pushes the umbrella `Chart.yaml` (`PUT /products/{name}/chart`, or the paste on the Product page) and each chart version becomes a snapshot with its members' chart versions, a sales name from the `sanshain/sales-name` annotation and the graph scope it is read from; the release sheet shows each member's API versions, require-set version and Pins, marks Pins that do not match what the release ships, exports CSV and prints, and two releases can be compared ([san-1033](https://github.com/paxel/sanshain-service/issues/147)).

### Changed
- A pasted chart with a version also creates its release, and only the highest chart version sets the Product's members; the paste answers the snapshot beside the member lists ([san-1033](https://github.com/paxel/sanshain-service/issues/147)).
- A refused provide from a release, bugfix or hotfix branch, or with a stream tag, is proposed a patch in its own minor (`2.6.0` gives `2.6.1`, not `2.24.0`); the maintenance kinds and an extra maintenance pattern are part of the Branch Names setting ([san-1030](https://github.com/paxel/sanshain-service/issues/144)).
- The statistics sankey "Dependencies between domains" draws each domain in its graph colour on both sides, its flows in the colour of their source, and the systems without a domain in grey ([san-1036](https://github.com/paxel/sanshain-service/issues/151)).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
