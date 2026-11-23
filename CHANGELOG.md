# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.15.0]

### Added
- A version diff on the Producers page has its own URL: the address bar names it once it is shown, and a link to a version that no longer exists says so ([san-1014](https://github.com/paxel/sanshain-service/issues/121)).

### Changed
- The complete example on the sanshain.yaml page shows `autoGa: false` and what `true` does ([san-1015](https://github.com/paxel/sanshain-service/issues/122)).
- The client pages list their options as one block per option, with environment variable, sanshain.yaml field, default and version beneath the name, instead of a six-column table ([san-1016](https://github.com/paxel/sanshain-service/issues/123)).

### Security
- Updated `cookie` to 0.18.3 (RUSTSEC-2026-0594, private cookie nonces can repeat in processes that fork) and `url` to 2.5.9 (RUSTSEC-2026-0592, stack overflow in `Url::origin()` on nested `blob:` URLs).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
