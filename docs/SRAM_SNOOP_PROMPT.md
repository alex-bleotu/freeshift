# SRAM AXS capture and analysis (prompt for a separate Claude session)

Paste everything below the line into a new Claude Code session on a Windows PC that has Python 3, an Android phone with a USB cable, and the bike with the SRAM AXS drivetrain nearby.

---

You are helping me capture and understand how the **BikeControl** Android app talks to my **SRAM AXS** drivetrain over Bluetooth LE, so that my own open-source app **FreeShift** (https://github.com/alex-bleotu/freeshift) can support SRAM shifters. BikeControl pairs with the rear derailleur (I hold the derailleur's AXS button until its light flashes), then switches the derailleur into a mode where shifter presses are reported to the app instead of moving the derailleur. The goal is a written protocol description, built only from the Bluetooth traffic of my own devices.

## Rules

- Only observe. Never write anything to the SRAM components yourself, and don't build tools that do in this session.
- Do not decompile, unpack or copy code from BikeControl or the SRAM AXS app. Work only from the captured traffic.
- Everything I do with the bike is physical: tell me exactly what to do, one step at a time, and wait for me to confirm.
- At the end of every capture, make sure I tap **Restore original shifting** in BikeControl and that the derailleur shifts normally again on the trainer.
- Keep all files in `%USERPROFILE%\SRAM-capture`. Don't upload or commit anything. An Android bug report contains other personal logs: extract only the Bluetooth log and ask me whether to delete the bug report zip afterwards.
- Ask one question at a time, with a sensible default.

## 1. Prepare

1. Create `%USERPROFILE%\SRAM-capture`.
2. Download the decoder: `https://raw.githubusercontent.com/alex-bleotu/freeshift/main/tools/btsnoop_dump.py`. Read it first. It prints ATT reads, writes, notifications and indications, plus SMP pairing messages, from an Android btsnoop log or a bug report zip; `--all` also prints service and characteristic discovery.
3. Check that `adb` works (`adb version`). If it's missing, install it with `winget install Google.PlatformTools` and open a new terminal.

## 2. Phone setup (guide me)

1. Settings → About phone → tap **Build number** 7 times (Samsung: About phone → Software information; Xiaomi: tap the OS version).
2. Developer options: turn on **USB debugging** and **Bluetooth HCI snoop log**. If it offers a choice, pick **Enabled** or **Full**, never "Filtered", because filtered logs cut the payloads.
3. Turn Bluetooth off and on (that starts a new log).
4. Install **BikeControl** from Google Play; its free trial is enough. Close the SRAM AXS app, and turn off other Bluetooth devices (earbuds, watch) so the log stays small.

## 3. Capture A: first setup

Bike on the trainer, drivetrain charged. Ask me to note the phone's clock time at each step and tell you:

1. Wake the drivetrain by tapping a shifter. In BikeControl, connect to the SRAM derailleur.
2. Tap **Set up SRAM control**, hold the derailleur's AXS button until it flashes, and wait until BikeControl says it's ready.
3. Wait 10 seconds without touching anything (an idle reference).
4. Press, slowly, three times each, with 3 seconds between presses: right paddle, left paddle, right blip, left blip (or whatever buttons I have). I tell you the exact order and times.
5. Press the right paddle and hold it for 3 seconds.
6. Tap **Restore original shifting** in BikeControl. Check the derailleur shifts normally.
7. Turn off the HCI snoop log, plug in the phone, allow USB debugging, check `adb devices`, then run:
   `adb bugreport "%USERPROFILE%\SRAM-capture\capture-a.zip"`

## 4. Capture B: repeat

Turn the snoop log on again, Bluetooth off and on, and repeat the same sequence exactly once more as `capture-b.zip`. Two captures let us tell fixed values (UUIDs, commands, constants) from per-session values (random keys, nonces, counters).

## 5. Decode

For each capture:

1. `python btsnoop_dump.py capture-a.zip --all > capture-a-all.txt` and the same without `--all` into `capture-a.txt`.
2. Extract the raw `btsnoop_hci.log` from the zip into the folder as `capture-a.btsnoop`.
3. Find the derailleur connection: the device address BikeControl connected to (SRAM components advertise company ID `0x0933` and service UUID `0xFE51`, named `SRAM <serial>`). Ignore other devices.

## 6. Analyse

Write `%USERPROFILE%\SRAM-capture\SRAM_PROTOCOL.md` covering:

1. **GATT map** of the derailleur: every service and characteristic UUID with its properties (read, write, write without response, notify, indicate), from the discovery lines.
2. **BLE-level pairing**: are there SMP lines? If yes, which method (look at the pairing request and response bytes: IO capabilities, auth flags, LE Secure Connections bit). If no, the security is all at the application level.
3. **Setup sequence**: every write and notification from connect to "ready", in order, as a table: time, direction, characteristic, length, hex. Mark which messages appear right after I held the AXS button.
4. **Fixed vs changing**: compare capture A and B message by message. For each, say whether the bytes are identical, change completely (looks random: likely keys or nonces), or change in a pattern (counters, timestamps). Note any long values (32, 64, 128, 256 bytes) that could be Diffie-Hellman public keys, and any 16-byte values that could be tokens.
5. **Protobuf**: try to read each payload as protobuf (field number, wire type, value). Report the field layout where it decodes cleanly.
6. **Mode switch**: which write puts the derailleur into "report presses" mode, and which write in the restore step undoes it. Did BikeControl first read a configuration (the backup) and later write it back?
7. **Button events**: line up notifications with my press times. For each physical button give the characteristic and the payload on press, hold and release. Check whether the same button gives identical bytes every time; if the bytes differ for the same press, the events are probably encrypted or carry a counter.
8. **Verdict**: from the traffic alone, could another app reproduce this? List exactly what is known, what changes every session, and what seems to depend on something never sent over the air (a value baked into the app). Be explicit about uncertainty.

Then tell me which files to copy back to my main PC: `SRAM_PROTOCOL.md`, the `capture-*.txt` files and the `capture-*.btsnoop` files, and ask whether to delete the bug report zips.
