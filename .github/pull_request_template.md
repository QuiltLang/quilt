## What

What does this PR change, and why? Reference the issue it addresses
(`Fixes #NN`) if there is one.

## How

Anything a reviewer should know about the approach or trade-offs.

## Checks

CI runs these in the Nix devShell; they can all be reproduced locally
(see CONTRIBUTING.md):

- [ ] `bin/ctest` — tests pass
- [ ] `bin/fmt-check` and `bin/lint -- -D warnings` — formatting + clippy clean
- [ ] `bin/check-bootstrap` — if you touched expansion or `mk_meta.rs.quilt` (never edit `meta.rs` by hand)
- [ ] `bin/check-examples` — committed example expansions still match
- [ ] `bin/ts-gen` output committed — if you changed `tree-sitter-quilt/grammar.js`
- [ ] Docs updated (`docs/wiki/`, README) if behavior or syntax changed
