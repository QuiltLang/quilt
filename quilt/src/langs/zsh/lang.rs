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

    /// Kept deliberately in the same order as [`super::super::bash`]'s table, so
    /// the two read side by side. Zsh shares bash's grammar lineage, so every
    /// node kind both grammars define must classify the same way in both — an
    /// emit into a zsh `for` body has no business behaving differently from the
    /// identical bash one (#150). `bash_and_zsh_agree_on_shared_kinds` in
    /// `quilt-conformance/tests/grammar_tags.rs` enforces exactly that, so the
    /// only entries that may differ are the ones the other grammar has no kind
    /// for, grouped at the end.
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
            // Zsh-only kinds — the bash grammar defines none of these, which is
            // why bash's table has no counterpart to them. (Bash's one unshared
            // entry is `simple_expansion`; zsh spells the same construct
            // `dollar_variable` / `variable_ref`.)
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
fn is_expr_tag(tag: &str) -> bool {
    matches!(
        tag,
        "word"
            | "string"
            | "number"
            | "binary_expression"
            | "unary_expression"
            | "postfix_expression"
            | "ternary_expression"
            | "parenthesized_expression"
            | "brace_expression"
            | "arithmetic_expansion"
            | "command_substitution"
            | "variable_ref"
            | "dollar_variable"
            | "expansion"
            | "concatenation"
    )
}

pub type ZshLanguage = TSLanguage<ZshProvider>;
pub type DynZshLanguage = DynTSLanguage<ZshProvider>;
