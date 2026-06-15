# Change Log

All notable changes to the "quilt" extension will be documented in this file.

Check [Keep a Changelog](http://keepachangelog.com/) for recommendations on how to structure this file.

## [Unreleased]

- Renamed the extension's package `name` from `quilt` to `quiltlang`, so it now
  publishes and installs as `quiltlang.quiltlang` (publisher unchanged), and
  changed the human-facing `displayName` from "Quilt" to "QuiltLang" (the VS
  Code Marketplace requires display names to be globally unique, and "Quilt"
  was already taken).
- The published version now comes from the release tag (`v1.2.3` → `1.2.3`)
  rather than the hardcoded `version` in `package.json`, so every `v*` tag
  publishes a matching extension version with no separate manual bump.

## [0.1.0]

- Filled in extension metadata for the alpha: `publisher` (`quiltlang`),
  `description`, `license` (`MIT OR Apache-2.0`), and `repository`
  (QuiltLang/quilt, `tools/quilt`).
- For the alpha the extension is installed manually via `bin/install_tools`
  (symlink into `~/.vscode/extensions`); Marketplace publishing is deferred.

## [0.0.2]

- Embedded-language highlighting in annotated quotes: `rust↖`/`rs↖`, `python↖`/`py↖`,
  `wgsl↖`, `html↖`, and `bash↖`/`zsh↖`/`sh↖` bodies are now tokenized with the
  corresponding language grammar (`embeddedLanguages` mapped so commenting and
  bracket matching follow suit). Requires the matching language extension for
  non-built-in grammars (e.g. WGSL).
- Quote/unquote brackets `↖ ↗ ↙ ↘` (and `↔ ↕`) are now highlighted; previously the
  keyword pattern only covered `← ↑ ↓ →`.
- Quote language annotations (e.g. the `wgsl` in `wgsl↖`) get their own scope.

## [0.0.1]

- Initial release