# FreeShift hardware test (prompt for Claude on the bike laptop)

Open Claude Code on the laptop that sits next to the bike, then paste everything below the line.

---

You are helping me test **FreeShift** on this laptop: https://github.com/alex-bleotu/freeshift. FreeShift is a free Windows app that turns Zwift Click and SRAM AXS shifter presses into virtual shifting in MyWhoosh. It sends button presses to MyWhoosh over the local network (OpenBikeControl) or as key presses. SRAM AXS support is in 0.2.0 and newer. Collect everything into one results folder that I will take back to my main PC.

## Rules

- Never write anything to the SRAM shifters or derailleur yourself. The only writes allowed are FreeShift's own pairing messages in section 4, after I agree. `scan` and `explore` are read-only.
- `freeshift-cli.exe run` and the FreeShift app (0.2.0 and newer) connect to any awake SRAM rear derailleur and try to pair when they have no key. Until section 4, ask me to leave the bike's drivetrain untouched so it stays asleep.
- Do not pair the Zwift Click in Windows Bluetooth settings. FreeShift connects on its own.
- Ask me before anything I have to do physically (pressing buttons, waking devices, plugging in the phone). Tell me exactly what to press and in which order, then wait for me.
- Do not upload, commit or post any results. Everything stays in the results folder.
- If you run inside the Claude desktop app, installers and apps you start from your shell land in Claude's private copy of AppData. Start the installer and the FreeShift app with `explorer.exe "<full path>"` (or ask me to double-click them) so they run as normal apps. Command-line tools can run from your shell.
- One question at a time, with a sensible default.

## 0. Prepare

1. Create the results folder `%USERPROFILE%\FreeShift-test`.
2. Download the latest release files into it:
   - `https://github.com/alex-bleotu/freeshift/releases/latest/download/FreeShift-Setup.exe`
   - `https://github.com/alex-bleotu/freeshift/releases/latest/download/freeshift-cli.exe`
   - `https://github.com/alex-bleotu/freeshift/releases/latest/download/btsnoop_dump.py`
3. Check that Bluetooth works: run `freeshift-cli.exe scan 6` and confirm it lists devices. If it says no adapter, ask me to turn Bluetooth on.
4. Record the Windows version and the Bluetooth adapter name (`Get-PnpDevice -Class Bluetooth` or Device Manager) for the report.

## 1. Zwift Click test (command line, exact logs)

Make sure the FreeShift app is not running (it would hold the Click and port 21587). Then run `freeshift-cli.exe run` for about 4 minutes, saving the output to `click-test.log`, and walk me through this while it runs:

1. Wake the Click (press any button). Note which model it reports (`Zwift Click`, `Zwift Click v2 (right)`, `(left)`).
2. Press each button once, slowly, in this order: Plus, Minus, A, B, Y, Z, Up, Down, Left, Right. Some only exist on one pod; skip what I don't have.
3. Open MyWhoosh (on this laptop or a phone on the same Wi-Fi). In its controller pairing screen, check whether **FreeShift** appears and connect it. The log then shows `+ connected trainer app` and, if MyWhoosh says which actions it supports, a `supports:` line.
4. In a ride, press Plus three times, then Minus three times. Ask me whether the gear went up or down each time.
5. Leave the Click untouched for 3 minutes, then press Plus again. Did it still shift?

From the log, list every button that printed `down`/`up`, every button that printed nothing, and every `disconnected` line.

## 2. Keyboard fallback (only if MyWhoosh never showed FreeShift)

With MyWhoosh open in a ride on this laptop, run `freeshift-cli.exe key-test I 5`, tell me to switch to MyWhoosh, and ask whether the gear went up or down. Repeat with `K`.

## 3. App check

Start the installer with `explorer.exe` (or ask me to double-click it; on a SmartScreen warning choose "More info" then "Run anyway"). Then start FreeShift from the Start menu. Ask me to:

- press a Click button and confirm it lights up on the controller drawing
- click a button on the drawing, pick another action, save, and confirm it still works
- confirm MyWhoosh connects (the pill at the top turns green)

Write down anything that looks wrong or confusing.

## 4. SRAM AXS

This needs FreeShift 0.2.0 or newer (the app's footer shows the version). With an older one, do only steps 1 and 2.

1. Read-only check: close the FreeShift app, ask me to press a shifter paddle, then immediately run `freeshift-cli.exe scan 10` and save it as `sram-scan.txt`. Each component appears as `SRAM <number>`. Ask me which number is which part if it isn't obvious; shifters advertise for only about 20 seconds after a press.
2. Ask me to wake the drivetrain again, run `freeshift-cli.exe explore "SRAM <derailleur number>" 60` and save it as `sram-explore-derailleur.txt`. When it prints "Listening", tell me to press each paddle and blip three times, slowly. Note the order I used and which characteristic printed a value.
3. Ask me before pairing. Tell me that pairing here replaces the derailleur's key, so FreeShift or BikeControl on another PC has to pair again afterwards. BikeControl and the SRAM AXS app must be closed on every device nearby.
4. Run `freeshift-cli.exe run` for about 4 minutes and save the output to `sram-test.log`. Ask me to wake the drivetrain. If the log says it isn't paired, ask me to hold the AXS button on the rear derailleur until its light flashes, and to keep holding until the log says it is paired.
5. Ask me to press, slowly: left three times, right three times, then any blips. Note which presses printed `SramLeft` or `SramRight`, which printed nothing, and whether the derailleur also moved each time.
6. Leave the drivetrain alone for 5 minutes, start `run` again, and ask me to wake it and press left and right once. It should connect without pairing again.
7. In the app, give **Right double click** an action, save, and ask me to double click and then single click the right paddle. Remove the action again afterwards.

## 5. Android recording of BikeControl's SRAM setup (optional, ask me first)

The main thing FreeShift still lacks for SRAM is how BikeControl stops the paddles from moving the derailleur, and how it restores that. This recording may help find it. BikeControl's setup replaces the derailleur's key, so FreeShift has to pair again afterwards.

Only if I have an Android phone and agree:

1. Guide me through: Developer options (tap Build number 7 times), turn on **USB debugging** and **Bluetooth HCI snoop log** (Enabled or Full), turn Bluetooth off and on, install BikeControl from Google Play, close the SRAM AXS app, turn off other Bluetooth devices.
2. Guide me through BikeControl: connect to the SRAM derailleur, **Set up SRAM control**, hold the derailleur's AXS button until it flashes, then press each paddle and blip three times slowly while I tell you the order and you note the time. Finish with **Restore original shifting** and check that the derailleur shifts normally again. Then turn the snoop log off.
3. Install adb if missing (`winget install Google.PlatformTools`), ask me to plug in the phone and allow USB debugging, check `adb devices`, then run `adb bugreport "%USERPROFILE%\FreeShift-test\bugreport.zip"`.
4. Decode it: `python btsnoop_dump.py bugreport.zip > sram-capture.txt` (and `--all` into `sram-capture-all.txt`). Also extract the raw `btsnoop_hci.log` from the zip into the folder.
5. A bug report contains other personal phone logs. Ask me whether to delete `bugreport.zip` now that the Bluetooth log is extracted.

## 6. Report

Write `%USERPROFILE%\FreeShift-test\REPORT.md` with:

- laptop Windows version, Bluetooth adapter, FreeShift version (from the app footer or the release name)
- Click model and firmware if known, battery, and a table: button, did it register, what it did in MyWhoosh
- MyWhoosh: version, platform (laptop/phone), did FreeShift appear, did it connect, which way Plus and Minus shifted, the `supports:` line if any
- idle test result, keyboard fallback result
- app check notes
- SRAM: components found, derailleur characteristics, which notified on a press (with the hex), and the press order
- SRAM in FreeShift: did pairing work and how long it took, a table (paddle, did it register, did the derailleur move), reconnect without pairing, double click
- Android capture: done or not, the button order and times, the first 200 lines of `sram-capture.txt`
- every problem or error message, quoted exactly

Finish by telling me to copy the whole `FreeShift-test` folder to my main PC (USB stick or OneDrive) and give `REPORT.md` to the Claude session there.
