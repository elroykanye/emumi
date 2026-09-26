# EmuMi product direction

EmuMi is a general-purpose Android emulator manager for Linux, not a
Frostguard-specific tool.

## Design rules

1. Profiles are the home screen.
2. Common choices use working presets and plain-language toggles.
3. Raw cores, memory, ports, GPU mode, and boot flags live under Advanced.
4. Setup problems are explained beside the path or dependency that needs work.
5. Logs and monitoring are readable without requiring a terminal.
6. Destructive profile operations require explicit confirmation.
7. The interface is bundled in a native Tauri/WebKit window, never an external browser tab.

## Planned delivery

- Prototype: discovery, creation, launch/stop, settings, logs, monitoring
- Profile management: clone, richer edit controls, snapshots
- Guided setup: download SDK components and accept licenses explicitly
- Distribution: self-contained user installation and Debian package
- Stable adapter surface for other apps to connect through ADB
