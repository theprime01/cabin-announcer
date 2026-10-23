# Cabin Announcer

Automatic cabin announcements for Microsoft Flight Simulator 2024. The app watches your flight through SimConnect, works out which phase you're in, and plays the right captain or cabin-crew announcement at the right moment, from boarding through to disembarking.

It works with any airliner. With the **PMDG 737** and **PMDG 777** it also reads the real seat belt sign selector, the passenger doors (on the 777, whether they're armed) and the FMC (cruise altitude, distance to top of descent, distance to destination and flight number).

## What plays when

| Announcement | Who | When |
|---|---|---|
| Boarding welcome | Cabin crew | At the gate, a passenger door has been open for 5 s |
| Arm doors | Cabin crew | Doors closed, or engines start or pushback begins |
| Captain's welcome | Captain | 20 s after the doors close, or at pushback |
| Safety demonstration | Cabin crew | Pushback or start of taxi |
| Cabin crew, seats for takeoff | Captain | Strobes or landing lights switched on while taxiing (or at 40 kt) |
| Seat belt sign off | Cabin crew | Belt sign switched off in the climb (aircraft without a belt sign: 2 min above 10,000 ft) |
| Captain's cruise update | Captain | 60 s after levelling off at cruise |
| Turbulence | Cabin crew | Belt sign switched back on in cruise (at most every 10 min) |
| Captain's descent announcement | Captain | 10 nm before the FMC's top of descent, or once descending |
| Prepare the cabin for landing | Cabin crew | Descending below about 10,000 ft, or belt sign on in the descent |
| Cabin crew, seats for landing | Captain | Gear down or below 2,500 ft AGL |
| Go-around | Captain | Climbing away after getting below 1,500 ft AGL on approach |
| Welcome after landing | Cabin crew | Slowed below 30 kt after touchdown (includes local time and temperature) |
| Disarm doors | Cabin crew | Parked, parking brake set, engines off |
| Disembarking | Cabin crew | Belt sign off at the gate |

If you connect mid-flight, the app works out the current phase and skips the announcements you've already missed. Ten minutes after disembarking it starts over for the next flight; the **New flight** button does the same straight away.

## Setup

1. Put `SimConnect.dll` next to `cabin-announcer.exe`. It comes with the MSFS 2024 SDK (`SimConnect SDK\lib\SimConnect.dll`).
2. **PMDG only:** the aircraft must broadcast its SDK data. Add this to `737_Options.ini` / `777_Options.ini` in `%APPDATA%\Microsoft Flight Simulator 2024\WASM\MSFS2024\pmdg-aircraft-*\work\`:
   ```ini
   [SDK]
   EnableDataBroadcast=1
   ```
   The app offers to do this for you when it sees a PMDG aircraft without data. Reload the aircraft afterwards.
3. Optional: enter your SimBrief username and press **Load latest flight plan**. This fills in the airline, flight number, cities, flight time and cruise altitude.

## Airline detection

The app works out the airline from your SimBrief plan first. Without one, it uses what the livery reports to the sim: its name, its ATC airline/callsign and the aircraft title. Sister companies count as the same airline, so Norwegian matches NAX, NSZ, NAN, IBK, NOZ and DAN, as well as "Norwegian", "NORSHUTTLE" and "REDNOSE". The detected airline's name is used in announcements unless you type one, and it picks the matching sound pack.

## Sound packs

A sound pack is a folder of recorded announcements. Install one on the **Sound packs** tab from a zip, a folder or an `https://` link, or drag it onto the window. Each file is sorted into an announcement by its name ("safety", "boarding", "turbulence"…), and anything that can't be matched goes into the pack's `_unsorted` folder for you to move.

Give a pack one or more airline codes and it plays automatically when you fly for that airline. A pack with no codes plays for every airline that doesn't have its own. **Play every file in order** suits packs with the same announcement in two languages. Announcements a pack doesn't cover fall back to the computer voice.

On the **Announcements** tab, **Recordings from** lets you preview any installed pack. Each announcement has **▶ Recording** and **▶ Voice** buttons, and when a pack has several versions of an announcement, each one is listed with its own play button.

Packs live in `%APPDATA%\CabinAnnouncer\packs\<pack>\<announcement>\`, and settings are in `pack.toml`. Only install recordings you have the right to use. Real airline announcements are copyrighted.

## Voices and your own recordings

Speech uses the voices installed in Windows (Settings > Time & language > Speech). You can pick one voice for the captain and one for the cabin crew. A PA filter and a synthesised chime make the voices sound like the cabin speakers.

To use real recordings, press **Open my recordings folder** on the Announcements tab and drop `.wav`, `.mp3`, `.ogg` or `.flac` files into the folder named after an announcement (for example `sounds\safety_demo`). If a folder has several files, the app picks one at random each time.

You can edit every script. Placeholders like `{destination}` are filled in from the flight, and a `[bracketed part]` is left out when its details are unknown.

Settings live in `%APPDATA%\CabinAnnouncer\config.toml`.

## Building

```powershell
cargo test
.\tools\package.ps1 -Dest "C:\Games\Cabin Announcer"
```

`package.ps1` builds a release, then copies the exe and `SimConnect.dll` into the destination folder. It looks for the DLL at the repo root (git ignores it), then in the MSFS 2024 SDK, or you can pass `-SimConnect <path>`.

`tools\pmdg-offsets.ps1` uses the Visual Studio C++ compiler to print the PMDG SDK struct offsets used in `src\pmdg.rs`. Re-run it after a PMDG update changes the SDK headers.
