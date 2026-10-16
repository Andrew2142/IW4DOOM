# games/ — put your own game files here

This folder is ignored by git. Nothing in it is ever uploaded.

Drop in your Modern Warfare 2 (2009) multiplayer folder (the one with `main/` and `zone/`, under
any name) and one or more Doom IWADs. Either of these layouts works:

```text
games/                                  games/
  Call of Duty Modern Warfare 2/          iw4/            (MW2 folder, link or copy)
  doom2.wad                               doom/doom2.wad
  soundfont.sf2   (optional, music)       soundfont.sf2
```

IWADs: doom2.wad, doom.wad, plutonia.wad, tnt.wad, doom1.wad (shareware) or Freedoom
(freedoom1.wad / freedoom2.wad).

A link saves copying the whole MW2 install.

Linux (symlink):

    ln -s "/path/to/Steam/steamapps/common/Call of Duty Modern Warfare 2" games/iw4

Windows (directory junction, no admin rights needed; in a Command Prompt in the repository):

    mklink /J games\iw4 "C:\Program Files (x86)\Steam\steamapps\common\Call of Duty Modern Warfare 2"

A Windows symbolic link (`mklink /D`) only works with Developer Mode or as administrator; use the
junction.

Then run ./iw4doom.sh (Linux) or iw4doom.bat (Windows) from the repository root.
