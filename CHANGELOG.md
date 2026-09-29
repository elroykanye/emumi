# Changelog

All notable changes to EmuMi are documented here. This project follows
[Semantic Versioning](https://semver.org/).

## [0.2.1] - 2026-09-29

### Added

- 30, 45 and 60 Hz automation frame-rate controls, with 30 Hz as the Lean Gaming default.
- Adaptive sequential startup: later launch requests queue until the current Android has booted and
  host CPU has settled.
- Automatic NVIDIA PRIME render offload for hardware-rendered profiles on hybrid-GPU Linux hosts.
- Optional Play Store and restore-service suspension during automation, reversible on the next launch.
- Explicit queued, starting and ready lifecycle states in the UI and activity log.

### Changed

- Lean Gaming now uses a measured 6.5 GB soft host threshold, 7 GB hard ceiling and 1 GB swap
  allowance for a 4 GB Android guest.
- Profile configuration writes are idempotent so unchanged settings no longer invalidate Quick Boot.
- Cold boot is a one-shot recovery action; normal launches return to saved-state boot automatically.
- Compatibility system images remain immutable so launches avoid large temporary writable-system
  copies.

### Fixed

- Recover stale snapshot locks left by interrupted emulator processes.
- Use a unique systemd memory scope for every launch so a shared `netsimd` helper cannot block a
  stopped profile from restarting.
- Re-enable Play Store services when automation suspension is turned off.

## [0.2.0] - 2026-09-29

### Added

- Per-emulator systemd scopes with soft memory reclaim, hard memory and swap ceilings.
- Contention-friendly CPU and I/O scheduling weights for emulator processes.
- Per-device RSS, PSS, swap, cgroup pressure, limit-event and OOM monitoring.
- Optional headless automation and Vulkan compatibility controls.
- Device-specific emulator logs and validation for unsafe resource-limit combinations.

### Changed

- Lean Gaming enables host resource protection by default while retaining 2 CPU cores, 4 GB of
  Android memory, hardware graphics, muted audio and the Frostguard-friendly 720 x 1280 display.
- The Monitor and Settings pages now expose resource containment health and runtime evidence.

## [0.1.1] - 2026-09-27

### Fixed

- Select a private, per-system-image compatibility runtime when available
- Preserve each profile's previous writable system overlay before changing its runtime
- Keep profile userdata, apps and accounts isolated while applying native-bridge fixes

## [0.1.0] - 2026-09-27

### Added

- Native Linux manager for Android SDK emulator profiles
- Profile creation, deletion and account-preserving cloning
- Stable sequential ADB port assignment
- Lean Gaming preset with 2 CPU cores, 4 GB RAM and host GPU rendering
- Resolution, Android density, window scale, audio and display-shape controls
- Four-device concurrent execution limit
- Live device, CPU and memory monitoring
- Local lifecycle API for Frostguard and other loopback clients
- Mint/Ubuntu `.deb` release packaging

[0.1.0]: https://github.com/elroykanye/emumi/releases/tag/v0.1.0
[0.1.1]: https://github.com/elroykanye/emumi/releases/tag/v0.1.1
[0.2.0]: https://github.com/elroykanye/emumi/releases/tag/v0.2.0
[0.2.1]: https://github.com/elroykanye/emumi/releases/tag/v0.2.1
