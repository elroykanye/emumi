# Intel hardware renderer trial

Device settings now offer **Hardware · Intel (Mesa)** (`gpu_mode: host-intel`).
This is opt-in per profile; existing hardware choices are unchanged. Lean Gaming
preserves an explicitly selected Intel renderer. FrostGuard's normal EmuMi start
endpoint uses the saved choice too.

The launcher still passes `-gpu host`, but selects Mesa/Intel for OpenGL and limits
Vulkan to the installed Intel ICD. Selecting OpenGL alone is insufficient: a live
trial selected Intel OpenGL while gfxstream still selected NVIDIA Vulkan.
Missing Intel hardware or its Vulkan manifest causes an explicit launch error,
not intentional fallback to NVIDIA or software. Always verify the actual adapter
in the emulator boot log; driver initialization can still fail.

The Android system image, writable-system compatibility overlay, guest memory,
accounts and userdata are not changed by this choice.

2026-10-03: Device 1 booted using Intel UHD 630 for both APIs and Whiteout reached
the existing city. This is an initial trial, not evidence of long-term stability
or multiple-device performance. A host-window capture also verified that the game
was presented correctly despite startup drawable warnings. No claim of permanent
NVIDIA driver repair is made.

The normal `/api/ports/5554/start` path was then tested with the saved Intel
choice: both adapters remained Intel, readiness returned true, and Whiteout
reached the city again. Final guest display: 720x1280, 240 dpi. The post-manager
restart game showed an upper blank band not present in the direct-launch trial;
display normalization/guest insets still need separate investigation before Bear
coordinate accuracy can be accepted. No rallies or other game actions were sent.

All 26 Rust tests and the frontend build passed. Package 0.2.5 was built and its
checksum verified, but system installation requires the user's sudo password.
The running manager is the local build; the system-installed executable is still
old until installation. Do not start the old executable with `host-intel` settings.

A short final host sample showed active swap traffic and 13–23% I/O wait despite
free host RAM. Device 1 retains its existing 5120 MiB MemoryHigh / 6144 MiB
MemoryMax scope. This is not evidence that Intel has solved resource efficiency;
inspect cgroup reclaim counters before changing memory policy.
