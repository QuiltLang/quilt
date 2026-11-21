# Release Notes

Each `v*` tag on [QuiltLang/quilt](https://github.com/QuiltLang/quilt) publishes every channel at the same version:

| Channel | Package |
|---|---|
| crates.io | [`quiltlang`](https://crates.io/crates/quiltlang), [`quilt-lsp`](https://crates.io/crates/quilt-lsp) |
| PyPI | [`quilt-python`](https://pypi.org/project/quilt-python/) (imported as `quilt`) |
| npm | [`quilt-wasm`](https://www.npmjs.com/package/quilt-wasm) |
| Open VSX | [`quiltlang.quiltlang`](https://open-vsx.org/extension/quiltlang/quiltlang) |
| GitHub Releases | prebuilt `quilt` + `quilt-lsp` binaries and the VS Code `.vsix` |

Older releases (v0.1.0-alpha through v0.5.0) are listed on the [GitHub Releases page](https://github.com/QuiltLang/quilt/releases).

## Releases

| Version | Date | Highlights |
|---|---|---|
| [v0.6.0](releases/v0.6.0.md) | 2026-11-21 | Lean 4, SQL and Nix; hand-written parser; machines and live notebooks; new demos; conformance harness |

## Adding a release

Each release gets its own page, `docs/wiki/releases/vX.Y.Z.md`, so that it has its own URL and its own social-sharing image. Copy the front matter from the latest release and fill it in:

- `description` is the link-preview text.
- `image` must be `/img/releases/vX.Y.Z.png`. The website renders that image (1200×630) from the `card` block at build time ([`scripts/release-cards.mjs`](https://github.com/QuiltLang/quiltlang.github.io/blob/main/docs-site/scripts/release-cards.mjs)), so there is nothing to draw or commit.
- `card.headline` and `card.highlights` are the text on the image. Quilt's glyph arrows are drawn in their colours.

Then add a row to the table above. The sidebar lists release pages newest first on its own. To preview a card locally, run `npm run release-cards` in the website's `docs-site/` and open `static/img/releases/vX.Y.Z.png`.
