# Changelog

All notable changes to EmuMi are documented here. This project follows
[Semantic Versioning](https://semver.org/).

## [1.0.1] - 2026-10-29

This patch release makes EmuMi obey explicit device intent and keeps Android management focused,
predictable and safe.

### Changed

- Remove host CPU, memory and temperature sampling, resource warnings and the Monitor page. EmuMi
  now reports only Android lifecycle and ADB state and leaves host resource policy to Linux.
- Replace the Monitor page with a focused Running page that lists active Android devices.

### Fixed

- Treat a Ready emulator closed outside EmuMi as an explicit stop, persist that state and never
  relaunch it until a user or API client sends a new Start request.
- Apply a notchless hardware identity before every rectangular-display launch so Android does not
  reserve a Pixel camera-cutout safe area.
- Treat Chrome and Google Play Store as protected packages: every managed boot enables and
  unsuspends both, and Lean Gaming never includes either in its optional package cleanup.
- Keep manager state, stop handling and cleanup responsive without collecting host telemetry.

## [1.0.0] - 2026-10-23

The first supported EmuMi release establishes a simple, stable Android device manager for Linux.

### Added

- Create, clone, configure, start and stop as many Android emulator profiles as the host can sustain,
  within Android's supported emulator-port range.
- Use fixed ADB ports, a 720 x 1280 automation canvas, hardware graphics, muted audio and optional
  headless operation for reliable automation.
- Persist the devices the user asked to keep running and supervise native crashes, boot timeouts and
  sustained ADB loss with exact per-device cleanup and bounded recovery.
- Keep per-launch systemd scopes for process ownership and cleanup without imposing CPU, memory or
  I/O throttles on emulator workloads.
- Prevent multi-gigabyte emulator core dumps and bound runaway `netsimd` logs.

### Changed

- Let Linux, KVM and firmware manage CPU, memory and temperature instead of implementing a second
  resource scheduler inside EmuMi.
- Do not impose an EmuMi device-count ceiling or resource-based admission blocks; report host
  pressure as warnings and let the user decide whether to start another device.

### Fixed

- Stop clears desired-running intent before terminating the exact ADB transport and owned process
  scope, preventing stale watchers from restarting a deliberately stopped device.
- Recover stale lifecycle state, failed snapshots and abandoned emulator processes without broad
  process-name cleanup.

[1.0.1]: https://github.com/elroykanye/emumi/releases/tag/v1.0.1
[1.0.0]: https://github.com/elroykanye/emumi/releases/tag/v1.0.0
