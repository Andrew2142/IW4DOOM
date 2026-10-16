# Install guide

The short version is in the [README](../README.md#quick-start): clone, put your files in `games/`, run `./iw4doom.sh` (Linux) or `iw4doom.bat` (Windows). This page has the details.

## What you need

- Linux or Windows 10/11 (64-bit), and about 4 GB of free disk space for the build
- Your own copy of **Call of Duty: Modern Warfare 2 (2009)** with its multiplayer component installed
- A **Doom IWAD** (see [Which Doom version](../README.md#which-doom-version))

## The games folder

Links are fine, so nothing large is copied:

```text
games/
  iw4/            your MW2 multiplayer install (the folder with main/ and zone/)
  doom/           one or more Doom IWADs: doom2.wad, doom.wad, doom1.wad, freedoom1.wad, ...
  soundfont.sf2   optional: a General MIDI soundfont for music
```

## What the first run does

It is the same command every time you play. The first run checks your files and your machine, builds the exporter, turns your MW2 install into a weapon pack in `packs/mw2`, builds the game, runs a short headless smoke test and starts the game. It takes 15-20 minutes, mostly compiling. Later runs see that everything is up to date and start the game within seconds.

If Rust or a build tool is missing, it says exactly what and offers to install it; nothing is installed unless you answer yes. Everything it ran is in `iw4doom.log`.

Options go before or after the game's own arguments:

| Option | What it does |
| --- | --- |
| `--list` | Show the IWADs found in `games/doom/` |
| `--iwad doom1.wad` | Play another IWAD than the preferred one |
| `--reexport` | Export the MW2 pack again |
| `--setup-only` | Do everything except starting the game |
| `--games DIR` | Use another folder than `games/` |
| anything else | Goes to the game, for example `-warp 1 1 -skill 4` |

## Linux

```bash
ln -s "$HOME/.steam/steam/steamapps/common/Call of Duty Modern Warfare 2" games/iw4
mkdir -p games/doom && cp /path/to/DOOM2.WAD games/doom/
./iw4doom.sh
```

Missing system libraries (a C compiler, `pkg-config`, ALSA, Wayland, xkbcommon, udev) are installed with your package manager after you confirm; `sudo` asks for your password. Missing Rust is installed with the official rustup installer into your home folder, also only after you confirm.

## Windows

Steam installs MW2 to `C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Modern Warfare 2` (or the same path under another Steam library). Link it into `games\` with a **directory junction**, which needs no admin rights. In a Command Prompt in the IW4DOOM folder:

```bat
mklink /J games\iw4 "C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Modern Warfare 2"
mkdir games\doom
copy "C:\path\to\DOOM2.WAD" games\doom\
iw4doom.bat
```

(In PowerShell, write `cmd /c mklink /J ...`.) A junction is the best choice. A symbolic link (`mklink /D`) only works with Developer Mode on or as administrator, and copying the folder works but duplicates the whole install. If `games\iw4` is missing, `iw4doom.bat` looks up your Steam libraries and prints the exact `mklink` line for your MW2 folder, and lists the DOOM IWADs it finds in Steam's DOOM folders.

The first run needs two things that Windows does not ship with:

- **Rust**, from [rustup.rs](https://rustup.rs) (`rustup-init.exe`, accept the defaults).
- **Build Tools for Visual Studio** with the **Desktop development with C++** workload, from [visualstudio.microsoft.com/visual-cpp-build-tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/). Rust uses its linker.

When `winget` is available, `iw4doom.bat` offers to install both for you (`Rustlang.Rustup` and `Microsoft.VisualStudio.2022.BuildTools`), after you answer yes; Windows then asks for permission for the Build Tools. Otherwise it prints the links above.

`iw4doom.bat` starts PowerShell with `-ExecutionPolicy Bypass` for that one run, so you do not need to change your execution policy. To run the script directly, use `powershell -ExecutionPolicy Bypass -File iw4doom.ps1`.
