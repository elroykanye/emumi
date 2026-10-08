# Changelog

All notable changes to EmuMi are documented here. This project follows
[Semantic Versioning](https://semver.org/).

## [0.2.8] - 2026-10-08

### Fixed

- Generic hardware rendering now prefers Intel/Mesa on Intel+NVIDIA hybrid systems instead of
  automatically forcing the confirmed NVIDIA GLX/gfxstream crash path. NVIDIA remains available
  as an explicit experimental profile choice and stays automatic on NVIDIA-only systems.
- Replace the fixed post-start CPU ceiling with adaptive per-scope control. Repeated cgroup
  throttling raises a busy emulator in 60% steps up to 300%, while calm periods lower it slowly.
- Share six logical CPUs across active emulators on an eight-CPU host, preserving two logical CPUs
  for Frostguard and the desktop. Thermal or host saturation prevents quota increases.
- Re-adopt surviving emulator scopes after an EmuMi manager restart and continue from each
  scope's real quota instead of replaying the startup default.

### Changed

- Emulator scopes start conservatively, then adapt to measured throttling, host load and CPU
  temperature instead of following a fixed post-start quota timer.

## [0.2.7] - 2026-10-07

### Changed

- New and previously unspecified profiles use Intel/Mesa hardware rendering with Vulkan disabled
  by default, avoiding the NVIDIA gfxstream crash path observed on hybrid-GPU Linux systems.
- Explicit per-profile renderer and Vulkan choices are preserved when applying Lean Gaming or
  migrating older configuration.

### Fixed

- Rectangular-display normalization now discovers and disables display-shape overlays from all
  Android overlay targets, including `com.android.systemui.emulation.*`, so Pixel-style notches do
  not reappear after boot.
## [0.2.6] - 2026-10-06

### Added

- Host launch admission checks require at least 8 GB of available memory and a CPU temperature
  below 90 C before another emulator starts.
- Live available-memory and CPU-temperature readings on the Monitor page.

### Changed

- All emulator launches now use Frostguard's fixed 720 x 1280 automation canvas.
- New and existing profiles default once to headless operation, muted audio, disabled front/back
  cameras, no boot animation, two virtual CPUs and 4 GB of guest RAM.
- Four-device scopes now use an 80% per-emulator CPU quota, lower CPU and I/O weights, a 5.5 GB
  soft memory threshold, a 6.5 GB hard ceiling and a 1 GB swap ceiling.
- Sequential starts wait at least 20 seconds after Android boots before admitting the next device.

## [0.2.5] - 2026-10-03

### Added

- Opt-in per-device Intel/Mesa hardware rendering for both OpenGL and Vulkan,
  retained across EmuMi/FrostGuard starts and Lean Gaming preset selection.
  Existing hardware defaults are unchanged. This is an alternative renderer,
  not a claim that NVIDIA driver crashes have been permanently fixed.

## [0.2.4] - 2026-10-02

### Fixed

- Profiles with a compatibility runtime boot that runtime again. Since 0.2.1 the emulator was
  ignoring it and opening the stock SDK system image instead, so the patched native bridge never loaded
  and Whiteout Survival crashed on Android 36 (`Unknown x86_64 sa_restorer in host sigaction`).

## [0.2.3] - 2026-10-02

### Added

- A per-port readiness endpoint so automation clients can distinguish an ADB-visible emulator from
  an Android instance that has completed EmuMi's clean-boot warm-up.

### Fixed

- Repeated start requests no longer erase a profile's in-progress startup state.
- A stale watcher from an older boot can no longer mark a newer restart ready.
- Frostguard can now defer Whiteout launch until the emulator is explicitly launch-ready.

## [0.2.2] - 2026-10-02

### Changed

- Lean Gaming profiles always perform a clean Android boot without deleting userdata, installed
  apps or accounts, preventing native games from inheriting an invalid saved-memory state.

### Fixed

- Continue through the sequential startup queue when one profile fails before launch instead of
  leaving every later profile stranded.

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
[0.2.2]: https://github.com/elroykanye/emumi/releases/tag/v0.2.2
[0.2.3]: https://github.com/elroykanye/emumi/releases/tag/v0.2.3
[0.2.4]: https://github.com/elroykanye/emumi/releases/tag/v0.2.4
[0.2.5]: https://github.com/elroykanye/emumi/releases/tag/v0.2.5
[0.2.6]: https://github.com/elroykanye/emumi/releases/tag/v0.2.6
[0.2.7]: https://github.com/elroykanye/emumi/releases/tag/v0.2.7
[0.2.8]: https://github.com/elroykanye/emumi/releases/tag/v0.2.8
