# FreeShift

Free Windows app that turns Zwift Click buttons, SRAM AXS shifters and other Bluetooth LE buttons into virtual gear shifts in MyWhoosh. No account, no subscription, no telemetry. MIT licence.

Not affiliated with Zwift, SRAM or MyWhoosh.

## Install

Run `installer\FreeShift-Setup.exe`. It installs for your user only (no admin), adds Start-menu and desktop shortcuts and an uninstaller (Settings → Apps). Closing the window keeps FreeShift in the tray; use the tray icon's **Quit** to stop it. **Start with Windows** is a switch in the app.

## How it talks to MyWhoosh

| Output | How | When to use |
|---|---|---|
| **Network** (default) | FreeShift advertises itself as an OpenBikeControl controller on your LAN (mDNS `_openbikecontrol._tcp`, TCP port 21587) and sends Shift Up / Shift Down messages | MyWhoosh on this PC or any device on the same network. No window focus needed |
| **Keyboard keys** | Brings the window whose title starts with `MyWhoosh` to the front and presses the key set for each action (I / K for shifting) | If MyWhoosh doesn't list FreeShift as a controller |

If Windows asks whether FreeShift may use the network, allow **private** networks.

## Supported controllers

| Device | Status |
|---|---|
| Zwift Click (v1) | `RideOn` handshake, Plus / Minus |
| Zwift Click v2, right pod | Plus, A, B, Y, Z; keep-alive every 3 s, idle pod reset at 55 s |
| Zwift Click v2, left pod | Works only while Zwift's own app has unlocked it in the last ~24 h |
| Zwift Ride | Same keypad message as Click v2, untested |
| Zwift Play | Not supported (encrypted) |
| SRAM AXS shifters | Through the rear derailleur: left and right paddle, single and double click, see below |
| Any other BLE button | "Custom trigger": any notification on one characteristic = tap, two quick ones = double tap |

### SRAM AXS

FreeShift connects to the rear derailleur, which reports every paddle press and which shifter it came from.

- **Left and right paddles** each get their own action (left shifts down and right shifts up by default).
- **Double clicks** on either side can have their own action. They are off by default, so single clicks act at once; with one set, single clicks on that side wait for the double-tap time (350 ms).
- **Pairing inside FreeShift**, without Windows Bluetooth settings or an account. If BikeControl is set up on the same PC, FreeShift uses the key it saved.
- **Reconnects by itself** when the drivetrain wakes up. The key stays on this PC in `%APPDATA%\FreeShift\sram_keys.json`.
- **Battery level** of the derailleur in the app.
- **Live drawing** of the shifters that lights up on presses.

To pair, wake the drivetrain with a paddle press. When **Activity** says the derailleur isn't paired, hold the AXS button on the rear derailleur until its light flashes and keep holding until Activity says it is paired.

The paddles also keep moving the derailleur unless another app has turned that off. Pairing gives the derailleur a new key, so an app that paired with it before has to pair again.

### Buttons and actions

The **Controller** card shows both Click v2 pods, and the SRAM shifters while a derailleur is connected. Buttons light up when you press them; click one to pick its action: shift up / down, menu navigation (Up, Down, Left, Right, OK, Back, Menu, Home), steering, U-turn, power-up, emote, camera, HUD, map and workout controls. These are the OpenBikeControl button IDs; when MyWhoosh reports which ones it accepts, the rest are dimmed. In keyboard mode each action gets its own key.

Default mapping: Plus → Shift up, Minus → Shift down, B → Shift down (so a right-pod-only Click v2 has both directions), A → OK, Y → Menu, Z → Back, Up / Down → menu up / down, Left / Right → steer, Tap → up, Double tap → down, SRAM left → Shift down, SRAM right → Shift up. **Swap shift up / down** flips only the gear actions.

Button actions, keys and the double-tap time apply when you press **Save**, without dropping any connection. Changes to the connection settings show **Save and restart**.

## Hardware test checklist

Do these in order and report the results (copy the lines from **Activity** or from the command output).

1. **Wake and connect.** Start FreeShift, press a Click button. Report: which model **Controllers** shows (Zwift Click / Click v2 right / left) and the battery %. Do *not* pair the Click in Windows Bluetooth settings.
2. **Buttons.** Press each Click button once. Report: the name that lights up in **Buttons** for each, and whether any press shows nothing.
3. **MyWhoosh (network).** Open MyWhoosh's controller pairing screen. Report: does "FreeShift" appear, and does the pill turn green ("Trainer app connected")? Then in a ride: Plus → gear goes up or down?
4. **Direction.** If gears go the wrong way, press **Swap up / down** and recheck.
5. **Idle test (Click v2).** Leave the Click untouched for 3 minutes, then press. Report: still shifts? Any "Disconnected" lines?
6. **Keyboard fallback** (only if step 3 fails). Output → Keyboard keys → Save. Report: does I shift up or down?
7. **SRAM AXS.** Wake the drivetrain with a paddle press and pair it as described above. Press left three times, then right three times. Report: the Activity lines, whether each press lights up the right paddle and acts in MyWhoosh, and whether the derailleur also moves.

## Command-line tool

Download `freeshift-cli.exe` from the latest release, or build it with `cargo build -p freeshift-cli --release` (it lands in `target\release`):

| Command | What it does |
|---|---|
| `freeshift-cli run` | Same engine as the app, printing every event |
| `freeshift-cli scan 10` | List nearby BLE devices with manufacturer / service data |
| `freeshift-cli explore <name> 60` | Read-only GATT dump of one device, then print every notification |
| `freeshift-cli obc-test` | Advertise to MyWhoosh and shift by typing `u` / `d` (tests MyWhoosh without a Click) |
| `freeshift-cli obc-probe 20` | Act like a trainer app: find a controller over mDNS, connect, print what it sends |
| `freeshift-cli key-test I 5` | Press I in MyWhoosh after 5 s |
| `freeshift-cli swap` / `output obc\|keys` / `keys I K` / `trigger <name> <uuid>` | Edit settings |

Settings live in `%APPDATA%\FreeShift\config.json` and SRAM keys in `sram_keys.json` next to it; the app and the CLI share them.

## Build from source

Toolchain used here (no admin needed):

```
winget install Rustlang.Rustup
rustup toolchain install stable-x86_64-pc-windows-gnu
winget install BrechtSanders.WinLibs.POSIX.UCRT --scope user
```

Then, from this folder (it pins the GNU toolchain with `rustup override`):

```
cargo test -p freeshift-core
cd app
npm install
npx tauri build
```

The installer lands in `target\release\bundle\nsis\FreeShift_<version>_x64-setup.exe`. The exe depends only on Windows system DLLs plus `WebView2Loader.dll`, which the installer ships; WebView2 itself is part of Windows 11. `app\src-tauri\build.rs` stops MinGW from linking its default manifest so Tauri's manifest (Common Controls v6) is the only one.

Building with MSVC instead also works: install Visual Studio Build Tools with the C++ workload, run `rustup override unset`, and build as above.

## Layout

```
crates/freeshift-core   protocol parsing, BLE engine (btleplug), OpenBikeControl server, keyboard output
crates/freeshift-cli    freeshift-cli.exe test and diagnostics tool
app/src-tauri           Tauri 2 desktop app (tray, autostart, settings commands)
app/ui                  plain HTML/CSS/JS front end, no build step
```

## Credits

- Zwift Click v2 protocol details: [ClickTamer](https://github.com/Tom-Webstar/ClickTamer) and [zwiftboard](https://github.com/AntoninGuilbaud/zwiftboard) (both MIT). No code copied; FreeShift is a fresh Rust implementation.
- Zwift Click v1 message layout: BikeControl v4.6.2, the last GPL-3.0 release (facts only, no code).
- SRAM AXS pairing and encryption: the protocol description in [sram-axs](https://github.com/GaborWnuk/sram-axs) (MPL-2.0). No code copied; its worked example is used as test data. Reading paddle presses was worked out from recorded Bluetooth traffic.
- [OpenBikeControl protocol](https://github.com/OpenBikeControl/openbikecontrol-protocol) (MIT).
