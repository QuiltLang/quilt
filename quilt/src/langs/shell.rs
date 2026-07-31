//! Tag tables shared by the two shell dialects (issue #150).
//!
//! `tree-sitter-zsh` is a fork of `tree-sitter-bash`, and `concrete-languages.md`
//! documents the two Quilt languages as near-equivalent — "a separate target with
//! Bash-specific quoting semantics". Tag sets maintained as two independent
//! `match` arms in two files therefore drift, silently, and that is what issue
//! #150 found.
//!
//! The `arity` half of that drift is no longer answered here. Issue #202 derives
//! each language's variadic tags from its own vendored `grammar.json`
//! (`quilt/src/langs/arity.rs`, written by `bin/gen-arity`), which fixes the
//! drift at its source rather than by asking two dialects to share one
//! hand-written answer: a construct the two grammars spell the same way now
//! classifies the same way because the *grammars* agree, and where they genuinely
//! differ — zsh's `function_definition` is `repeat1(field('name', …))`, so
//! `function a b c { … }` defines three functions at once — the tables differ
//! too, truthfully. `bash_and_zsh_agree_on_shared_kinds` in
//! `quilt-conformance/tests/grammar_tags.rs` guards that weaker claim and pins
//! each exception with its reason.
//!
//! What stays here is the part no grammar rule answers: which tags name an
//! *expression* rather than a statement. That is a Quilt-level judgement about
//! how to label a squashed fragment, so it has no derivation to fall back on and
//! is shared for exactly the reason #150 gives.

/// Tags that are shell "expressions" rather than statements.
///
/// Used only to label a squashed single-fragment quote (`TSProvider::unwrap`),
/// so the label is advisory — but it was drifting the same way the arity tables
/// were, and for the same reason, so it is shared for the same reason.
#[must_use]
pub fn is_expr_tag(tag: &str) -> bool {
    matches!(
        tag,
        "word"
            | "string"
            | "raw_string"
            | "ansi_c_string"
            | "translated_string"
            | "number"
            | "binary_expression"
            | "unary_expression"
            | "ternary_expression"
            | "postfix_expression"
            | "parenthesized_expression"
            | "brace_expression"
            | "arithmetic_expansion"
            | "command_substitution"
            | "process_substitution"
            | "expansion"
            // bash-only.
            | "simple_expansion"
            // zsh-only.
            | "variable_ref"
            | "dollar_variable"
            | "concatenation"
    )
}
