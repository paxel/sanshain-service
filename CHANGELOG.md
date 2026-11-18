# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.14.0]

### Added
- The release on GitHub carries a signed build provenance attestation for the binaries `sanshain-linux-x86_64` and `sanshain-linux-aarch64` and for the source SBOM, with its Sigstore bundle as `sanshain-service-<tag>-provenance.sigstore.json`; `gh attestation verify sanshain-linux-x86_64 -R paxel/sanshain-service` checks a download. The image holds those attested binaries; its cosign signature and SBOM attestation are as before ([san-1008](https://github.com/paxel/sanshain-service/issues/112)).

### Changed
- The Go and JavaScript client pages describe sanshain-go 2.6.0 (Go 1.26 or later) and sanshain 2.7.0 (Node 22.12 or later, also for the GitHub Action).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
