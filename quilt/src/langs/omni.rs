//! `Multi`s containing all languages enabled by Rust features.

#[cfg(feature = "bash")]
use super::bash::lang::BashLanguage;
#[cfg(feature = "html")]
use super::html::lang::HtmlLanguage;
#[cfg(feature = "python")]
use super::python::lang::PythonLanguage;
#[cfg(feature = "rust")]
use super::rust::lang::RustLanguage;
#[cfg(feature = "text")]
use super::text::lang::TextLanguage;
#[cfg(feature = "wgsl")]
use super::wgsl::lang::WgslLanguage;
#[cfg(feature = "zsh")]
use super::zsh::lang::ZshLanguage;
use crate::lang::{Arity, FlatNode, Hole, InnerKind, Language, LanguagePost};
#[cfg(feature = "bash")]
use crate::langs::bash::lang::DynBashLanguage;
#[cfg(feature = "html")]
use crate::langs::html::lang::DynHtmlLanguage;
#[cfg(feature = "python")]
use crate::langs::python::{lang::DynPythonLanguage, meta::PythonMetaLanguage};
#[cfg(feature = "rust")]
use crate::langs::rust::{lang::DynRustLanguage, meta::RustMetaLanguage};
#[cfg(feature = "text")]
use crate::langs::text::lang::DynTextLanguage;
#[cfg(feature = "wgsl")]
use crate::langs::wgsl::lang::DynWgslLanguage;
#[cfg(feature = "zsh")]
use crate::langs::zsh::lang::DynZshLanguage;
use crate::meta::{MetaLanguage, OuterKind};
use crate::multi::{DictMulti, Languages, MetaLanguages, Multi};
use crate::prelude::*;
use crate::qterm::QTerm;
use crate::term::CmdOrHole;
use miette::miette;

/**************************************************************/

/// An "Omni"-language using enums.
pub struct OmniLanguages {
    #[cfg(feature = "bash")]
    bash: OmniLanguage,
    #[cfg(feature = "html")]
    html: OmniLanguage,
    #[cfg(feature = "python")]
    py: OmniLanguage,
    #[cfg(feature = "rust")]
    rs: OmniLanguage,
    #[cfg(feature = "text")]
    txt: OmniLanguage,
    #[cfg(feature = "wgsl")]
    wgsl: OmniLanguage,
    #[cfg(feature = "zsh")]
    zsh: OmniLanguage,
}

impl Default for OmniLanguages {
    #[allow(clippy::default_constructed_unit_structs)]
    fn default() -> Self {
        Self {
            #[cfg(feature = "bash")]
            bash: OmniLanguage::Bash(BashLanguage::default()),
            #[cfg(feature = "html")]
            html: OmniLanguage::Html(HtmlLanguage::default()),
            #[cfg(feature = "python")]
            py: OmniLanguage::Python(PythonLanguage::default()),
            #[cfg(feature = "rust")]
            rs: OmniLanguage::Rust(RustLanguage::default()),
            #[cfg(feature = "text")]
            txt: OmniLanguage::Text(TextLanguage::default()),
            #[cfg(feature = "wgsl")]
            wgsl: OmniLanguage::Wgsl(WgslLanguage::default()),
            #[cfg(feature = "zsh")]
            zsh: OmniLanguage::Zsh(ZshLanguage::default()),
        }
    }
}

impl Languages for OmniLanguages {
    type Language = OmniLanguage;

    fn get(&self, lang: &str) -> Result<&Self::Language> {
        match lang {
            #[cfg(feature = "bash")]
            "bash" => Ok(&self.bash),
            #[cfg(feature = "html")]
            "html" => Ok(&self.html),
            #[cfg(feature = "python")]
            "python" | "py" => Ok(&self.py),
            #[cfg(feature = "rust")]
            "rust" | "rs" => Ok(&self.rs),
            #[cfg(feature = "text")]
            "text" | "txt" => Ok(&self.txt),
            #[cfg(feature = "wgsl")]
            "wgsl" => Ok(&self.wgsl),
            #[cfg(feature = "zsh")]
            "zsh" => Ok(&self.zsh),
            _ => Err(miette!("{lang:?} can't be used as Language")),
        }
    }
    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::Language> {
        match lang {
            #[cfg(feature = "bash")]
            "bash" => Ok(&mut self.bash),
            #[cfg(feature = "html")]
            "html" => Ok(&mut self.html),
            #[cfg(feature = "python")]
            "python" | "py" => Ok(&mut self.py),
            #[cfg(feature = "rust")]
            "rust" | "rs" => Ok(&mut self.rs),
            #[cfg(feature = "text")]
            "text" | "txt" => Ok(&mut self.txt),
            #[cfg(feature = "wgsl")]
            "wgsl" => Ok(&mut self.wgsl),
            #[cfg(feature = "zsh")]
            "zsh" => Ok(&mut self.zsh),
            _ => Err(miette!("{lang:?} can't be used as Language")),
        }
    }
}

pub struct OmniMetaLanguages {
    #[cfg(feature = "python")]
    py: OmniMetaLanguage,
    #[cfg(feature = "rust")]
    rs: OmniMetaLanguage,
}

impl Default for OmniMetaLanguages {
    #[allow(clippy::default_constructed_unit_structs)]
    fn default() -> Self {
        Self {
            #[cfg(feature = "python")]
            py: OmniMetaLanguage::Python(PythonMetaLanguage::default()),
            #[cfg(feature = "rust")]
            rs: OmniMetaLanguage::Rust(RustMetaLanguage::default()),
        }
    }
}

impl MetaLanguages for OmniMetaLanguages {
    type MetaLanguage = OmniMetaLanguage;

    fn get(&self, lang: &str) -> Result<&Self::MetaLanguage> {
        match lang {
            #[cfg(feature = "python")]
            "python" | "py" => Ok(&self.py),
            #[cfg(feature = "rust")]
            "rust" | "rs" => Ok(&self.rs),
            _ => Err(miette!("{lang:?} can't be used as MetaLanguage")),
        }
    }
    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::MetaLanguage> {
        match lang {
            #[cfg(feature = "python")]
            "python" | "py" => Ok(&mut self.py),
            #[cfg(feature = "rust")]
            "rust" | "rs" => Ok(&mut self.rs),
            _ => Err(miette!("{lang:?} can't be used as MetaLanguage")),
        }
    }
}

// pub enum Foo {}

// impl Foo {
//     pub fn foo(&self) {
//         match self {
//             _ => {}
//         }
//     }
// }

/// A `Language` used by `Omni`.
pub enum OmniLanguage {
    #[cfg(feature = "bash")]
    Bash(BashLanguage),
    #[cfg(feature = "html")]
    Html(HtmlLanguage),
    #[cfg(feature = "python")]
    Python(PythonLanguage),
    #[cfg(feature = "rust")]
    Rust(RustLanguage),
    #[cfg(feature = "text")]
    Text(TextLanguage),
    #[cfg(feature = "wgsl")]
    Wgsl(WgslLanguage),
    #[cfg(feature = "zsh")]
    Zsh(ZshLanguage),
}

impl Language for OmniLanguage {
    type Post = OmniLanguagePost;

    #[allow(unused_variables)]
    fn parse_pre(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post> {
        match self {
            #[cfg(feature = "bash")]
            OmniLanguage::Bash(lang) => lang.parse_pre(ikind, code).map(OmniLanguagePost::Bash),
            #[cfg(feature = "html")]
            OmniLanguage::Html(lang) => lang.parse_pre(ikind, code).map(OmniLanguagePost::Html),
            #[cfg(feature = "python")]
            OmniLanguage::Python(lang) => lang.parse_pre(ikind, code).map(OmniLanguagePost::Python),
            #[cfg(feature = "rust")]
            OmniLanguage::Rust(lang) => lang.parse_pre(ikind, code).map(OmniLanguagePost::Rust),
            #[cfg(feature = "text")]
            OmniLanguage::Text(lang) => lang.parse_pre(ikind, code).map(OmniLanguagePost::Text),
            #[cfg(feature = "wgsl")]
            OmniLanguage::Wgsl(lang) => lang.parse_pre(ikind, code).map(OmniLanguagePost::Wgsl),
            #[cfg(feature = "zsh")]
            OmniLanguage::Zsh(lang) => lang.parse_pre(ikind, code).map(OmniLanguagePost::Zsh),
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }

    fn hashbang(&self) -> Option<&'static str> {
        match self {
            #[cfg(feature = "bash")]
            OmniLanguage::Bash(lang) => lang.hashbang(),
            #[cfg(feature = "html")]
            OmniLanguage::Html(lang) => lang.hashbang(),
            #[cfg(feature = "python")]
            OmniLanguage::Python(lang) => lang.hashbang(),
            #[cfg(feature = "rust")]
            OmniLanguage::Rust(lang) => lang.hashbang(),
            #[cfg(feature = "text")]
            OmniLanguage::Text(lang) => lang.hashbang(),
            #[cfg(feature = "wgsl")]
            OmniLanguage::Wgsl(lang) => lang.hashbang(),
            #[cfg(feature = "zsh")]
            OmniLanguage::Zsh(lang) => lang.hashbang(),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    #[allow(unused_variables)]
    fn arity(&self, tag: &str) -> Arity {
        match self {
            #[cfg(feature = "bash")]
            OmniLanguage::Bash(lang) => lang.arity(tag),
            #[cfg(feature = "html")]
            OmniLanguage::Html(lang) => lang.arity(tag),
            #[cfg(feature = "python")]
            OmniLanguage::Python(lang) => lang.arity(tag),
            #[cfg(feature = "rust")]
            OmniLanguage::Rust(lang) => lang.arity(tag),
            #[cfg(feature = "text")]
            OmniLanguage::Text(lang) => lang.arity(tag),
            #[cfg(feature = "wgsl")]
            OmniLanguage::Wgsl(lang) => lang.arity(tag),
            #[cfg(feature = "zsh")]
            OmniLanguage::Zsh(lang) => lang.arity(tag),
            #[allow(unreachable_patterns)]
            _ => Default::default(),
        }
    }

    #[allow(unused_variables)]
    fn typ(&self, tag: &str) -> InnerKind {
        match self {
            #[cfg(feature = "bash")]
            OmniLanguage::Bash(lang) => lang.typ(tag),
            #[cfg(feature = "html")]
            OmniLanguage::Html(lang) => lang.typ(tag),
            #[cfg(feature = "python")]
            OmniLanguage::Python(lang) => lang.typ(tag),
            #[cfg(feature = "rust")]
            OmniLanguage::Rust(lang) => lang.typ(tag),
            #[cfg(feature = "text")]
            OmniLanguage::Text(lang) => lang.typ(tag),
            #[cfg(feature = "wgsl")]
            OmniLanguage::Wgsl(lang) => lang.typ(tag),
            #[cfg(feature = "zsh")]
            OmniLanguage::Zsh(lang) => lang.typ(tag),
            #[allow(unreachable_patterns)]
            _ => Default::default(),
        }
    }
}

/// A `LanguagePost` used by `Omni`.
#[derive(Debug)]
pub enum OmniLanguagePost {
    #[cfg(feature = "bash")]
    Bash(<BashLanguage as Language>::Post),
    #[cfg(feature = "html")]
    Html(<HtmlLanguage as Language>::Post),
    #[cfg(feature = "python")]
    Python(<PythonLanguage as Language>::Post),
    #[cfg(feature = "rust")]
    Rust(<RustLanguage as Language>::Post),
    #[cfg(feature = "text")]
    Text(<TextLanguage as Language>::Post),
    #[cfg(feature = "wgsl")]
    Wgsl(<WgslLanguage as Language>::Post),
    #[cfg(feature = "zsh")]
    Zsh(<ZshLanguage as Language>::Post),
}

impl LanguagePost for OmniLanguagePost {
    fn holes(&self) -> &[Hole] {
        match self {
            #[cfg(feature = "bash")]
            OmniLanguagePost::Bash(post) => post.holes(),
            #[cfg(feature = "html")]
            OmniLanguagePost::Html(post) => post.holes(),
            #[cfg(feature = "python")]
            OmniLanguagePost::Python(post) => post.holes(),
            #[cfg(feature = "rust")]
            OmniLanguagePost::Rust(post) => post.holes(),
            #[cfg(feature = "text")]
            OmniLanguagePost::Text(post) => post.holes(),
            #[cfg(feature = "wgsl")]
            OmniLanguagePost::Wgsl(post) => post.holes(),
            #[cfg(feature = "zsh")]
            OmniLanguagePost::Zsh(post) => post.holes(),
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }

    #[allow(unused_variables)]
    fn parse_post(&self, plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>> {
        match self {
            #[cfg(feature = "bash")]
            OmniLanguagePost::Bash(post) => post.parse_post(plugs),
            #[cfg(feature = "html")]
            OmniLanguagePost::Html(post) => post.parse_post(plugs),
            #[cfg(feature = "python")]
            OmniLanguagePost::Python(post) => post.parse_post(plugs),
            #[cfg(feature = "rust")]
            OmniLanguagePost::Rust(post) => post.parse_post(plugs),
            #[cfg(feature = "text")]
            OmniLanguagePost::Text(post) => post.parse_post(plugs),
            #[cfg(feature = "wgsl")]
            OmniLanguagePost::Wgsl(post) => post.parse_post(plugs),
            #[cfg(feature = "zsh")]
            OmniLanguagePost::Zsh(post) => post.parse_post(plugs),
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }
}

/// A `MetaLanguage` used by `Omni`.
pub enum OmniMetaLanguage {
    #[cfg(feature = "python")]
    Python(PythonMetaLanguage),
    #[cfg(feature = "rust")]
    Rust(RustMetaLanguage),
}

impl OmniMetaLanguage {
    fn inner(&self) -> &dyn MetaLanguage {
        match self {
            #[cfg(feature = "python")]
            OmniMetaLanguage::Python(lang) => lang,
            #[cfg(feature = "rust")]
            OmniMetaLanguage::Rust(lang) => lang,
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }
}

impl MetaLanguage for OmniMetaLanguage {
    #[allow(unused_variables)]
    fn expand_quote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        match self {
            #[cfg(feature = "python")]
            OmniMetaLanguage::Python(lang) => lang.expand_quote(lang1, tag, i, lang2, qterm, cmds),
            #[cfg(feature = "rust")]
            OmniMetaLanguage::Rust(lang) => lang.expand_quote(lang1, tag, i, lang2, qterm, cmds),
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }

    #[allow(unused_variables)]
    fn expand_unquote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        match self {
            #[cfg(feature = "python")]
            OmniMetaLanguage::Python(lang) => {
                lang.expand_unquote(lang1, tag, i, lang2, qterm, cmds)
            }
            #[cfg(feature = "rust")]
            OmniMetaLanguage::Rust(lang) => lang.expand_unquote(lang1, tag, i, lang2, qterm, cmds),
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }

    #[allow(unused_variables)]
    fn expand_tuple(
        &self,
        lang1: &str,
        tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        arity: crate::lang::Arity,
    ) -> Result<Arc<QTerm>> {
        match self {
            #[cfg(feature = "python")]
            OmniMetaLanguage::Python(lang) => lang.expand_tuple(lang1, tag, qterms, cmds, arity),
            #[cfg(feature = "rust")]
            OmniMetaLanguage::Rust(lang) => lang.expand_tuple(lang1, tag, qterms, cmds, arity),
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }

    fn wrap_child(&self, qterm: Arc<QTerm>, okind: OuterKind) -> Result<Arc<QTerm>> {
        self.inner().wrap_child(qterm, okind)
    }

    fn lift_str(&self, target: &str) -> Result<&'static str> {
        self.inner().lift_str(target)
    }

    fn reduce_str(&self) -> &'static str {
        self.inner().reduce_str()
    }

    fn emit_str(&self) -> &'static str {
        self.inner().emit_str()
    }

    fn type_str(&self) -> &'static str {
        self.inner().type_str()
    }

    fn name_str(&self) -> &'static str {
        self.inner().name_str()
    }
}

pub type Omni = Multi<OmniLanguages, OmniMetaLanguages>;

/**************************************************************/

/// A `MultiLanguage` using `Box<dyn>`s.
#[derive(Default)]
pub struct DynOmniLanguages {
    #[cfg(feature = "bash")]
    bash: DynBashLanguage,
    #[cfg(feature = "html")]
    html: DynHtmlLanguage,
    #[cfg(feature = "python")]
    py: DynPythonLanguage,
    #[cfg(feature = "rust")]
    rs: DynRustLanguage,
    #[cfg(feature = "text")]
    txt: DynTextLanguage,
    #[cfg(feature = "wgsl")]
    wgsl: DynWgslLanguage,
    #[cfg(feature = "zsh")]
    zsh: DynZshLanguage,
}

impl Languages for DynOmniLanguages {
    type Language = dyn Language<Post = Box<dyn LanguagePost>>;

    fn get(&self, lang: &str) -> Result<&Self::Language> {
        match lang {
            #[cfg(feature = "bash")]
            "bash" => Ok(&self.bash),
            #[cfg(feature = "html")]
            "html" => Ok(&self.html),
            #[cfg(feature = "python")]
            "python" | "py" => Ok(&self.py),
            #[cfg(feature = "rust")]
            "rust" | "rs" => Ok(&self.rs),
            #[cfg(feature = "text")]
            "text" | "txt" => Ok(&self.txt),
            #[cfg(feature = "wgsl")]
            "wgsl" => Ok(&self.wgsl),
            #[cfg(feature = "zsh")]
            "zsh" => Ok(&self.zsh),
            _ => Err(miette!("{lang:?} can't be used as Language")),
        }
    }
    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::Language> {
        match lang {
            #[cfg(feature = "bash")]
            "bash" => Ok(&mut self.bash),
            #[cfg(feature = "html")]
            "html" => Ok(&mut self.html),
            #[cfg(feature = "python")]
            "python" | "py" => Ok(&mut self.py),
            #[cfg(feature = "rust")]
            "rust" | "rs" => Ok(&mut self.rs),
            #[cfg(feature = "text")]
            "text" | "txt" => Ok(&mut self.txt),
            #[cfg(feature = "wgsl")]
            "wgsl" => Ok(&mut self.wgsl),
            #[cfg(feature = "zsh")]
            "zsh" => Ok(&mut self.zsh),
            _ => Err(miette!("{lang:?} can't be used as Language")),
        }
    }
}

/// A `MultiLanguage` using `Box<dyn>`s.
#[derive(Default)]
pub struct DynOmniMetaLanguages {
    #[cfg(feature = "python")]
    py: PythonMetaLanguage,
    #[cfg(feature = "rust")]
    rs: RustMetaLanguage,
}

impl MetaLanguages for DynOmniMetaLanguages {
    type MetaLanguage = dyn MetaLanguage;

    fn get(&self, lang: &str) -> Result<&Self::MetaLanguage> {
        match lang {
            #[cfg(feature = "python")]
            "python" | "py" => Ok(&self.py),
            #[cfg(feature = "rust")]
            "rust" | "rs" => Ok(&self.rs),
            _ => Err(miette!("{lang:?} can't be used as MetaLanguage")),
        }
    }

    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::MetaLanguage> {
        match lang {
            #[cfg(feature = "python")]
            "python" | "py" => Ok(&mut self.py),
            #[cfg(feature = "rust")]
            "rust" | "rs" => Ok(&mut self.rs),
            _ => Err(miette!("{lang:?} can't be used as MetaLanguage")),
        }
    }
}

/**************************************************************/

#[allow(clippy::default_constructed_unit_structs)]
pub fn dict_omni_language() -> DictMulti {
    #[allow(unused_mut)]
    let mut ret = DictMulti::default();
    #[cfg(feature = "python")]
    {
        ret.add_lang("python", bx(DynPythonLanguage::default()));
        ret.add_lang("py", bx(DynPythonLanguage::default()));
        ret.add_meta("python", bx(PythonMetaLanguage::default()));
        ret.add_meta("py", bx(PythonMetaLanguage::default()));
    }
    #[cfg(feature = "rust")]
    {
        ret.add_lang("rust", bx(DynRustLanguage::default()));
        ret.add_lang("rs", bx(DynRustLanguage::default()));
        ret.add_meta("rust", bx(RustMetaLanguage::default()));
        ret.add_meta("rs", bx(RustMetaLanguage::default()));
    }
    #[cfg(feature = "text")]
    {
        ret.add_lang("text", bx(DynTextLanguage::default()));
        ret.add_lang("txt", bx(DynTextLanguage::default()));
    }
    #[cfg(feature = "wgsl")]
    {
        // WGSL is a target language only — the host's MetaLanguage drives
        // expansion, so no `add_meta`.
        ret.add_lang("wgsl", bx(DynWgslLanguage::default()));
    }
    #[cfg(feature = "html")]
    {
        // HTML is a target language only — the host's MetaLanguage drives
        // expansion, so no `add_meta`.
        ret.add_lang("html", bx(DynHtmlLanguage::default()));
    }
    #[cfg(feature = "zsh")]
    {
        // Zsh is a target language only — the host's MetaLanguage drives
        // expansion, so no `add_meta`.
        ret.add_lang("zsh", bx(DynZshLanguage::default()));
    }
    #[cfg(feature = "bash")]
    {
        // Bash is a target language only — the host's MetaLanguage drives
        // expansion, so no `add_meta`.
        ret.add_lang("bash", bx(DynBashLanguage::default()));
    }

    ret
}
