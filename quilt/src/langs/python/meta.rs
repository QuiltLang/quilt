use miette::Result;

use super::ops::{build_quote_code, build_tuple_code, build_unquote_code, build_variadic_block};
use crate::lang::Arity;
use crate::prelude::{Index, *};
use crate::{meta::MetaLanguage, qterm::QTerm, term::CmdOrHole};

/**************************************************************/

#[derive(Default)]
pub struct PythonMetaLanguage;

impl MetaLanguage for PythonMetaLanguage {
    fn expand_quote(
        &self,
        _lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        Ok(build_quote_code(tag, i, lang2, qterm, cmds))
    }

    fn expand_unquote(
        &self,
        _lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        Ok(build_unquote_code(tag, i, lang2, qterm, cmds))
    }

    fn expand_tuple(
        &self,
        _lang1: &str,
        tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        arity: Arity,
    ) -> Result<Arc<QTerm>> {
        Ok(if arity == Arity::Variadic {
            build_variadic_block(tag, cmds, qterms)
        } else {
            build_tuple_code(tag, cmds, qterms)
        })
    }

    // `↑` is target-directed and written prefix, `↑(value)`: into Python it
    // spells the `qlift` function (int, str, or QTerm — a method can't hang
    // off builtin ints); into HTML it spells `qlift_html`, which
    // entity-escapes lifted strings at runtime. Both live in the
    // `quilt_python` runtime.
    fn lift_str(&self, target: &str) -> Result<&'static str> {
        match target {
            "python" | "py" => Ok("qlift"),
            "html" => Ok("qlift_html"),
            _ => miette::bail!("python can't lift into {target:?}: no spelling registered"),
        }
    }

    fn reduce_str(&self, target: &str) -> Result<&'static str> {
        match target {
            "" | "python" | "py" => Ok("reduce()"),
            "rust" | "rs" => Ok("reduce_rs()"),
            _ => miette::bail!("python can't reduce via {target:?}: no reduce_spelling registered"),
        }
    }

    fn name_str(&self) -> Result<&'static str> {
        Ok("name")
    }

    fn type_str(&self) -> Result<&'static str> {
        Ok("QTerm")
    }

    /// No spelling: `←` needs a named `b_` accumulator in scope, and this host
    /// has none (issue #152).
    ///
    /// Rust's variadic block is a *block expression*, so it can bind
    /// `let mut b_ = tb(..)` and let ground statements append to it. Python has
    /// no statement-block in expression position, so
    /// [`build_variadic_block`](super::ops::build_variadic_block) emits the
    /// fluent `tb(..).e(child).b()` chain instead — nothing to bind, and the
    /// hole sits in argument position where a ground `for` is a syntax error.
    /// The `quilt_python` runtime also exposes no `emit` method on a term, so
    /// even a `b_` in scope would have nothing to call.
    ///
    /// This used to return `"emit(b_)"`, which quietly expanded a ground `←`
    /// into generated Python referencing an undefined name — the same
    /// silent-corruption failure the string-based hosts had in #190, and the
    /// reason this accessor returns `Result` at all.
    ///
    /// The working alternative needs no accumulator from *us*: ground Python
    /// builds the sequence with its own builder and splices the finished term,
    /// which goes through `wrap_child` and the `.e(..)` chain as usual.
    ///
    /// ```python
    /// b = tb("block")
    /// for n in names:
    ///     b.e(↖print(↙name(n)↘)↗)
    /// body = b.b()
    /// out = ↖def f():
    ///     ↙body↘
    /// ↗
    /// ```
    fn emit_str(&self) -> Result<&'static str> {
        miette::bail!(
            "python can't emit `←`: the fluent `.e(child)` chain has no named `b_` accumulator \
             to emit into — build the sequence with your own `tb(..)` builder in ground code and \
             splice the finished term with `↙…↘`"
        )
    }
}
