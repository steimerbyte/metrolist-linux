# Bundled fonts

All fonts are redistributable; license texts are in this directory.

| File | Family / style | Size (bytes) | SHA-256 |
|---|---|---:|---|
| `SpaceGrotesk-Regular.ttf` | Space Grotesk Regular | 88324 | `8dc72eec1d9ae518152e18410bd5634aceb42855393dacfbf7de7b0b3dd51274` |
| `SpaceGrotesk-Medium.ttf` | Space Grotesk Medium | 88332 | `726bad76e2df54a20d649a494aba93f6b88b2ecbe1be2cc2fb4eb833a7ec7f7f` |
| `Doto-Bold.ttf` | Doto Bold | 140920 | `604a9651ebb92d9227c6c04ddee8f99433286aae60913eeb64ec1ac6f43057a1` |
| `SpaceMono-Regular.ttf` | Space Mono Regular | 99356 | `95837e182baeeada83368f7748db28357f0a1b75c6b84ff7065b5edf933c8e18` |
| `MaterialSymbolsRounded.ttf` | Material Symbols Rounded variable | 15235612 | `7e74e36533ccee12c2b81e75bfcf3ccf123d3bb7e7e46d65d94445f23db0c97c` |

## Provenance

- Space Grotesk Regular/Medium, Doto Bold, Space Mono Regular: byte-identical
  copies of the compile-time set in `crates/ui/assets/fonts/` (see that
  directory's `SOURCES.md` for the upstream URLs). All three families are
  SIL Open Font License 1.1: `OFL-SpaceGrotesk.txt`, `OFL-Doto.txt`,
  `OFL-SpaceMono.txt`.
- Material Symbols Rounded:
  `https://github.com/google/material-design-icons/blob/master/variablefont/MaterialSymbolsRounded%5BFILL%2CGRAD%2Copsz%2Cwght%5D.ttf`.
  License: Apache License 2.0, `Apache-2.0-MaterialSymbols.txt` (includes the
  Google copyright notice).

The shipped set is the Nothing triad: Space Grotesk for body and UI, Doto for
display sizes (>= 36 px), Space Mono for uppercase labels; Material Symbols
stays for the icon slots. Nothing has no separate bold cut, so medium carries
both weights. Re-copy from `crates/ui/assets/fonts/` after changing that set so
the hashes above stay true.
