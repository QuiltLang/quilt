//! The Zsh target language.
//!
//! Zsh is only ever a *quoted* language (e.g. `zsh↖ … ↗` inside Rust): Quilt
//! parses Zsh fragments into a `QTerm` and the host language's `MetaLanguage`
//! drives expansion. There is therefore no `ZshMetaLanguage` — only this
//! `Language` impl, built on the shared tree-sitter helper.

use crate::{
    lang::{Arity, InnerKind},
    qterm::QTerm,
    treesitter::{DynTSLanguage, TSLanguage, TSProvider},
};
use miette::Result;
use tree_sitter::Parser;

/**************************************************************/

pub struct ZshProvider(tree_sitter::Parser);

impl Default for ZshProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::zsh::LANGUAGE.into())
            .expect("Error loading Zsh parser");
        Self(parser)
    }
}

impl TSProvider for ZshProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser {
        &mut self.0
    }

    fn hole_str(&self) -> &'static str {
        // The grammar.js fork already defines `quilt_hole` and adds it to
        // statement/expression positions, but parser.c hasn't been regenerated
        // yet (tree-sitter generate takes very long on this grammar).  Until
        // then `__QUILT_HOLE__` parses as a `word` node, and range-based hole
        // detection in `treesitter.rs` still works correctly.
        "__QUILT_HOLE__"
    }

    /// Squash the `program` wrapper around a single quoted fragment so the
    /// term is the fragment itself (command / statement). A multi-statement
    /// fragment (a whole script) stays a `program`.
    fn unwrap(&self, qterm: QTerm, _ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)> {
        let QTerm::Tuple { tag, terms, .. } = &qterm else {
            return Ok((qterm, InnerKind::default()));
        };
        if &**tag != "program" {
            return Ok((qterm, InnerKind::default()));
        }
        if terms.len() != 1 {
            return Ok((qterm, InnerKind::File));
        }
        let kind = match &*terms[0] {
            QTerm::Tuple { tag, .. } if is_expr_tag(tag) => InnerKind::Expr,
            _ => InnerKind::Stmt,
        };
        Ok((qterm.squash(), kind))
    }

    /// Zsh's variadic containers.
    ///
    /// The first block is the shell-shared set, kept in the same order as
    /// [`crate::langs::bash::lang::BashProvider::arity`] so the two read as
    /// diffable copies of one table; `shell_arity_tables_agree` in
    /// `quilt-conformance/tests/grammar_tags.rs` fails if they disagree about
    /// any node kind both grammars define (issue #150). The second block is
    /// what zsh has and bash does not, so it has no counterpart to match.
    fn arity(&self, tag: &str) -> Arity {
        match tag {
            "program"
            | "compound_statement"
            | "subshell"
            | "list"
            | "pipeline"
            | "command"
            | "command_name"
            | "command_substitution"
            | "process_substitution"
            | "if_statement"
            | "elif_clause"
            | "else_clause"
            | "case_statement"
            | "case_item"
            | "do_group"
            | "for_statement"
            | "c_style_for_statement"
            | "while_statement"
            | "function_definition"
            | "redirected_statement"
            | "file_redirect"
            | "heredoc_redirect"
            | "herestring_redirect"
            | "variable_assignment"
            | "variable_assignments"
            | "declaration_command"
            | "unset_command"
            | "negated_command"
            | "test_command"
            | "string"
            | "raw_string"
            | "ansi_c_string"
            | "translated_string"
            | "concatenation"
            | "array"
            | "expansion"
            | "brace_expression"
            | "arithmetic_expansion"
            | "binary_expression"
            | "unary_expression"
            | "ternary_expression"
            | "postfix_expression"
            | "parenthesized_expression"
            | "subscript"
            | "number"
            | "heredoc_body"
            // zsh-only: absent from the bash grammar.
            | "compound_statement_no_always"
            | "expansion_default_list"
            | "select_statement"
            | "repeat_statement"
            | "dollar_variable"
            | "variable_ref"
            | "zsh_array_subscript_flags" => Arity::Variadic,
            _ => Arity::Unknown,
        }
    }

    fn hashbang(&self) -> Option<&'static str> {
        Some("#!/usr/bin/env zsh")
    }
}

/// Tags that are Zsh "expressions".
///
/// Aligned with bash's list — see the note on [`ZshProvider::arity`]; the
/// trailing two are zsh's own spelling of an expansion, which bash writes as
/// `simple_expansion`. This list had drifted from bash's the same way `arity`
/// had, omitting `raw_string`, `ansi_c_string`, `translated_string` and
/// `process_substitution`.
///
/// Unlike `arity`, the divergence was latent rather than observable: the
/// `InnerKind` this feeds is the second half of `unwrap`'s return, which its
/// only caller currently discards (`let (qterm, _ikind) = …` in
/// `treesitter.rs`). Aligning it now means the two shells will not disagree once
/// that kind is threaded through.
fn is_expr_tag(tag: &str) -> bool {
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
            | "concatenation"
            | "variable_ref"
            | "dollar_variable"
    )
}

pub type ZshLanguage = TSLanguage<ZshProvider>;
pub type DynZshLanguage = DynTSLanguage<ZshProvider>;
