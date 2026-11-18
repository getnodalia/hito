# macOS DMG window

What Finder shows when the DMG opens: a 660 × 400 pt background with the app icon and the
`Applications` link side by side. `package.sh` copies these files into the image; nothing here is
generated at build time.

| File | What |
|---|---|
| `background.svg` | Source of the background, drawn by `tools/dmg-background/draw_background.py` in [nodalia-design](https://github.com/getnodalia/nodalia-design): two Nodalia islands on the window colour, the left one in Hito's green (`#3D9A2B`) with a Gantt chart whose bars fill as their tasks get done, the milestone they lead to and the overall progress; the right one white under the icons. Its text (Sora) is outlined, so rendering it needs no fonts. |
| `background.tiff` | The background at 1x (660 × 400 px, 72 dpi) and 2x (1320 × 800 px, 144 dpi) in one HiDPI TIFF (Deflate, sRGB). Goes to `.background/background.tiff`. |
| `dmg-layout.DS_Store` | Finder's view settings for the volume: window size, icon size 128, Hito.app at (326, 205), `Applications` at (574, 205), and the background. Goes to `.DS_Store`. |
| `generate.py` | Writes `background.tiff` and `dmg-layout.DS_Store` from the SVG and the layout above. |

## Rules

- **The volume name has no version** (`Hito`). `.DS_Store` points at the background through an
  alias that includes the volume name, so a versioned name loses the background. The DMG file
  name still carries the version.
- **The bundle is `Hito.app`, without the accent**, so its name is the same in every Unicode
  normal form and `.DS_Store` always finds it. Its `CFBundleName` is Hito.
- **Finder draws the icon labels in black in light and dark mode** when a window has a
  background, so the area under both icons stays light (the white island).

## Regenerate

Regenerate `background.svg` with nodalia-design's `draw_background.py hito background.svg`, then,
on any OS:

```sh
pip install 'pillow>=12' ds_store==1.3.3 mac_alias==2.2.3
python3 packaging/macos/dmg/generate.py   # needs resvg on PATH
```
