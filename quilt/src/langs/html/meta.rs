//! The HTML meta-language: an identity host, like text.
//!
//! Markup computes nothing, so there is no code an `html↖…↗` quote could be
//! *translated into* when HTML is the ground language. What an HTML host can
//! do is hold: an `.html.quilt` file expands to itself, with every quoted
//! cell (`py↖…↗`, `sql↖…↗`, …) kept verbatim — the same
//! [`HoldMetaLanguage`](crate::langs::hold::HoldMetaLanguage) the text host
//! is, under HTML's name.
//!
//! Two things follow, and both are the point. `quilt check nb.html.quilt`
//! parses every cell with its own language's grammar and expands nothing,
//! so a notebook is validated without a single cell running. And the
//! computation lives somewhere honest instead: `quilt notebook` (see
//! `crate::notebook`) walks the held cells, feeds each to its language's
//! machine, and feeds the results back into the page — the HTML *machine*
//! is where HTML "runs", not its meta.
//!
//! Unlike text, this meta *is* registered in `Omni`, because a `.html.quilt`
//! file is a first-class input to the CLI.

use crate::langs::hold::{HoldMetaLanguage, Holds};

/**************************************************************/

/// The HTML dialect marker for the identity meta.
pub struct Html;

impl Holds for Html {
    const NAME: &'static str = "html";
    const ANNO: &'static str = "html";
}

pub type HtmlMetaLanguage = HoldMetaLanguage<Html>;

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langs::omni::Omni;
    use crate::meta::MetaLanguage as _;
    use crate::prelude::*;
    use crate::term::STerm as _;

    fn expand(src: &str) -> Result<String> {
        let mut multi = Omni::default();
        let parsed = multi.parse_chain(&["html"], src)?;
        Ok(multi.expand_lang("html", &parsed)?.coparse())
    }

    /// A page with cells expands to itself with each cell contributing its
    /// body — the brackets are staging, not content, exactly as in the text
    /// host — and nothing runs. (`quilt expand` on a notebook does not stop
    /// here: it renders the notebook, see `bin.rs`; this is what `quilt
    /// check` validates.)
    #[test]
    fn a_notebook_expands_to_its_held_cells() -> Result<()> {
        let src =
            "<div class=\"row\">\n  py↖\n    x = 5\n    print(x)\n  ↗\n  sql↖SELECT 1;↗\n</div>\n";
        assert_eq!(
            expand(src)?,
            "<div class=\"row\">\n  x = 5\n  print(x)\n  SELECT 1;\n</div>\n"
        );
        Ok(())
    }

    /// A cell's *nested* quotes belong to the cell's own stage, so they are
    /// held with their glyphs.
    #[test]
    fn nested_quotes_in_a_cell_are_held() -> Result<()> {
        assert_eq!(
            expand("<p>py↖print(html↖<b>x</b>↗.coparse())↗</p>")?,
            "<p>print(html↖<b>x</b>↗.coparse())</p>"
        );
        Ok(())
    }

    /// The cells *are* parsed by their own grammars — a syntax error in a
    /// cell fails the expansion, which is what `quilt check` reports.
    #[test]
    fn a_cell_with_a_syntax_error_is_rejected() {
        assert!(expand("<p>py↖def oops(:↗</p>").is_err());
    }

    /// The refusals name html, not text.
    #[test]
    fn refusals_name_html() {
        let meta = HtmlMetaLanguage::default();
        let err = meta.lift_str("html").unwrap_err().to_string();
        assert!(err.starts_with("html can't lift"), "{err}");
        assert!(err.contains("`html↖ … ↗`"), "{err}");
    }
}
