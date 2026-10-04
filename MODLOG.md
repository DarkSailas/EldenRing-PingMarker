# MODLOG

## Target

- Elden Ring 1.17.1, `eldenring.exe` 2.7.1.0, The Convergence, me3 0.13.0, Seamless Co-op (`ersc.dll`).
- Idea: Nightreign-style world marker on `V` visible to co-op players, pick up and use a ground item on `1`, rebindable keys.
- Done means: works in the game. Current state: loads, the overlay hooks, `V` places a marker (seen in the user's game). The input block was reworked on 2026-10-04 and is waiting for a run.

## Route

Native DLL loaded by me3 (`[[natives]]`). Rust cdylib.

- Game structures: `fromsoftware-rs` rev `59fbd3b3b7daaf14aca47c9f73530493dba6bc79` (`eldenring`, `fromsoftware-shared`).
- Overlay: hudhook 0.9.3, `dx12` feature, imgui 0.12.
- Game logic runs as a recurring task in `CSTaskGroupIndex::ChrIns_PostPhysics`. The task handle is leaked on purpose, dropping it unregisters the task.
- Network: flat API of the game's `steam_api64.dll`, `ISteamNetworkingMessages`, own channel `0x5047`, packet magic `ERPM`, version 1 (hello, ping).
- Peer discovery: session entries from the game, plus an optional hook on vtable slot 0 of `ISteamNetworkingMessages` (`SendMessageToUser`) that records who the game talks to.
- Input block for the settings window, four layers (see "Input block" below). Menu key is `F7`: `F5` and `F6` belong to VisualAtmosphere in the same profile.
- Quick use: `SendInput` scancodes for the game's interact key, watch the inventory for a new Goods item, swap it into the selected quick slot, send the use key, restore the slot.

## Build notes

- Toolchain `x86_64-pc-windows-gnu`, WinLibs MinGW-w64 on `PATH`.
- imgui is C++. By default the DLL imported `libstdc++-6.dll`, which the game does not have, so the native would fail to load. Fixed in `.cargo/config.toml`: `-static`, `+crt-static` and `CXXSTDLIB=static:-bundle=stdc++`. Check with `objdump -p er_ping_marker.dll | grep "DLL Name"`: only system DLLs must remain.
- Do not set `panic = "abort"`: panics are caught with `catch_unwind` and switch the mod off instead of crashing the game.

## Deployment on the dev machine

- DLL: `D:\Games\ConvergenceER\mod\dll\er_ping_marker.dll`.
- Profiles `convergence.me3` and `convergence - seamless.me3` got a `[[natives]]` entry. Copies from before the change: `D:\Games\ConvergenceER\_fds_backup\pingmarker_2026-10-04\`.
- Undo: restore both `.me3` files from that folder, or delete the two added lines.

## Input block

Failed first: patching vtable slots 9 and 10 of the DirectInput keyboard device plus hudhook's `MessageFilter::InputAll`. In the game the character kept moving and the mouse kept turning the camera while the window was open.

What the exe does: the global `FD4PadManager` pointer sits at RVA `0x4861d30` (2.7.1.0). Its update runs `if (m[0x2f9] || m[0x2f8]) { m[0x2f9] = m[0x2f8]; m[0x2f8] = 0; }`, and the input readers return early while `[mgr+0x2f9] != 0`. Writing 1 to both bytes every frame holds the block; it clears itself one frame after the writes stop. AOB for the pointer: `48 8B 05 ?? ?? ?? ?? 80 B8 F9 02 00 00 00 0F 85`, slot = match + 7 + disp32.

Current method:

1. Pad-manager flags, written every frame while the window is open.
2. Inline hook (ilhook) on the code of `GetDeviceState`, address taken from vtable slot 9 of throwaway keyboard and mouse devices. The buffer is zeroed only for sizes 256, 16 and 20, so other devices are untouched.
3. Window messages swallowed, cursor drawn by ImGui.
4. `ClipCursor(NULL)` every frame while the window is open.

Gamepad coverage is unknown.

## Not verified (needs the running game)

1. Projection: handedness of the camera matrix and whether fov is radians or degrees. `MirrorX` is the manual workaround.
2. Marker position across map blocks (`physics_center` offsets).
3. Steam IDs of session members under Seamless Co-op; whether the send hook slot is right.
4. Quick-slot swap: whether the game accepts the temporary item and the restore.
5. Cursor-visible heuristic for "a game menu is open".
6. Coexistence with other Present hooks: worked with PostureBarMod, ERSS-FG and spellwheel; VisualAtmosphere now adds one more and that set has not been run.
7. The reworked input block: character and camera must stay still, the mouse must work inside the window.
8. Anything with a second player.

## Next step

Start the game, open the window with `F7`, check item 7 first, then read `er_ping_marker.log` next to the DLL and go through the rest of the list. Builds from before the rework: `D:\Games\ConvergenceER\_fds_backup\standalone_menus_2026-10-04\`.
