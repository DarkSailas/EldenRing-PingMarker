# MODLOG

## Target

- Elden Ring 1.17.1, `eldenring.exe` 2.7.1.0, The Convergence, me3 0.13.0, Seamless Co-op (`ersc.dll`).
- Idea: Nightreign-style world marker on `V` visible to co-op players, pick up and use a ground item on `1`, rebindable keys.
- Done means: works in the game. Current state: builds, deployed, not yet run.

## Route

Native DLL loaded by me3 (`[[natives]]`). Rust cdylib.

- Game structures: `fromsoftware-rs` rev `59fbd3b3b7daaf14aca47c9f73530493dba6bc79` (`eldenring`, `fromsoftware-shared`).
- Overlay: hudhook 0.9.3, `dx12` feature, imgui 0.12.
- Game logic runs as a recurring task in `CSTaskGroupIndex::ChrIns_PostPhysics`. The task handle is leaked on purpose, dropping it unregisters the task.
- Network: flat API of the game's `steam_api64.dll`, `ISteamNetworkingMessages`, own channel `0x5047`, packet magic `ERPM`, version 1 (hello, ping).
- Peer discovery: session entries from the game, plus an optional hook on vtable slot 0 of `ISteamNetworkingMessages` (`SendMessageToUser`) that records who the game talks to.
- Input block for the settings window: vtable slots 9 (`GetDeviceState`) and 10 (`GetDeviceData`) of the DirectInput keyboard device, ANSI and Unicode.
- Quick use: `SendInput` scancodes for the game's interact key, watch the inventory for a new Goods item, swap it into the selected quick slot, send the use key, restore the slot.

## Build notes

- Toolchain `x86_64-pc-windows-gnu`, WinLibs MinGW-w64 on `PATH`.
- imgui is C++. By default the DLL imported `libstdc++-6.dll`, which the game does not have, so the native would fail to load. Fixed in `.cargo/config.toml`: `-static`, `+crt-static` and `CXXSTDLIB=static:-bundle=stdc++`. Check with `objdump -p er_ping_marker.dll | grep "DLL Name"`: only system DLLs must remain.
- Do not set `panic = "abort"`: panics are caught with `catch_unwind` and switch the mod off instead of crashing the game.

## Deployment on the dev machine

- DLL: `D:\Games\ConvergenceER\mod\dll\er_ping_marker.dll`.
- Profiles `convergence.me3` and `convergence - seamless.me3` got a `[[natives]]` entry. Copies from before the change: `D:\Games\ConvergenceER\_fds_backup\pingmarker_2026-10-04\`.
- Undo: restore both `.me3` files from that folder, or delete the two added lines.

## Not verified (needs the running game)

1. Projection: handedness of the camera matrix and whether fov is radians or degrees. `MirrorX` is the manual workaround.
2. Marker position across map blocks (`physics_center` offsets).
3. Steam IDs of session members under Seamless Co-op; whether the send hook slot is right.
4. Quick-slot swap: whether the game accepts the temporary item and the restore.
5. Cursor-visible heuristic for "a game menu is open".
6. Coexistence with other Present hooks in the same profile (PostureBarMod, ERSS-FG, spellwheel).
7. Mouse and keyboard inside the ImGui window.
8. Anything with a second player.

## Next step

Start the game, read `er_ping_marker.log` next to the DLL, go through the list above in order.
