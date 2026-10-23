# Changelog

All notable changes to EmuMi are documented here. This project follows
[Semantic Versioning](https://semver.org/).

## [1.0.0] - 2026-10-23

The first supported EmuMi release establishes a simple, stable Android device manager for Linux.

### Added

- Create, clone, configure, start and stop up to four Android emulator profiles.
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
- Allow up to four devices to start concurrently without resource-based admission blocks or staged
  startup delays.

### Fixed

- Stop clears desired-running intent before terminating the exact ADB transport and owned process
  scope, preventing stale watchers from restarting a deliberately stopped device.
- Recover stale lifecycle state, failed snapshots and abandoned emulator processes without broad
  process-name cleanup.

[1.0.0]: https://github.com/elroykanye/emumi/releases/tag/v1.0.0
