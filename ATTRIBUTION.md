# Attribution

Every asset in this repository, with its author, source and licence. Code is `MIT OR Apache-2.0`
(`LICENSE-MIT`, `LICENSE-APACHE`); third-party Rust crates keep their own licences.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|
| `assets/icons/hito-portrait-1024.png` | App icon artwork: Lillian Moller Gilbreth (1878–1972), a pioneer of motion study and of how work is planned and measured, cut out in black and white on Hito's green | Nodalia, from a portrait photograph of Gilbreth (Smithsonian Institution) | [Wikimedia Commons: Lillian Moller Gilbreth.jpg](https://commons.wikimedia.org/wiki/File:Lillian_Moller_Gilbreth.jpg); made with nodalia-design `tools/app-portrait` | Photograph: no known restrictions (Smithsonian Institution). Artwork: MIT OR Apache-2.0 |
| `assets/icons/hicolor/*/apps/dev.danielmiguelt.Hito.png` | App icon at each size (rounded tile of the artwork above) | Nodalia | made from the artwork above | MIT OR Apache-2.0 |
| `docs/screenshots/*.png` | Screenshots of Hito with demo accounts and projects | Nodalia | taken from the running app | MIT OR Apache-2.0 |
| `packaging/linux/dev.danielmiguelt.Hito.desktop` | Desktop entry | Nodalia | original work | MIT OR Apache-2.0 |
| `packaging/linux/dev.danielmiguelt.Hito.metainfo.xml.in` | AppStream metadata | Nodalia | original work | CC0-1.0 |
| `packaging/macos/dmg/background.svg`, `background.tiff` | Background of the macOS DMG window: a progress chart, in Hito's green; text outlined from Sora | Nodalia | drawn by nodalia-design `tools/dmg-background/draw_background.py`; Sora by Jonathan Barnbrook and Julián Moncada | Artwork: MIT OR Apache-2.0. Sora outlines: OFL-1.1 |
| `packaging/macos/dmg/dmg-layout.DS_Store` | Finder layout of the DMG window | Nodalia | written by `packaging/macos/dmg/generate.py` | MIT OR Apache-2.0 |

Interface icons, colours and the window controls are drawn by `nodalia-theme`, from
[nodalia-design](https://github.com/getnodalia/nodalia-design) (MIT). Text uses egui's default
fonts (Ubuntu Light, Hack, Noto Emoji and emoji-icon-font, under their own open licences, inside
the `epaint` crate). Characters those fonts lack, such as most emoji, are drawn with fonts
already installed on the computer, read at run time and never shipped.
