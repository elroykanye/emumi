# Changelog

All notable changes to EmuMi are documented here. This project follows
[Semantic Versioning](https://semver.org/).

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
