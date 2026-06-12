---
name: Bug report
about: Something doesn't work the way it should
title: ""
labels: bug
assignees: ""
---

## What happened

A clear description of the bug.

## Reproduction

The `.quilt` source (or steps) that triggers it — minimal if possible. Paste
the arrow-bracket syntax verbatim in a fenced code block; the glyphs are
significant even inside comments.

```
<!-- your .quilt source here -->
```

How you ran it:

```sh
quilt expand path/to/file.rs.quilt   # or run / check / via the LSP / ...
```

## Expected vs. actual

- **Expected:**
- **Actual:** (error output, wrong generated code, …)

## Environment

- OS / platform:
- How you installed quilt (cargo install, from source, …):
- Version / commit:
