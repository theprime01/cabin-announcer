# Cabin Announcer

Real cabin announcements for Microsoft Flight Simulator 2024, played automatically at the right moment of every flight, from boarding to disembarking.

The app watches your flight through SimConnect and works out which phase you're in. It then plays the matching recording from a **sound pack**. It works with any airliner, and it understands:

- **PMDG 737 and 777**: the real seat belt sign selector, the passenger doors (on the 777, whether they're armed) and the FMC (cruise altitude and top of descent).
- **GSX Pro**: boarding, pushback and deboarding follow GSX's own progress.
- **Your airline**: detected from your SimBrief plan or your livery, so a Norwegian flight plays the Norwegian pack.

There's no computer voice. Announcements without a recording simply stay silent.

## What plays when

| Announcement | Who | When |
|---|---|---|
| Boarding welcome | Cabin crew | GSX starts boarding, or a door has been open at the gate for 5 s |
| Boarding complete | Cabin crew | GSX finishes boarding, or the doors close |
| Arm doors | Cabin crew | Doors closed, before pushback |
| Departure delay | Flight deck | 10 min past the SimBrief departure time, or 10 min at the gate with the doors closed |
| Captain's welcome | Flight deck | 20 s after the doors close, or at pushback |
| Before the safety briefing, safety briefing | Cabin crew | Pushback (GSX or tug) or start of taxi |
| Cabin lights dimmed for takeoff | Cabin crew | Taxiing at night |
| Cabin crew, secure the cabin for takeoff | Flight deck | 45 s into the taxi, or entering the runway |
| Takeoff delay | Flight deck | Stopped for 4 min while taxiing out |
| Cabin crew, seats for takeoff | Flight deck | Strobes or landing lights switched on while taxiing (or at 40 kt) |
| After takeoff, seat belt sign off | Cabin crew | Belt sign switched off in the climb |
| Captain's cruise update | Flight deck | 60 s after levelling off at cruise |
| Seat belts on (turbulence) | Cabin crew | Belt sign switched back on in cruise (at most every 10 min) |
| Holding | Flight deck | A full circle flown at a steady altitude (at most every 30 min) |
| Captain's descent announcement | Flight deck | 10 nm before the FMC top of descent, or once descending |
| Descent, seat belts on | Cabin crew | Descending below about 10,000 ft, or belt sign on in the descent |
| Cabin lights dimmed for landing | Cabin crew | Descending at night |
| Cabin crew, secure the cabin for landing | Flight deck | Below 5,000 ft on the approach |
| Cabin crew, seats for landing | Flight deck | Gear down or below 2,500 ft |
| Go-around | Flight deck | Climbing away after getting below 1,500 ft on approach |
| Welcome after landing | Cabin crew | Slowed below 30 kt after touchdown |
| Disarm doors | Cabin crew | Parked, parking brake set, engines off |
| Disembarking | Cabin crew | Belt sign off at the gate, or GSX deboarding starts |

Boarding music plays at the gate (and while deboarding) and fades out for pushback. Optional cabin ambience plays from doors closed to the gate. Both dip under announcements.

If you connect mid-flight, the app works out the current phase and skips the announcements you've already missed. Ten minutes after disembarking it starts over for the next flight; **New flight** does the same straight away.

## Sound packs

A pack is a folder of recordings, one subfolder per announcement. Packs live in `%APPDATA%\CabinAnnouncer\packs`.

- **Fenix A320 owners:** the **Sound packs** tab finds Fenix-format announcement sets in your Community folder (`<package>\Announcements\<airline>\`) and imports each one with a click. Each airline folder (BAW, EZY, …) becomes a pack for that airline, and `Default` plays for the rest. Universal Announcer packs use the same format.
- **Any other pack:** install it from a zip, a folder or an `https://` link, or drag it onto the window. Files are sorted into announcements by name: Fenix names like `CrewSeatsTakeoff.ogg`, or words like "safety", "boarding" and "turbulence". Files that can't be matched go into `_unsorted` for you to move.

Each pack can be tagged with airline codes, and sister airlines count, so `NAX` covers all of Norwegian. Three per-pack options are available:
- **Play every file in order**, for an announcement recorded in two languages.
- **Add a PA chime**, for recordings that don't start with one.
- **Fill gaps** (on by default): when an airline's pack lacks an announcement, the recording comes from a pack without an airline, such as Fenix Default.

The app can also pick recordings by time of day and choose between alternative versions:
- Fenix-style tags pick by time of day, so `BoardingWelcome[Morning].ogg` plays only in the morning (sim local time). A `morning`, `afternoon`, `evening` or `night` folder does the same.
- Numbered versions such as `ArmDoors[2].ogg` are picked at random.

On the **Announcements** tab you can preview any pack, including every recorded version of each announcement.

Only install recordings you have the right to use. Real airline announcements are copyrighted, so none are included here.

## Sound

- **Cockpit camera:** quieter and muffled, as if heard through the flight deck door. Outside cameras have their own volume.
- **Cabin PA effect:** makes clean studio recordings sound like the cabin speakers.
- **Hotkeys** (they work while the sim has focus, and in VR): `Ctrl+Shift+F9` replays the last announcement, `F10` stops, `F11` pauses or resumes all sound.

## Setup

1. Download the latest release and put `SimConnect.dll` next to `cabin-announcer.exe`. It comes with the MSFS 2024 SDK (`SimConnect SDK\lib\SimConnect.dll`).
2. **PMDG only:** the aircraft must broadcast its SDK data. The app offers to switch this on when it sees a PMDG aircraft without data. You can also add it yourself to `737_Options.ini` or `777_Options.ini` in `%APPDATA%\Microsoft Flight Simulator 2024\WASM\MSFS2024\pmdg-aircraft-*\work\`:
   ```ini
   [SDK]
   EnableDataBroadcast=1
   ```
3. Optional: tick **Start Cabin Announcer with MSFS 2024** on the Flight tab. This adds the app to the sim's `exe.xml` (a backup of the original is kept), and the app then closes when the sim does.
4. Optional: enter your SimBrief username to set the airline and the scheduled departure time.

Settings live in `%APPDATA%\CabinAnnouncer\config.toml`.

## Building

```powershell
cargo test
.\tools\package.ps1 -Dest "C:\Games\Cabin Announcer"
```

`package.ps1` builds a release, then copies the exe and `SimConnect.dll` into the destination folder. It looks for the DLL at the repo root (git ignores it), then in the MSFS 2024 SDK, or you can pass `-SimConnect <path>`.

`tools\pmdg-offsets.ps1` uses the Visual Studio C++ compiler to print the PMDG SDK struct offsets used in `src\pmdg.rs`. Re-run it after a PMDG update changes the SDK headers.

## License

MIT, see [LICENSE](LICENSE). This covers the code only. Sound packs, the PMDG SDK and SimConnect belong to their owners.
