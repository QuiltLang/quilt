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
use tree_sitter::Parser;

/**************************************************************/

pub struct ZshProvider(tree_sitter::Parser);

impl Default for ZshProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_zsh::LANGUAGE.into())
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
    fn unwrap(&self, qterm: QTerm, _ikind: Option<InnerKind>) -> (QTerm, InnerKind) {
        let QTerm::Tuple { tag, terms, .. } = &qterm else {
            return (qterm, InnerKind::default());
        };
        if &**tag != "program" {
            return (qterm, InnerKind::default());
        }
        if terms.len() != 1 {
            return (qterm, InnerKind::File);
        }
        let kind = match &*terms[0] {
            QTerm::Tuple { tag, .. } if is_expr_tag(tag) => InnerKind::Expr,
            _ => InnerKind::Stmt,
        };
        (qterm.squash(), kind)
    }

    fn arity(&self, tag: &str) -> Arity {
        match tag {
            "program"
            | "compound_statement"
            | "compound_statement_no_always"
            | "list"
            | "pipeline"
            | "command"
            | "command_name"
            | "command_substitution"
            | "subshell"
            | "if_statement"
            | "elif_clause"
            | "else_clause"
            | "case_statement"
            | "case_item"
            | "do_group"
            | "redirected_statement"
            | "variable_assignments"
            | "declaration_command"
            | "negated_command"
            | "string"
            | "concatenation"
            | "array"
            | "expansion"
            | "expansion_default_list"
            | "brace_expression"
            | "arithmetic_expansion"
            | "binary_expression"
            | "unary_expression"
            | "postfix_expression"
            | "parenthesized_expression"
            | "number"
            | "heredoc_body"
            | "heredoc_redirect"
            | "herestring_redirect"
            | "process_substitution"
            | "select_statement"
            | "repeat_statement"
            | "unset_command"
            | "dollar_variable"
            | "variable_ref"
            | "translated_string"
            | "zsh_array_subscript_flags" => Arity::Variadic,
            _ => Arity::Unknown,
        }
    }

    /// Classify a Zsh tag as expression, statement, or file-level.
    fn typ(&self, tag: &str) -> InnerKind {
        if tag == "program" {
            InnerKind::File
        } else if is_expr_tag(tag) {
            InnerKind::Expr
        } else {
            InnerKind::Stmt
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
