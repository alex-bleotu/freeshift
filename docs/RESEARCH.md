# FreeShift: research for building your own shifter app

Goal: a free Windows app that turns Zwift Click and SRAM AXS button presses into virtual gear shifts in MyWhoosh.

Status of each piece, as of 2026-11-17:

| Piece | Confidence | Notes |
|---|---|---|
| Zwift Click v1 input | High | Plain BLE, no encryption, protocol fully known |
| Zwift Click v2 input (right pod) | High | Same family, different message type, needs keep-alive |
| Zwift Click v2 input (left pod) | Low | Needs a daily unlock signed by Zwift's servers |
| Output to MyWhoosh over the network (OpenBikeControl) | Medium-high | Open MIT spec, MyWhoosh lists it as implemented |
| Output to MyWhoosh as I/K key presses | Medium | Works, but up/down direction is disputed and MyWhoosh must have focus |
| SRAM AXS individual buttons | Low | Proprietary bonding + crypto, nothing public |
| SRAM AXS "any button pressed" | Medium-low | An earlier BikeControl release did this; UUIDs are not public, you have to find them |

Before anything else: ask yourself which Click you own. v1 is a single grey puck that pairs alone. v2 comes as a left and right pair with more buttons.

---

## 1. Recommended architecture

```
 Zwift Click ─┐                                   ┌─► MyWhoosh (OpenBikeControl, TCP over LAN)   ← preferred
              ├─► BLE input layer ─► button events ─► mapper ─┤
 SRAM AXS  ───┘     (btleplug)        (press/release)         └─► SendInput I/K into MyWhoosh window ← fallback
```

- **Input:** one module per device type that turns BLE notifications into `ButtonEvent { device, button, pressed }`.
- **Mapper:** a user-editable table from button to action (`ShiftUp`, `ShiftDown`, `None`). Swapping up/down is just editing the table.
- **Output:** two pluggable sinks. Prefer OpenBikeControl: MyWhoosh doesn't need focus, and the protocol has explicit Shift Up/Shift Down messages, so there is no I/K direction ambiguity.

### Stack (Rust + Tauri 2)

| Need | Crate / tool | Notes |
|---|---|---|
| BLE | `btleplug` | Uses WinRT on Windows. Scan, connect, `discover_services`, `subscribe`, then read `notifications()` stream |
| Async runtime | `tokio` | btleplug is async |
| mDNS advertising | `mdns-sd` | Registers `_openbikecontrol._tcp.local.` |
| TCP server | `tokio::net::TcpListener` | MyWhoosh connects to you |
| Key injection | `windows` crate (`Win32::UI::Input::KeyboardAndMouse::SendInput`) | Use scan codes, see section 5 |
| UI + tray | Tauri 2 | Uses WebView2 (built into Windows 11) |
| Start with Windows | `tauri-plugin-autostart` | |
| Installer | Tauri bundler (`cargo tauri build`) | Produces NSIS `.exe` and/or MSI with Start-menu shortcut and uninstaller; no Inno Setup needed |
| Settings | `serde` + JSON in `%APPDATA%\FreeShift` | |

Build the BLE + output core as a plain Rust CLI first. Add the Tauri UI only once a Click press reliably shifts MyWhoosh.

### Toolchain (one-time)

1. Visual Studio C++ build tools (needs admin, about 7 GB), in an elevated terminal:
   ```
   winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   ```
2. Rust: `winget install Rustlang.Rustup`, then `rustup default stable-msvc`.
3. Tauri CLI: `cargo install tauri-cli --version "^2"`.
4. Node.js (already installed) if you want a JS front end; plain HTML also works.
5. A BLE explorer for reverse engineering: **nRF Connect** on an Android phone, or Microsoft's **Bluetooth LE Explorer** from the Store.

---

## 2. Zwift Click protocol

Sources: BikeControl v4.6.2 (last GPL-3.0 release, tag `gpl3`, https://github.com/OpenBikeControl/bikecontrol/tree/gpl3), ClickTamer (MIT), zwiftboard (MIT), qdomyos-zwift PR #4743, ajchellew/zwiftplay.

### 2.1 Discovery

- Manufacturer data company ID **`0x094A`** (Zwift, Inc., decimal 2378). Name usually starts with `Zwift`.
- The first byte of manufacturer data is the device type, per BikeControl v4.6.2 constants:

| Byte | Device |
|---|---|
| `0x09` | Zwift Click (v1), "BC1" |
| `0x0A` | Click v2 right (marked "unconfirmed" upstream) |
| `0x0B` | Click v2 left (marked "unconfirmed" upstream) |
| `0x02` / `0x03` | Zwift Play right / left |
| `0x07` / `0x08` | Zwift Ride right / left |

- No Windows pairing is needed. Do **not** pair the Click in Windows Settings; just connect from your app.

### 2.2 GATT

| Role | UUID | Properties |
|---|---|---|
| Custom service | `00000001-19ca-4651-86e5-fa29dcdd09d1` | |
| Custom service (newer firmware, Ride/Click v2 may use it) | `0000fc82-0000-1000-8000-00805f9b34fb` | try this first, then the one above |
| Async (button events) | `00000002-19ca-4651-86e5-fa29dcdd09d1` | Notify |
| SyncRX (you write here) | `00000003-19ca-4651-86e5-fa29dcdd09d1` | Write / Write Without Response |
| SyncTX (device replies) | `00000004-19ca-4651-86e5-fa29dcdd09d1` | Indicate |

### 2.3 Click v1 sequence

1. Subscribe to Async (notify) and SyncTX (indicate).
2. Write `52 69 64 65 4F 6E` (ASCII `RideOn`) to SyncRX, write-without-response.
3. The device answers on SyncTX with `52 69 64 65 4F 6E 01 03 …` (`RideOn` + `01 03` + public key bytes, which you can ignore). Click v1 does **not** need encryption; Zwift Play does.
4. Notifications on Async: byte 0 is the message type, the rest is a protobuf message.

| Type byte | Meaning | Payload |
|---|---|---|
| `0x37` (55) | Click button state | `ClickKeyPadStatus` |
| `0x15` (21) | Idle / empty | ignore |
| `0x19` (25) | Battery | protobuf, battery % is payload byte index 1 (after the `08` tag) |
| `0xFE` | Disconnect (another app took over) | |

`ClickKeyPadStatus`: field 1 = `Button_Plus`, field 2 = `Button_Minus`, both enum `PlayButtonStatus` where **`ON = 0` (pressed), `OFF = 1` (released)**. Both are varints, so tags are `0x08` and `0x10`:

```
37 08 00 10 01   plus pressed,  minus released
37 08 01 10 00   plus released, minus pressed
37 08 01 10 01   nothing pressed
```

Byte strings above are derived from the protobuf encoding rules; confirm against a real capture. Proto3 omits fields that equal the default `0`, so a field missing from the message means **pressed**. Parse it as real protobuf (`prost` or a 10-line varint reader), not as fixed offsets.

Fire a shift on the **released → pressed** edge, and remember the previous state per button.

### 2.4 Click v2 differences

From ClickTamer / zwiftboard (both MIT, Go, Windows) and qdomyos PR #4743:

- Same service and characteristics.
- Activation after connecting: `RideOn 02 03`, then `00 08 00`, then `00 08 10` (ClickTamer calls this "the activation trio"). Then **re-send `RideOn` every 3 s** to both pods. That keep-alive matters: the right pod deep-sleeps after about 56 s idle and the link drops with HCI error `0x08`.
- After about 55 s without any button press, ClickTamer/zwiftboard send a pod reset `0x18` (the README doesn't say which characteristic; read their source).
- Button frames start with type **`0x23`**. Protobuf field 1 is a bitmap where a **0 bit means pressed**:

| Bit | Button |
|---|---|
| `0x0001` | Left |
| `0x0002` | Up |
| `0x0004` | Right |
| `0x0008` | Down |
| `0x0010` | A |
| `0x0020` | B |
| `0x0040` | Y |
| `0x0080` | Z |
| `0x0100` | Minus (shift) |
| `0x1000` | Plus (shift) |

- **Lock state:** after the handshake an unlocked pod echoes `0x52` (`R`) on SyncTX. A locked pod sends a challenge starting `0xFF` (`FF 03 …`).
- **Right pod is never locked.** The **left pod** needs an unlock signed by Zwift's servers roughly every 24 h. It can't be reproduced offline, and spoofing it isn't worth it. Practical approach, as zwiftboard does: connect both pods (the left one is the BLE anchor the right one relays through), but **map buttons only from the right pod**. Reports conflict on whether newer firmware still requires the unlock, so test yours.
- ClickTamer says right-pod presses show up on both pods' streams; zwiftboard says both pods send the same frame. Either way, de-duplicate by button + edge.

---

## 3. SRAM AXS: what is known and how to find the rest

### Known

- AXS components advertise only while awake, about 20–24 s for coin-cell shifters/blips after a press, and about 5 min for derailleur/dropper.
- Advertisements: service data UUID **`0xFE51`** (SRAM), manufacturer company ID **`0x0933`**, local name `SRAM <serial>`. These carry battery, firmware and serial, **not button presses** (esphome-sram-axs teardown).
- Shifting itself uses SRAM's own encrypted AIREA radio, not BLE. BLE is for configuration and firmware.
- BikeControl ≤ v4.6.2 (GPL): connected to the device, subscribed to one "trigger" characteristic, and could only tell **that some button was pressed**, not which one. It exposed "single tap = shift up, double tap = shift down" with a configurable 150–600 ms window. Its UUIDs lived in the private module and aren't public.
- BikeControl ≥ 6.3: connects to the **rear derailleur**, does a Diffie-Hellman bond (you hold the derailleur's AXS button until it flashes), backs up the button config, then reconfigures the derailleur to **report** presses instead of shifting. The public stubs name the pieces (`bondService`, `bondChar`, `tokenChar`, `controlTriggerChar`, `gearChar`, `componentEventChar`, DH `g`/`N`, protobuf fields), but every value in them is a fake placeholder.

### Don'ts

- Don't write to any SRAM characteristic you don't understand. You can break your real shifting configuration. Restoring it means the SRAM AXS app or a dealer.
- Reverse engineering your own device for interoperability is allowed in the EU (Software Directive 2009/24/EC, art. 6), but don't copy code from the current non-commercial BikeControl or its stubs.

### Discovery plan (read-only)

1. Wake the shifter. In nRF Connect (phone) or Bluetooth LE Explorer (PC), scan for `SRAM …`. Note which component is which (byte 8 of the `0xFE51` service data: `0x00`/`0x01` = controllers, `0x81` = rear derailleur, `0x84` = dropper).
2. Connect to a **shifter** first, then the **derailleur**. Record every service and characteristic UUID and its properties.
3. Enable notify/indicate on every characteristic that supports it. Press each paddle, then each blip. Write down which characteristic fires and the exact bytes.
4. If something fires on any press with identical bytes, that's the "trigger" characteristic. Implement tap/double-tap on it the same way BikeControl v4.6.2 did.
5. If nothing fires without bonding, individual buttons are behind the DH bond. Stop there, document what you saw, and ship Click-only.
6. Note: some reports say AXS components stop advertising once SRAM's own app connects. Close the SRAM AXS app on your phone while testing.

Also expect the shifter to still shift the real derailleur when you press it, unless you change that button's function in the SRAM AXS app. The blips are the cleanest buttons to repurpose.

---

## 4. Output A (preferred): OpenBikeControl to MyWhoosh

Spec: https://github.com/OpenBikeControl/openbikecontrol-protocol (MIT), mirrored at https://openbikecontrol.org/protocol. MyWhoosh is listed as implemented over the network; community projects say since MyWhoosh 5.7.0, using mDNS + TCP.

Your app plays the **device** role:

1. Start a TCP listener on any port (one community project reports `21587`; any port works because MyWhoosh reads it from mDNS).
2. Register mDNS service `FreeShift._openbikecontrol._tcp.local.` on that port with TXT records:
   `version=1`, `id=<stable unique id>`, `name=FreeShift`, `manufacturer=FreeShift`, `model=FreeShift`, `service-uuids=d273f680-d548-419d-b9d1-fa0472345229`.
3. In MyWhoosh, pick FreeShift as a controller; MyWhoosh connects as a TCP client.
4. Send button state messages only on change:

| Bytes | Meaning |
|---|---|
| `01 01 01` | Shift Up pressed |
| `01 01 00` | Shift Up released |
| `01 02 01` | Shift Down pressed |
| `01 02 00` | Shift Down released |

5. Every 30–60 s send device status `02 <battery 0-100 or FF> 01`.
6. MyWhoosh may send app info (`04 …`, lists the buttons it supports) and haptics (`03 …`). Read and ignore them, but do read them so the socket buffer doesn't fill.

There's no length prefix on the TCP stream, so always write a whole message in a single `write_all`.

Allow FreeShift through Windows Firewall on **private** networks, and keep the PC and MyWhoosh on the same subnet.

## 5. Output B (fallback): keyboard I/K into MyWhoosh

- MyWhoosh's help page lists **I and K** for shifting but not which is up. The two reference projects disagree: ClickTamer maps `Plus → i` (commented "gear down"), and zwiftboard maps `Plus → k` (commented "gear down"). Make the mapping swappable and test on the bike.
- Use `SendInput` with `KEYEVENTF_SCANCODE` (I = scan code `0x17`, K = `0x25`). MyWhoosh is a game engine app, and those often ignore virtual-key-only input.
- Send key-down, wait about 30–50 ms, then key-up. Too short a press can be missed by games that poll keys.
- Input only reaches the **focused** window. ClickTamer brings the window whose title starts with `MyWhoosh` to the front before each key; you can do the same with `FindWindowW` + `SetForegroundWindow`.
- UIPI: if MyWhoosh runs as admin, your app has to as well, or the keys are silently dropped.

---

## 6. Milestones and how to test each

1. **Scan:** CLI lists BLE devices with company ID `0x094A` and prints the type byte. Test: Click shows up.
2. **Click events:** connect, handshake, print `+ down`, `+ up`, `- down`, `- up`. Test: each press prints exactly one down and one up.
3. **Keep-alive (v2 only):** leave it untouched for 3 minutes, then press. Test: still connected, press is seen.
4. **OpenBikeControl:** advertise + TCP server, map Click `+` to Shift Up and `-` to Shift Down. Test in MyWhoosh: gear number goes up on `+` and down on `-`.
5. **Fallback keys:** same mapping, I/K output. Test which key shifts which way and record it.
6. **SRAM discovery:** section 3 plan, read-only.
7. **Tauri UI:** device list with connect state and battery, mapping editor with a "swap up/down" button, output mode switch, tray icon, autostart toggle.
8. **Installer:** `cargo tauri build` produces `src-tauri\target\release\bundle\nsis\FreeShift_<ver>_x64-setup.exe`. Install, launch, test, uninstall.

### Hardware test checklist

- [ ] Which Click model (v1 / v2), firmware version (Zwift Companion app shows it)
- [ ] Each Click button: does exactly one press event arrive? Paste the raw notification hex
- [ ] MyWhoosh version, and whether FreeShift appears as an OpenBikeControl controller
- [ ] `+` press: MyWhoosh gear before → after
- [ ] `-` press: MyWhoosh gear before → after
- [ ] I key alone: gear goes up or down?  K key alone: up or down?
- [ ] v2: still works after 3 min idle? left pod events received or locked (`FF` on SyncTX)?
- [ ] SRAM: list of services/characteristics per component, and which (if any) notify on a press

---

## 7. Reference projects and what you may reuse

| Project | Licence | Use it for |
|---|---|---|
| [ClickTamer](https://github.com/Tom-Webstar/ClickTamer) | MIT | Click v2 on Windows in Go: handshake, keep-alive, reset, SendInput. Code reusable with attribution |
| [zwiftboard](https://github.com/AntoninGuilbaud/zwiftboard) | MIT | Same, simpler, right-pod-only approach |
| BikeControl ≤ v4.6.2 (tag `gpl3`) | GPL-3.0 | Click v1 parsing, SRAM tap/double-tap idea. Copying code makes FreeShift GPL-3.0, which is fine if you publish source |
| BikeControl current | Non-commercial, not open source | Read only. Don't copy code |
| [openbikecontrol-protocol](https://github.com/OpenBikeControl/openbikecontrol-protocol) | MIT | Output protocol |
| [esphome-sram-axs](https://github.com/tarekrached/esphome-sram-axs) | see repo | SRAM advertisement layout |
| [qdomyos-zwift PR #4743](https://github.com/cagnulein/qdomyos-zwift/pull/4743) | GPL-3.0 project | Click v2 keep-alive and lock detection notes |
| [ajchellew/zwiftplay](https://github.com/ajchellew/zwiftplay) | not stated | Zwift Play (encrypted) background only |
| [swift-click](https://github.com/benoitzelsmann/swift-click) | closed engine | Shows the Click v2 → OpenBikeControl → MyWhoosh path works; free app you could also just use |

Name and branding: don't use "Zwift", "SRAM", "MyWhoosh" or "BikeControl" in the app name or logo. Saying "works with Zwift Click" in a README is fine.

---

## 8. Open questions that only hardware can answer

1. Your Click model and whether v2 left-pod unlock is still enforced on current firmware.
2. Whether your MyWhoosh build discovers `_openbikecontrol._tcp` (and its version).
3. I vs K direction.
4. Whether SRAM AXS exposes any unbonded notify characteristic on button press.
5. Whether btleplug on Windows needs the device unpaired (expected: yes, keep it unpaired) and how it behaves after sleep/resume.

## Sources

- BikeControl repository, tags `gpl3` (GPL-3.0) and HEAD: https://github.com/OpenBikeControl/bikecontrol
- ClickTamer: https://github.com/Tom-Webstar/ClickTamer
- zwiftboard: https://github.com/AntoninGuilbaud/zwiftboard
- qdomyos-zwift PR #4743: https://github.com/cagnulein/qdomyos-zwift/pull/4743
- zwiftplay: https://github.com/ajchellew/zwiftplay
- swift-click: https://github.com/benoitzelsmann/swift-click
- OpenBikeControl spec: https://github.com/OpenBikeControl/openbikecontrol-protocol and https://openbikecontrol.org/
- mywhoosh-shifter (OpenBikeControl over Wi-Fi): https://github.com/hannahinthere/mywhoosh-shifter
- SB20 power proxy issue citing MyWhoosh port 21587: https://github.com/cauldnz/SB20-power-proxy/issues/367
- MyWhoosh MyShift (I/K keys): https://mywhoosh.com/myshift-mywhooshs-virtual-shifting-explained/
- esphome-sram-axs: https://github.com/tarekrached/esphome-sram-axs
- BikeControl SRAM AXS page: https://bikecontrol.app/sram-axs/
- DC Rainmaker on AXS BLE vs AIREA: https://www.dcrainmaker.com/2019/02/hands-everything-about.html
- SendInput docs (UIPI, scan codes): https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput
- btleplug: https://github.com/deviceplug/btleplug
