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
- Simple performance, window-size and audio controls
- Lean Gaming defaults: 2 CPU cores, 4 GB RAM, 720 × 1280 at 240 dpi, 30 Hz and host GPU
- Independent starts with no EmuMi device-count cap
- Stable Intel/Mesa rendering on hybrid-GPU Linux systems, with explicit experimental NVIDIA
  offload available per profile
- Quick Boot with exact-profile snapshot quarantine, one cold retry and stale-lock cleanup
- Persistent desired-device supervision with generation-safe cancellation, crash, boot-timeout and
  sustained ADB-loss recovery
- 30-second, 2-minute and 10-minute retry backoff followed by a 30-minute single-probe circuit
- Per-emulator Linux lifecycle scopes used only for exact process ownership and cleanup
- A single kernel-locked manager, atomic API endpoint publication and a non-blocking stop path
- Windowless automation by default while preserving ADB screenshots and input
- As many concurrently running Android devices as the host can sustain
- Live Android lifecycle and ADB state without host-hardware telemetry
- Automatic protection against runaway Android `netsimd` temp logs filling the host disk
- Automatic Android SDK, Java, ADB, emulator and KVM discovery
- Local lifecycle API for tools such as Frostguard
- Native Tauri window with a bundled React and Material UI interface
- Per-image compatibility runtimes stored privately in EmuMi's user data

## Requirements

- A Linux desktop with KVM virtualization available
- A working per-user systemd manager and `prlimit` from util-linux
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
sudo apt install ./emumi_1.0.1_amd64.deb
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
- `src/supervisor.rs` — persisted lifecycle generations, backoff and recovery circuits
- `src/snapshot_recovery.rs` — exact-profile failed-snapshot quarantine
- `src/model.rs` — shared application models
- `frontend/` — React, TypeScript and Material UI source
- `web/` — compiled interface bundled into the native binary
- `scripts/package-deb.sh` — reproducible Mint/Ubuntu release package

EmuMi is independent from Frostguard/WOSBot. Frostguard integration uses
EmuMi's local-only lifecycle API and stable ADB port assignments.

## Resource policy

EmuMi configures Android virtual hardware and leaves host CPU, memory and temperature management to
Linux, KVM and the computer firmware. It does not sample or report host hardware, and it does not
apply CPU quotas, scheduling weights, host-memory ceilings, thermal holds or resource-based launch
blocks.
The default Lean Gaming profile gives Android two virtual CPU cores and 4 GB of guest RAM, and users
can change those fixed launch settings when needed.

Every managed emulator still launches in an individual systemd user scope so EmuMi can terminate
only the process tree it owns. EmuMi does not impose a device-count ceiling; Android's supported
emulator-port range provides up to 64 stable slots. The Running page reports only Android lifecycle
and ADB state. Headless automation is the default because Frostguard operates
through ADB; users can still enable a visible window for interactive play.

Chrome and Google Play Store are protected packages. EmuMi enables and unsuspends both after every
managed boot, including Lean Gaming boots, and never adds either package to its optional Android
background-app cleanup. The separate Google Restore service may still be paused when the user keeps
the "Pause device restore during automation" option enabled.

Native emulator processes inherit a zero-byte core-dump limit through `prlimit`; a crash therefore
cannot create another multi-gigabyte systemd coredump. EmuMi retains the bounded per-device log,
cleans the exact failed scope and ADB transport, and retries through a persisted circuit breaker.
An intentional stop is recorded before any ADB or systemd cleanup starts, so a hung probe cannot
resurrect an older launch generation.

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
