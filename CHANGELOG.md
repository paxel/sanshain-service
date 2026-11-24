# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.16.0]

### Added
- A Remove button on a Producer group of the Consumer page removes every dependency of that Consumer on that Producer, in the dev and the main graph ([san-1018](https://github.com/paxel/sanshain-service/issues/129)).

### Changed
- A trunk require or bundle replaces the Consumer's other dependencies on that Producer and API type: an endpoint the Consumer stopped requiring, or an old Pin, leaves the main and the dev graph with the next trunk build instead of after 90 or 30 days ([san-1018](https://github.com/paxel/sanshain-service/issues/129)).
- An AsyncAPI provide with `subscribe` or `receive` operations no longer logs the warning "Skipping N SUB operation(s)"; those operations are harvested and reported in the provide response ([san-1017](https://github.com/paxel/sanshain-service/issues/128)).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
