# games/ — put your own game files here

This folder is ignored by git. Nothing in it is ever uploaded.

```text
games/
  iw4/            Your Modern Warfare 2 (2009) MULTIPLAYER install — a link or a copy of
                  the Steam folder that contains iw4mp.exe, main/ and zone/.
  doom/           One or more Doom IWADs you own: doom2.wad, doom.wad, plutonia.wad,
                  tnt.wad, doom1.wad, or Freedoom (freedoom1.wad / freedoom2.wad).
  soundfont.sf2   Optional: a General MIDI soundfont for music.
```

A link saves copying the whole MW2 install.

Linux (symlink):

    ln -s "/path/to/Steam/steamapps/common/Call of Duty Modern Warfare 2" games/iw4

Windows (directory junction, no admin rights needed; in a Command Prompt in the repository):

    mklink /J games\iw4 "C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Modern Warfare 2"

A Windows symbolic link (`mklink /D`) only works with Developer Mode or as administrator; use the
junction.

Then run ./iw4doom.sh (Linux) or iw4doom.bat (Windows) from the repository root.
