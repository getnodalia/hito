# Attribution

Every asset in this repository, with its author, source and licence. Code is `MIT OR Apache-2.0`
(`LICENSE-MIT`, `LICENSE-APACHE`); third-party Rust crates keep their own licences.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|
| `assets/icons/hito-portrait-1024.png` | App icon artwork: Socrates, cut out in black and white on Hito's teal | Nodalia, from a photograph of the marble bust of Socrates in the Louvre (Roman copy, 1st c., of a Greek original) by Eric Gaba (User:Sting), July 2005 | [Wikimedia Commons: Socrates Louvre.jpg](https://commons.wikimedia.org/wiki/File:Socrates_Louvre.jpg); made with nodalia-design `tools/app-portrait` | Photograph: public domain (released by its author). Artwork: MIT OR Apache-2.0 |
| `assets/icons/hicolor/*/apps/dev.danielmiguelt.Hito.png` | App icon at each size (rounded tile of the artwork above) | Nodalia | made from the artwork above | MIT OR Apache-2.0 |
| `docs/screenshots/*.png` | Screenshots of Hito with demo accounts and messages | Nodalia | taken from the running app | MIT OR Apache-2.0 |
| `packaging/linux/dev.danielmiguelt.Hito.desktop` | Desktop entry | Nodalia | original work | MIT OR Apache-2.0 |
| `packaging/linux/dev.danielmiguelt.Hito.metainfo.xml.in` | AppStream metadata | Nodalia | original work | CC0-1.0 |
| `packaging/macos/dmg/background.svg`, `background.tiff` | Background of the macOS DMG window: a channel list, a conversation and a lock, in Hito's teal; text outlined from Sora | Nodalia | drawn by nodalia-design `tools/dmg-background/draw_background.py`; Sora by Jonathan Barnbrook and Julián Moncada | Artwork: MIT OR Apache-2.0. Sora outlines: OFL-1.1 |
| `packaging/macos/dmg/dmg-layout.DS_Store` | Finder layout of the DMG window | Nodalia | written by `packaging/macos/dmg/generate.py` | MIT OR Apache-2.0 |

Interface icons, colours and the window controls are drawn by `nodalia-theme`, from
[nodalia-design](https://github.com/getnodalia/nodalia-design) (MIT). Text uses egui's default
fonts (Ubuntu Light, Hack, Noto Emoji and emoji-icon-font, under their own open licences, inside
the `epaint` crate). Characters those fonts lack, such as most emoji, are drawn with fonts
already installed on the computer, read at run time and never shipped.
