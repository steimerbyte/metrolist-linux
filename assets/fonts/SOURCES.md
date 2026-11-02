# Bundled fonts

All fonts are redistributable; license texts are in this directory.

| File | Family / style | Size (bytes) | SHA-256 |
|---|---|---:|---|
| `Roboto-Regular.ttf` | Roboto Regular (wght 400, wdth 100) | 157584 | `8a8ff6c1fec0dea6c0bd3214717f0a08efa2062266f2e934ef20e2443d6c9972` |
| `Roboto-Medium.ttf` | Roboto Medium (wght 500, wdth 100) | 157732 | `436f8a1cd0730ca5d1b6719106eddf5e5fdd648fc7afe650a3884f0df3cda2e3` |
| `Roboto-Bold.ttf` | Roboto Bold (wght 700, wdth 100) | 158288 | `af2af433e294176f25517b800f536f37ce4f17f94814966ded2e1a9a5a240911` |
| `MaterialSymbolsRounded.ttf` | Material Symbols Rounded variable | 15235612 | `7e74e36533ccee12c2b81e75bfcf3ccf123d3bb7e7e46d65d94445f23db0c97c` |

## Provenance

- Roboto: `https://github.com/google/fonts/blob/main/ofl/roboto/Roboto%5Bwdth%2Cwght%5D.ttf`
  (`sha256 d7598e12c5dbef095ff8272cfc55da0250bd07fbdecbac8a530b9b277872a134`).
  Static weights were generated with `fonttools varLib.instancer` (fontTools
  4.68.0 with `--update-name-table`) at `wght=400/500/700`, `wdth=100`,
  preserving the OFL grant. License:
  SIL Open Font License 1.1, `OFL-1.1-Roboto.txt`.
- Material Symbols Rounded:
  `https://github.com/google/material-design-icons/blob/master/variablefont/MaterialSymbolsRounded%5BFILL%2CGRAD%2Copsz%2Cwght%5D.ttf`.
  License: Apache License 2.0, `Apache-2.0-MaterialSymbols.txt` (includes the
  Google copyright notice).

Rendered names were checked with `fc-scan`: Roboto Regular/Medium/Bold and
Material Symbols Rounded family. Do not regenerate the static Roboto files from
a different variable source without updating these hashes.
