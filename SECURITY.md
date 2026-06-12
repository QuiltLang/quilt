# Security Policy

## Supported versions

Quilt is in early alpha and has not had a stable release yet. Only the latest
state of the `main` branch (and the most recent tagged pre-release, once one
exists) receives security fixes.

## Reporting a vulnerability

Please **do not** open a public issue for security problems.

Instead, report vulnerabilities privately via
[GitHub's private vulnerability reporting](https://github.com/QuiltLang/quilt/security/advisories/new)
("Report a vulnerability" under the repository's **Security** tab), or by
email to **varga.alex@gmail.com**.

Include what you'd put in a good bug report: the affected component (`quilt`
CLI / library, `quilt-lsp`, `quilt_python` bindings, the VS Code extension),
a reproduction, and the impact as you understand it.

You should receive an acknowledgement within a few days. Since this is a
small, pre-release project, there is no formal SLA, but reports will be
triaged and fixed as quickly as practical, and you'll be credited in the fix
unless you prefer otherwise.

## Scope notes

Keep in mind that Quilt is a code-generation tool: `quilt run` and
`quilt expand` **execute the generation-time code in the input file by
design** (Rust via rust-script, Python via python3). Running an untrusted
`.quilt` file is equivalent to running an untrusted program — that alone is
not a vulnerability. Bugs where Quilt does something its documentation says
it won't (for example, executing code during a plain `quilt check`/parse)
are in scope.
