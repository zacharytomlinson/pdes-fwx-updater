# PDES Firmware Updater

[![Linux Deb Build](https://github.com/zacharytomlinson/pdes-fwx-updater/actions/workflows/linux-deb.yml/badge.svg)](https://github.com/zacharytomlinson/pdes-fwx-updater/actions/workflows/linux-deb.yml)
[![Release (Windows + Linux)](https://github.com/zacharytomlinson/pdes-fwx-updater/actions/workflows/release.yml/badge.svg)](https://github.com/zacharytomlinson/pdes-fwx-updater/actions/workflows/release.yml)

Cross-platform desktop firmware updater built with egui/eframe. Use the Linux deb workflow for Debian packages and the release workflow for Windows MSI + Linux artifacts.

## Bootstrap installs (device token)

For first-run bootstrap, mint a short-lived `bootstrap_pkg_url` with `max_uses >= 2` (recommended 5–10 for retries), then embed it into the installer payload as `seed-config.json` (or set `PDES_PKG_URL` during packaging). The app derives the API base URL from `bootstrap_pkg_url`, exchanges it for a long-lived device token, stores the device token in the per-user updates directory, and removes `bootstrap_pkg_url` from the persisted config.

- Mint helper scripts: `scripts/linux/mint-bootstrap-url.sh:1`, `scripts/windows/mint-bootstrap-url.ps1:1`
- Device token storage: `src/device_token.rs:5`
