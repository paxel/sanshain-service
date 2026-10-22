# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.10.0]

### Removed
- The browser's hidden WebSocket mode for live updates (the `sanshain_use_ws` switch in local storage), which sent the session token in the URL and was always refused; the pages use the update stream with the `Authorization` header. `/api/ws/updates` stays for clients that send the header.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
