# EmuMi

**Android emulators, made easy.**

EmuMi is a friendly Linux desktop manager for Android Virtual Devices. It keeps
the everyday controls obvious, while preserving detailed emulator controls in a
collapsed Advanced section.

## Current prototype

- Discovers Android SDK, Java, AVD profiles, ADB devices, and KVM
- Shows profile status and host CPU/RAM monitoring
- Starts and stops emulator profiles through the official Android Emulator
- Stores SDK/JDK paths in the user's XDG configuration directory
- Keeps an in-app activity log

## Run from source

```bash
cargo run
```

Android Studio is not required. EmuMi uses the standalone Android SDK command-
line tools and Emulator package.

## Project layout

- `src/app.rs` — native egui desktop interface
- `src/android.rs` — SDK discovery and emulator/ADB adapter
- `src/config.rs` — user configuration persistence
- `src/monitor.rs` — lightweight Linux host monitoring
- `src/model.rs` — shared application models

This repository is intentionally independent from Frostguard/WOSBot.

