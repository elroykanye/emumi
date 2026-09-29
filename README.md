# EmuMi

[![CI](https://github.com/elroykanye/emumi/actions/workflows/ci.yml/badge.svg)](https://github.com/elroykanye/emumi/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/elroykanye/emumi)](https://github.com/elroykanye/emumi/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**Android emulators, made easy on Linux.**

EmuMi is a focused desktop manager for Android Virtual Devices. It provides a
small, readable interface for creating, cloning, configuring, starting and
stopping Android profiles without requiring Android Studio.

## Features

- Create, clone and remove Android emulator profiles
- Preserve apps, accounts and Android data when cloning a stopped profile
- Stable sequential ADB ports (`5554`, `5556`, `5558`, ...)
- Simple performance, resolution, window-size and audio controls
- Lean Gaming defaults: 2 CPU cores, 4 GB RAM, 720 × 1280 at 240 dpi, 30 Hz and host GPU
- Adaptive sequential startup instead of simultaneous emulator boot storms
- Automatic NVIDIA PRIME render offload on supported hybrid-GPU Linux systems
- Quick Boot with one-shot cold recovery and stale-lock cleanup
- Per-emulator Linux scopes with memory/swap ceilings and contention-friendly CPU/I/O priority
- Optional windowless automation while preserving ADB screenshots and input
- Up to four concurrently running Android devices
- Live device state plus host CPU and memory monitoring
- Automatic Android SDK, Java, ADB, emulator and KVM discovery
- Local lifecycle API for tools such as Frostguard
- Native Tauri window with a bundled React and Material UI interface
- Per-image compatibility runtimes stored privately in EmuMi's user data

## Requirements

- A Linux desktop with KVM virtualization available
- Android SDK command-line tools, Emulator and an x86_64 system image
- Java 17 or newer
- WebKitGTK 4.1 and GTK 3 runtime libraries

Android Studio is not required. EmuMi uses the standalone Android SDK tools.

On Ubuntu or Linux Mint, the desktop runtime libraries can be installed with:

```bash
sudo apt install libwebkit2gtk-4.1-0 libjavascriptcoregtk-4.1-0 libgtk-3-0t64
```

## Install a release

Download the `.deb` from the latest GitHub release and install it with:

```bash
sudo apt install ./emumi_0.2.1_amd64.deb
```

EmuMi then appears in the desktop application menu.

## Build from source

The compiled frontend is checked into `web/`, so Node.js is not needed unless
you are modifying the interface:

```bash
cargo build --release
```

To rebuild the frontend, use Node.js 24:

```bash
cd frontend
npm install
npm run build
cd ..
cargo build --release
```

Create the Debian package with:

```bash
./scripts/package-deb.sh
```

## Project layout

- `src/app.rs` — private loopback API and application state
- `src/android.rs` — Android SDK, emulator and ADB integration
- `src/config.rs` — XDG user configuration persistence
- `src/monitor.rs` — Linux host monitoring
- `src/model.rs` — shared application models
- `frontend/` — React, TypeScript and Material UI source
- `web/` — compiled interface bundled into the native binary
- `scripts/package-deb.sh` — reproducible Mint/Ubuntu release package

EmuMi is independent from Frostguard/WOSBot. Frostguard integration uses
EmuMi's local-only lifecycle API and stable ADB port assignments.

## Resource policy

Lean Gaming profiles launch in an individual systemd user scope. Android receives two virtual
CPU cores and 4 GB of guest RAM, while Linux begins reclaiming host memory above 6.5 GB and applies
a 7 GB last-resort ceiling plus a 1 GB swap ceiling. Emulator CPU and disk work use a lower weight
than normal desktop applications, helping Linux stay responsive when several devices are busy.

These controls do not pretend that a 4 GB Android guest consumes no memory. The Monitor page shows
each emulator's resident/proportional memory, swap, scope limits and pressure events so limits can
be tuned from evidence. Headless automation is optional because it removes the emulator window;
visible profiles retain the same resource policy.

## Compatibility runtimes

EmuMi can select a locally prepared compatibility system image from
`$XDG_DATA_HOME/emumi/runtime`, or `~/.local/share/emumi/runtime` when
`XDG_DATA_HOME` is unset. The Android SDK's original system image is never
modified.

Every profile continues to use its own userdata disk. Compatibility system
images are launched as immutable private copies, which avoids a large temporary
writable-system copy and keeps Quick Boot reliable. Apps, Google accounts and
game data stay in the profile's separate userdata image.

Locally modified Android system images require verified boot to be disabled
inside the emulator guest. This does not change Linux host security.

## License

EmuMi is available under the [MIT License](LICENSE).

Security issues should be reported according to [SECURITY.md](SECURITY.md).
Contributions are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md).
