# Change Log

All notable changes to the "quilt" extension will be documented in this file.

Check [Keep a Changelog](http://keepachangelog.com/) for recommendations on how to structure this file.

## [0.0.2]

- Embedded-language highlighting in annotated quotes: `rust↖`/`rs↖`, `python↖`/`py↖`,
  `wgsl↖`, `html↖`, and `bash↖`/`zsh↖`/`sh↖` bodies are now tokenized with the
  corresponding language grammar (`embeddedLanguages` mapped so commenting and
  bracket matching follow suit). Requires the matching language extension for
  non-built-in grammars (e.g. WGSL).
- Quote/unquote brackets `↖ ↗ ↙ ↘` (and `↔ ↕`) are now highlighted; previously the
  keyword pattern only covered `← ↑ ↓ →`.
- Quote language annotations (e.g. the `wgsl` in `wgsl↖`) get their own scope.

## [Unreleased]

- Initial release