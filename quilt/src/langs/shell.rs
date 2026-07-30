//! Tag tables shared by the two shell dialects (issue #150).
//!
//! `tree-sitter-zsh` is a fork of `tree-sitter-bash`, and `concrete-languages.md`
//! documents the two Quilt languages as near-equivalent — "a separate target with
//! Bash-specific quoting semantics". Their `Language::arity` tables were
//! nevertheless maintained as two independent `match` arms in two files, and
//! drifted: bash claimed `for_statement`, `while_statement`, `function_definition`
//! and nine more that zsh's table omitted despite zsh's grammar defining every
//! one of them.
//!
//! That drift is silent. `arity` decides whether the expander wraps a node's
//! children for emission (`Arity::Variadic` → `build_variadic_block`, so `←` and
//! statement-position quotes append into a `b_` accumulator; anything else →
//! `build_tuple_code`, fixed positional children). So an emit into a zsh `for`
//! body compiled differently from the identical bash one, with no diagnostic.
//!
//! One table for both dialects makes that failure mode structural rather than a
//! matter of remembering to edit two files. A tag that only one grammar defines
//! is harmless in the shared table — it simply never matches for the other
//! dialect, and `quilt-conformance`'s `variadic_tags_snapshot` reports the
//! *effective* set per language (table ∩ grammar), so the dialect-only entries
//! stay visible in review.

use crate::lang::Arity;

/// Which shell node kinds hold a sequence their children can be emitted into.
///
/// Grouped by role rather than alphabetically, because the question a reader
/// arrives with is "can `←` append here", and that is a property of the
/// construct, not of its spelling.
#[must_use]
pub fn arity(tag: &str) -> Arity {
    match tag {
        // Scripts and statement containers. `do_group` is the body of every
        // loop, `compound_statement` of `{ … }` and of a function.
        "program"
        | "compound_statement"
        | "subshell"
        | "list"
        | "pipeline"
        | "do_group"
        | "if_statement"
        | "elif_clause"
        | "else_clause"
        | "case_statement"
        | "case_item"
        | "for_statement"
        | "c_style_for_statement"
        | "while_statement"
        | "function_definition"
        | "redirected_statement"
        | "negated_command"
        | "command"
        | "command_name"
        // Commands whose grammar rule is a `repeat1` over their operands.
        | "declaration_command"
        | "unset_command"
        | "test_command"
        // Redirections. `heredoc_body` holds the interpolated body content.
        | "file_redirect"
        | "heredoc_redirect"
        | "herestring_redirect"
        | "heredoc_body"
        // Assignments. Only the *plural* node is a container: `variable_assignments`
        // is the `X=1 Y=2` prefix of a command, a genuine `repeat1`, while
        // `variable_assignment` is a fixed name/`=`/value triple. bash's table
        // claimed both — the copy-paste divergence issue #150 names — which made
        // `X=…` generate a `b_` block that can only ever take one child.
        | "variable_assignments"
        // Words and expansions: each of these concatenates a run of parts.
        | "string"
        | "raw_string"
        | "ansi_c_string"
        | "translated_string"
        | "concatenation"
        | "array"
        | "subscript"
        | "expansion"
        | "command_substitution"
        | "process_substitution"
        | "arithmetic_expansion"
        // Arithmetic and test expressions.
        | "brace_expression"
        | "parenthesized_expression"
        | "binary_expression"
        | "unary_expression"
        | "ternary_expression"
        | "postfix_expression"
        | "number"
        // bash-only kinds. zsh spells `$x` as `variable_ref` / `dollar_variable`
        // below and has no `simple_expansion`.
        | "simple_expansion"
        // zsh-only kinds. The first group are the dialect's own statement
        // containers — `{ … } always { … }`, `for x (a b c) cmd`, `select`,
        // `repeat n { … }`, `coproc` — i.e. the zsh analogues of the shared
        // entries above, so they answer `←` the same way.
        | "compound_statement_no_always"
        | "always_clause"
        | "terse_for_statement"
        | "select_statement"
        | "repeat_statement"
        | "coprocess_statement"
        // The rest are zsh's extra expansion vocabulary.
        | "expansion_default_list"
        | "dollar_variable"
        | "variable_ref"
        | "zsh_array_subscript_flags" => Arity::Variadic,
        _ => Arity::Unknown,
    }
}

/// Tags that are shell "expressions" rather than statements.
///
/// Used only to label a squashed single-fragment quote (`TSProvider::unwrap`),
/// so the label is advisory — but it was drifting the same way `arity` was, and
/// for the same reason, so it is shared for the same reason.
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
