# FreeShift

Free Windows app that turns Zwift Click buttons (and other Bluetooth LE buttons) into virtual gear shifts in MyWhoosh. No account, no subscription, no telemetry. MIT licence.

Not affiliated with Zwift, SRAM or MyWhoosh.

## Install

Run `installer\FreeShift-Setup.exe`. It installs for your user only (no admin), adds Start-menu and desktop shortcuts and an uninstaller (Settings → Apps). Closing the window keeps FreeShift in the tray; use the tray icon's **Quit** to stop it. **Start with Windows** is a switch in the app.

## How it talks to MyWhoosh

| Output | How | When to use |
|---|---|---|
| **Network** (default) | FreeShift advertises itself as an OpenBikeControl controller on your LAN (mDNS `_openbikecontrol._tcp`, TCP port 21587) and sends Shift Up / Shift Down messages | MyWhoosh on this PC or any device on the same network. No window focus needed |
| **Keyboard keys** | Brings the window whose title starts with `MyWhoosh` to the front and presses I / K with scan codes | If MyWhoosh doesn't list FreeShift as a controller |

If Windows asks whether FreeShift may use the network, allow **private** networks.

## Supported controllers

| Device | Status |
|---|---|
| Zwift Click (v1) | `RideOn` handshake, Plus / Minus |
| Zwift Click v2, right pod | Plus, A, B, Y, Z; keep-alive every 3 s, idle pod reset at 55 s |
| Zwift Click v2, left pod | Works only while Zwift's own app has unlocked it in the last ~24 h |
| Zwift Ride | Same keypad message as Click v2, untested |
| Zwift Play | Not supported (encrypted) |
| SRAM AXS | Not supported yet, see below |
| Any other BLE button | "Custom trigger": any notification on one characteristic = tap, two quick ones = double tap |

### SRAM AXS

Current SRAM AXS indoor shifting works by bonding an app to the rear derailleur (hold its AXS button until the light flashes) and switching the derailleur into a mode where shifter presses are reported to the app instead of moving the derailleur. That bonding uses an encrypted key exchange whose UUIDs, key parameters and message format are not public, so FreeShift cannot do it today. Adding it needs a capture of the Bluetooth traffic during that setup on your own drivetrain. The custom trigger above only helps if a shifter exposes a plain notification on a press without bonding, which `freeshift-cli explore SRAM` shows.

Default mapping: Plus → Shift up, Minus → Shift down, B → Shift down (so a right-pod-only Click v2 has both directions), Tap → up, Double tap → down. Change any button in **Buttons**; **Swap up / down** flips every mapping.

## Hardware test checklist

Do these in order and report the results (copy the lines from **Activity** or from the command output).

1. **Wake and connect.** Start FreeShift, press a Click button. Report: which model **Controllers** shows (Zwift Click / Click v2 right / left) and the battery %. Do *not* pair the Click in Windows Bluetooth settings.
2. **Buttons.** Press each Click button once. Report: the name that lights up in **Buttons** for each, and whether any press shows nothing.
3. **MyWhoosh (network).** Open MyWhoosh's controller pairing screen. Report: does "FreeShift" appear, and does the pill turn green ("Trainer app connected")? Then in a ride: Plus → gear goes up or down?
4. **Direction.** If gears go the wrong way, press **Swap up / down** and recheck.
5. **Idle test (Click v2).** Leave the Click untouched for 3 minutes, then press. Report: still shifts? Any "Disconnected" lines?
6. **Keyboard fallback** (only if step 3 fails). Output → Keyboard keys → Save. Report: does I shift up or down?
7. **SRAM AXS (read-only).** Wake the shifter, then run `target\release\freeshift-cli.exe explore SRAM 60` from this folder. Press each paddle and blip during the 60 s. Send the whole output. Nothing is written to the derailleur.

## Command-line tool

`target\release\freeshift-cli.exe` (build it with `cargo build -p freeshift-cli --release`):

| Command | What it does |
|---|---|
| `freeshift-cli run` | Same engine as the app, printing every event |
| `freeshift-cli scan 10` | List nearby BLE devices with manufacturer / service data |
| `freeshift-cli explore <name> 60` | Read-only GATT dump of one device, then print every notification |
| `freeshift-cli obc-test` | Advertise to MyWhoosh and shift by typing `u` / `d` (tests MyWhoosh without a Click) |
| `freeshift-cli obc-probe 20` | Act like a trainer app: find a controller over mDNS, connect, print what it sends |
| `freeshift-cli key-test I 5` | Press I in MyWhoosh after 5 s |
| `freeshift-cli swap` / `output obc\|keys` / `keys I K` / `trigger <name> <uuid>` | Edit settings |

Settings live in `%APPDATA%\FreeShift\config.json`; the app and the CLI share them.

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
- [OpenBikeControl protocol](https://github.com/OpenBikeControl/openbikecontrol-protocol) (MIT).
