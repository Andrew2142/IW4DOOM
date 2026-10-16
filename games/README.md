# games/ — put your own game files here

This folder is ignored by git. Nothing in it is ever uploaded.

```text
games/
  iw4/            Your Modern Warfare 2 (2009) MULTIPLAYER install — a copy or a symlink
                  to the Steam folder that contains iw4mp.exe, main/ and zone/.
  doom/           One or more Doom IWADs you own: doom2.wad, doom.wad, plutonia.wad,
                  tnt.wad, doom1.wad, or Freedoom (freedoom1.wad / freedoom2.wad).
  soundfont.sf2   Optional: a General MIDI soundfont for music.
```

A symlink saves copying the whole MW2 install:

    ln -s "/path/to/Steam/steamapps/common/Call of Duty Modern Warfare 2" games/iw4

Then run ./setup.sh from the repository root.
