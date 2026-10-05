# Elden Ring Ping Marker

A native DLL for Elden Ring 1.17.1 (`eldenring.exe` 2.7.1.0) that adds two things from Nightreign:

- **World marker.** Press `V` and a badge with a symbol appears at the point the camera looks at. It stays for a few seconds and is drawn in the game world, not on the map. Players in the same Seamless Co-op session see it too, as long as they run this mod.
- **Quick use.** Press `1` near an item on the ground: the mod picks it up and uses it right away instead of leaving it in the inventory.

All keys can be rebound in the settings window (`F7`) or in `er_ping_marker.ini`.

> Status: version 0.1.1 loaded in the game, the overlay worked and `V` placed a marker. Version 0.1.2 starts the overlay 10 seconds after the game loop begins, because the earlier start could crash the game in `sl.common.dll` (Streamline, shipped with ERSS-FG); 0.1.2 has not been run yet. Co-op delivery, quick use and the input block of the settings window (reworked in 0.1.1: the game can no longer move or lock the cursor while the window is open) have not been confirmed yet. See "Known limits".

## Install

The mod is loaded as a native by [me3](https://github.com/garyttierney/me3) or by Mod Engine 2.

1. Copy `release/er_ping_marker.dll` next to your other native mods, for example `mod/dll/`.
2. Add it to the profile.

   me3 (`*.me3`):

   ```toml
   [[natives]]
   path = './../mod/dll/er_ping_marker.dll'
   ```

   Mod Engine 2 (`config_eldenring.toml`):

   ```toml
   external_dlls = ["mod/dll/er_ping_marker.dll"]
   ```

3. Start the game offline or through Seamless Co-op. Do not use the mod with Easy Anti-Cheat enabled.

On the first start the mod writes `er_ping_marker.ini` and `er_ping_marker.log` next to the DLL.

## Keys

| Action | Default | Ini key |
|---|---|---|
| Place a marker | `V` | `PingKey` |
| Pick up and use | `1` | `QuickUseKey` |
| Settings window | `F7` | `MenuKey` |

To rebind in the game: open the settings window, click the button with the key name, press the new key. `Esc` cancels. Mouse buttons 3 to 5 work as well.

Quick use presses the game's own keys for you, so the mod has to know them. If you changed "interact" (`E`) or "use item" (`R`) in the Elden Ring keyboard settings, set the same keys in `GameInteractKey` and `GameUseItemKey`. Gamepad bindings are not supported for these two.

While the settings window is open, keyboard and mouse input does not reach the game.

## Settings

| Section | Key | Meaning |
|---|---|---|
| `[Marker]` | `Enabled` | Turn the marker off without removing the mod |
| | `Duration` | Lifetime in seconds, 2 to 60 |
| | `Symbol` | 0 pin, 1 exclamation mark, 2 arrow, 3 cross, 4 circle |
| | `Color` | `R,G,B`, 0 to 255 |
| | `ShowName`, `ShowDistance` | Player name and distance under the badge |
| | `Scale` | Badge size, 0.5 to 2.5 |
| | `MirrorX` | Set to 1 if markers show up mirrored left to right |
| `[QuickUse]` | `Enabled` | Turn quick use off |
| `[General]` | `Language` | `ru` or `en` |
| | `IgnoreWhenCursorVisible` | Ignore the two action keys while a game menu is open |
| | `Log` | Write `er_ping_marker.log` |
| `[Network]` | `Enabled` | Send markers to other players and show theirs |
| | `PeerDiscoveryHook` | Learn session members from the Steam messages the game sends |

## How it works

**Marker.** The mod stores a world position and projects it to the screen every frame with the game camera. The badge is drawn by an ImGui overlay on the DirectX 12 swap chain ([hudhook](https://github.com/veeenu/hudhook)). A marker that is off screen or behind the camera sticks to the screen edge.

**Co-op.** Markers travel over `ISteamNetworkingMessages` from the `steam_api64.dll` the game already loads, on a separate channel, so Seamless Co-op traffic is left alone. A packet carries the position, the symbol, the colour and the lifetime. Every player who should see markers needs the mod; without it the packets are ignored.

**Quick use.** The mod presses the interact key, waits for a new consumable to appear in the inventory, puts it into the selected quick slot for a moment, presses the use key and then restores the slot. Items that are not consumables are just picked up.

Game structures come from [fromsoftware-rs](https://github.com/vswarte/fromsoftware-rs).

## Known limits

- Confirmed in the game: loading, the overlay, placing a marker with `V`. Not confirmed: the network part, quick use and the input block of the settings window (it was reworked after the first test); send the log if something is off.
- The settings window blocks keyboard and mouse. Gamepad input may still reach the game.
- Co-op was written against Seamless Co-op and has not been tried with a second player.
- Quick use depends on the keyboard bindings above and does nothing for gamepad-only setups.
- Several overlays hooking the same swap chain (frame generation mods, HUD mods) can conflict. If the game does not start with this DLL, remove it from the profile and check the log.
- Built for 1.17.1. Other game versions may shift the structures the mod reads.

## Build

Rust with the `x86_64-pc-windows-gnu` toolchain and MinGW-w64 on `PATH`:

```
cargo build --release
```

The DLL appears in `target/x86_64-pc-windows-gnu/release/`. `.cargo/config.toml` links the C++ runtime statically, so the DLL needs nothing beyond system libraries.

## License

MIT, see `LICENSE`.

---

# Метка и быстрое использование (по-русски)

Нативная DLL для Elden Ring 1.17.1. Добавляет две механики из Nightreign:

- **Метка в мире.** По `V` в точке, куда смотрит камера, появляется значок с символом. Он держится несколько секунд и рисуется прямо в игре, а не на карте. Его видят и другие игроки сессии Seamless Co-op, если у них стоит этот же мод.
- **Быстрое использование.** По `1` рядом с лежащим предметом мод подбирает его и сразу применяет, не оставляя в инвентаре.

Клавиши меняются в окне настроек (`F7`) или в `er_ping_marker.ini`.

> Состояние: версия 0.1.1 загружалась в игре, оверлей работал, `V` ставила метку. Версия 0.1.2 подключает оверлей через 10 секунд после начала игрового цикла: ранний запуск мог ронять игру в `sl.common.dll` (Streamline из ERSS-FG); 0.1.2 в игре ещё не запускалась. Доставка меток в коопе, быстрое использование и блокировка ввода в окне настроек (переделана в 0.1.1: пока окно открыто, игра не может двигать и запирать курсор) пока не подтверждены.

## Установка

1. Скопируйте `release/er_ping_marker.dll` к остальным нативным модам, например в `mod/dll/`.
2. Добавьте в профиль me3:

   ```toml
   [[natives]]
   path = './../mod/dll/er_ping_marker.dll'
   ```

3. Запускайте игру без Easy Anti-Cheat: офлайн или через Seamless Co-op.

При первом запуске рядом с DLL появятся `er_ping_marker.ini` и `er_ping_marker.log`.

## Клавиши

| Действие | По умолчанию | Ключ в ini |
|---|---|---|
| Поставить метку | `V` | `PingKey` |
| Подобрать и применить | `1` | `QuickUseKey` |
| Окно настроек | `F7` | `MenuKey` |

Чтобы переназначить клавишу в игре, откройте окно настроек, нажмите кнопку с названием клавиши и затем новую клавишу. `Esc` отменяет ввод.

Быстрое использование нажимает за вас игровые клавиши. Если в настройках Elden Ring вы меняли «взаимодействие» (`E`) или «использовать предмет» (`R`), укажите те же клавиши в `GameInteractKey` и `GameUseItemKey`. С геймпадом эта часть не работает.

Пока окно настроек открыто, клавиатура и мышь до игры не доходят.

## Ограничения

- В игре подтверждено: загрузка, оверлей, постановка метки по `V`. Не подтверждено: сеть, быстрое использование и блокировка ввода в окне настроек (её переделали после первой проверки). Если что-то работает не так, пришлите лог.
- Окно настроек блокирует клавиатуру и мышь. Ввод с геймпада может доходить до игры.
- Сетевая часть написана под Seamless Co-op и со вторым игроком не испытывалась. Метки видят только игроки с модом.
- Если метки отражены слева направо, включите `MirrorX`.
- Несколько оверлеев на одной цепочке кадров (генерация кадров, моды интерфейса) могут конфликтовать. Если игра не запускается, уберите DLL из профиля и посмотрите лог.
