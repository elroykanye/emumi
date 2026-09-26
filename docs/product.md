# EmuMi product direction

EmuMi is a general-purpose Android emulator manager for Linux, not a
Frostguard-specific tool.

## Design rules

1. Profiles are the home screen.
2. Common choices use friendly presets: speed, picture, and window.
3. Raw cores, memory, DPI, ports, GPU mode, and boot flags live under Advanced.
4. Setup problems are explained beside the path or dependency that needs work.
5. Logs and monitoring are readable without requiring a terminal.
6. Destructive profile operations require explicit confirmation.

## Planned delivery

- Prototype: discovery, launch/stop, settings, logs, monitoring
- Profile management: create, clone, edit, delete, snapshots
- Guided setup: download SDK components and accept licenses explicitly
- Distribution: self-contained user installation and Debian package
- Stable adapter surface for other apps to connect through ADB

