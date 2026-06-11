use crate::lang::{Arity, InnerKind, Language, LanguagePost};
#[cfg(feature = "parse")]
use crate::lang::{FlatNode, Hole};
use crate::meta::{MetaLanguage, OuterKind};
#[cfg(feature = "parse")]
use crate::node::Node;
use crate::prelude::*;
#[cfg(feature = "parse")]
use crate::qterm::QTermBuilder;
use crate::qterm::{tuple, QTerm};
use crate::term::CmdOrHole;
#[cfg(feature = "parse")]
use crate::zipper::{List, Zipper};
use miette::{bail, ensure, miette};
#[cfg(feature = "parse")]
use regex::Regex;
use std::collections::BTreeMap;
#[cfg(feature = "parse")]
use std::sync::LazyLock;

/**************************************************************/

#[cfg(feature = "parse")]
static RE_WS_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*").unwrap());
const DEFAULT_LANG: &str = "rs";

/**************************************************************/

/// The depth at which a term is being expanded. Either Ground or some language and positive depth.
#[derive(Debug, Clone, Default)]
pub enum Stage {
    #[default]
    Ground,
    /// invariant: Index > 0
    Sky(Box<str>, Index),
}

/// Seed a parse zipper from a language chain (host first, then the defaults for
/// nested un-annotated quotes). The host sits at the cursor (`head`) and the
/// rest seed `anti`, so an un-annotated `↖…↗` resolves the next default language
/// via [`Zipper::back`]; when the chain is exhausted, parsing falls back to the
/// host (matching the single-language `["rs"]` behavior).
#[cfg(feature = "parse")]
fn zipper_from_chain(chain: &[&str]) -> Zipper<Box<str>> {
    let host: Box<str> = chain.first().copied().unwrap_or(DEFAULT_LANG).into();
    let mut anti = List::new();
    for lang in chain.get(1..).unwrap_or(&[]).iter().rev() {
        anti = anti.cons((*lang).into());
    }
    Zipper {
        list: List::new().cons(host),
        anti,
    }
}

/**************************************************************/

pub trait Languages {
    type Language: Language + ?Sized;

    fn get(&self, lang: &str) -> Result<&Self::Language>;
    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::Language>;
}

pub trait MetaLanguages {
    type MetaLanguage: MetaLanguage + ?Sized;

    fn get(&self, lang: &str) -> Result<&Self::MetaLanguage>;
    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::MetaLanguage>;

    /**************************************************************/

    fn lift_str(&self, lang: &str, target: &str) -> Result<&'static str> {
        self.get(lang)?.lift_str(target)
    }
    fn reduce_str(&self, lang: &str) -> Result<&'static str> {
        Ok(self.get(lang)?.reduce_str())
    }
    fn emit_str(&self, lang: &str) -> Result<&'static str> {
        Ok(self.get(lang)?.emit_str())
    }
    fn type_str(&self, lang: &str) -> Result<&'static str> {
        Ok(self.get(lang)?.type_str())
    }
    fn name_str(&self, lang: &str) -> Result<&'static str> {
        Ok(self.get(lang)?.name_str())
    }
}

#[derive(Default)]
pub struct Multi<LS: Languages, MS: MetaLanguages> {
    pub langs: LS,
    pub metas: MS,
}

impl<LS: Languages, MS: MetaLanguages> Multi<LS, MS> {
    pub fn get_lang(&self, lang: &str) -> Result<&LS::Language> {
        self.langs.get(lang)
    }
    pub fn get_lang_mut(&mut self, lang: &str) -> Result<&mut LS::Language> {
        self.langs.get_mut(lang)
    }
    pub fn get_meta(&self, lang: &str) -> Result<&MS::MetaLanguage> {
        self.metas.get(lang)
    }
    pub fn get_meta_mut(&mut self, lang: &str) -> Result<&mut MS::MetaLanguage> {
        self.metas.get_mut(lang)
    }
    pub fn lift_str(&self, lang: &str, target: &str) -> Result<&'static str> {
        self.metas.lift_str(lang, target)
    }
    pub fn reduce_str(&self, lang: &str) -> Result<&'static str> {
        self.metas.reduce_str(lang)
    }
    pub fn emit_str(&self, lang: &str) -> Result<&'static str> {
        self.metas.emit_str(lang)
    }
    pub fn type_str(&self, lang: &str) -> Result<&'static str> {
        self.metas.type_str(lang)
    }
    pub fn name_str(&self, lang: &str) -> Result<&'static str> {
        self.metas.name_str(lang)
    }

    #[cfg(feature = "parse")]
    pub fn parse(&mut self, s: &str) -> Result<Arc<QTerm>> {
        self.parse_lang(DEFAULT_LANG, s)
    }

    #[cfg(feature = "parse")]
    pub fn parse_lang(&mut self, lang: &str, s: &str) -> Result<Arc<QTerm>> {
        self.parse_chain(&[lang], s)
    }

    /// Parse `s` whose language is given by a *chain* of file extensions: the
    /// host (ground) language first, then the default language for successively
    /// nested un-annotated `↖…↗` quotes. This comes from a file name like
    /// `shaders.wgsl.rs.quilt` (chain `["rs", "wgsl"]`): Rust is the host and a
    /// bare `↖…↗` defaults to WGSL. A single-element chain (`["rs"]`) is the
    /// plain `foo.rs.quilt` case where bare quotes default to the host.
    #[cfg(feature = "parse")]
    pub fn parse_chain(&mut self, chain: &[&str], s: &str) -> Result<Arc<QTerm>> {
        let nodes = Node::parse(s)
            .iter()
            .map(|n| n.clone().into())
            .collect::<Vec<_>>();

        Ok(self
            .build_nodes(
                tb(""),
                &Hole::default(),
                &nodes,
                &zipper_from_chain(chain),
                None,
                false,
            )?
            .first()
            .unwrap())
    }

    /// Build a `QTerm` from quilt `Node`s. `splice_target` is the language the
    /// fragment's value will be spliced into: `Some` only for unquote bodies
    /// (the lang of the enclosing quote), `None` otherwise. It directs `↑` —
    /// a lift inside `wgsl↖ … ↙x.↑↘ … ↗` targets WGSL; elsewhere a lift is
    /// homogeneous (targets the fragment's own language). `bracketed` is true
    /// when `nodes` is the body of a `↖…↗`/`↙…↘` pair and false at the top
    /// level: only bracketed bodies get their boundary newlines trimmed.
    #[cfg(feature = "parse")]
    pub fn build_nodes(
        &mut self,
        mut builder: QTermBuilder,
        hole: &Hole,
        nodes: &[Arc<Node>],
        zipper: &Zipper<Box<str>>,
        splice_target: Option<&str>,
        bracketed: bool,
    ) -> Result<QTermBuilder> {
        let lang = zipper.head().unwrap();

        // dbg!(&nodes);

        // Pass 1: dedent nodes and find common prefix
        let mut nodes_new = Vec::with_capacity(nodes.len());
        let mut indent = None;
        let mut on_nl = false;
        let mut num_nl = 0;
        for n in nodes {
            match &**n {
                Node::Content(s) if on_nl && !hole.prefix.is_empty() => {
                    let mut s = &**s;
                    for p in &hole.prefix {
                        s = s
                            .strip_prefix(&**p)
                            .ok_or_else(|| miette!("Failed to strip prefix"))?;
                    }
                    if !s.is_empty() {
                        nodes_new.push(arc(Node::Content(s.into())));
                        indent = match indent {
                            None => {
                                // extract prefix
                                let m = RE_WS_PREFIX.find(s).unwrap();
                                Some(m.as_str())
                            }
                            Some(pre) => {
                                // find common prefix
                                let c = pre
                                    .chars()
                                    .zip(s.chars())
                                    .take_while(|(x, y)| x == y)
                                    .count();
                                Some(&pre[..c])
                            }
                        }
                    }
                }
                _ => nodes_new.push(n.clone()),
            }
            on_nl = **n == Node::NewLine;
            num_nl += usize::from(on_nl);
        }
        let mut nodes = nodes_new;
        let indent = indent.unwrap_or_default();

        // dbg!(&nodes_new);

        // Pass 2: dedent by common prefix
        if !indent.is_empty() {
            let mut nodes_new = Vec::with_capacity(nodes.len());
            for n in nodes {
                match &*n {
                    Node::Content(s) if on_nl => {
                        let s = s.strip_prefix(indent).unwrap();
                        if !s.is_empty() {
                            nodes_new.push(arc(Node::Content(s.into())));
                        }
                    }
                    _ => nodes_new.push(n.clone()),
                }
                on_nl = *n == Node::NewLine;
            }
            nodes = nodes_new;
        }
        // Trim one boundary newline on each side so `↖\n…\n↗` parses the same
        // body as `↖…↗`; the trimmed newlines are re-emitted as builder cmds
        // below. At the top level (no enclosing brackets) the boundary
        // newlines are real source content and must stay in `code`.
        let first_nl = bracketed && !nodes.is_empty() && **nodes.first().unwrap() == Node::NewLine;
        let last_nl = bracketed
            && !nodes.is_empty()
            && **nodes.last().unwrap() == Node::NewLine
            && num_nl != usize::from(first_nl); // distinguish ↖↗ from ↖\n↗
        let nodes = &nodes[usize::from(first_nl)..nodes.len() - usize::from(last_nl)];

        // Pass 3: build up a string of code with holes // TODO: avoid creating string
        let mut code = Vec::with_capacity(nodes.len());
        for n in nodes {
            match &**n {
                Node::Content(s) => code.push(FlatNode::Str(s)),
                Node::NewLine => code.push(FlatNode::NewLine),
                Node::Quote { .. } | Node::Unquote { .. } => code.push(FlatNode::Hole),
                Node::Lift => code.push(FlatNode::Str(
                    self.lift_str(lang, splice_target.unwrap_or(lang))?,
                )),
                Node::Reduce => code.push(FlatNode::Str(self.reduce_str(lang)?)),
                Node::Emit => code.push(FlatNode::Str(self.emit_str(lang)?)),
                Node::Type => code.push(FlatNode::Str(self.type_str(lang)?)),
                Node::Name => code.push(FlatNode::Str(self.name_str(lang)?)),
            }
        }

        // println!("PRE: code: '{code}'");

        // dbg!(&code);
        // dbg!(&indent);
        // dbg!(&first_nl);
        // dbg!(&last_nl);

        // parse this level and get hole types of children
        let post = self.get_lang_mut(lang)?.parse_pre(hole.ikind, &code)?;
        let mut holes = post.holes().iter();

        // parse children using hole types
        let mut plugs = Vec::new();
        for n in nodes {
            match &**n {
                Node::Content(_)
                | Node::NewLine
                | Node::Lift
                | Node::Reduce
                | Node::Emit
                | Node::Type
                | Node::Name => {}
                Node::Quote { anno, nodes } => {
                    let hole = holes
                        .next()
                        .ok_or_else(|| miette!("Ran out of holes for quote: {n:?}"))?;
                    // .unwrap_or_else(|| miette!("Ran out of holes for quote: {n:?}"))?;
                    // Descend into the quote's embedded language. An annotation
                    // selects it explicitly; an un-annotated `↖…↗` resolves the
                    // default embedded language via `.back()`, which walks the
                    // file's extension chain (e.g. `wgsl` for `*.wgsl.rs.quilt`)
                    // or a previously-used lang, falling back to the host when
                    // none is set. The quote's lang is then the head of the
                    // resolved zipper — the expander uses it to pick the arity
                    // for the body's tags (e.g. `wgsl`'s `case_compound_statement`).
                    let zipper = if anno.is_empty() {
                        zipper
                            .clone()
                            .back()
                            .unwrap_or_else(|| zipper.clone().cons(lang.clone()))
                    } else {
                        zipper.clone().cons(anno.clone())
                    };
                    let quote_lang = zipper.head().unwrap();
                    let mut builder = qb(&hole.otag, 1, quote_lang);
                    builder.write(anno).write("↖");
                    // A quote's body is free-form (a statement-shaped term may
                    // fill an expression hole and vice versa), so don't coerce
                    // it to the hole's kind.
                    let hole = Hole {
                        ikind: None,
                        ..hole.clone()
                    };
                    let mut builder =
                        self.build_nodes(builder, &hole, nodes, &zipper, None, true)?;
                    builder.write("↗");
                    plugs.push(builder.b());
                }
                Node::Unquote { anno, nodes } => {
                    let hole = hole
                        + holes
                            .next()
                            .ok_or_else(|| miette!("Ran out of holes for unquote: {n:?}"))?;
                    let mut builder = ub(&hole.otag, 1, lang);
                    // The body's value is spliced into *this* fragment's
                    // language: it directs any `↑` in the body (e.g. a lift
                    // inside `wgsl↖…↗` lifts into WGSL).
                    let splice_target = lang;
                    let zipper = zipper.clone().tail().unwrap();
                    builder.write(anno).write("↙");
                    let mut builder = self.build_nodes(
                        builder,
                        &hole,
                        nodes,
                        &zipper,
                        Some(splice_target),
                        true,
                    )?;
                    builder.write("↘");
                    plugs.push(builder.b());
                }
            }
        }

        // fill holes with plugs
        let term = post.parse_post(&plugs)?;

        // builder result
        builder
            .push_if(indent, !indent.is_empty())
            .nl_if(first_nl)
            .child(&term)
            .pop_if(!indent.is_empty())
            .nl_if(last_nl);
        Ok(builder)
    }

    pub fn expand(&mut self, qterm: &QTerm) -> Result<Arc<QTerm>> {
        self.expand_lang(DEFAULT_LANG, qterm)
    }

    pub fn expand_lang(&mut self, lang: &str, qterm: &QTerm) -> Result<Arc<QTerm>> {
        let Multi {
            ref mut langs,
            ref mut metas,
        } = self;
        let meta = metas.get(lang)?;
        Expander {
            langs,
            meta,
            lang,
            pattern_vars: None,
        }
        .expand(&Default::default(), qterm)
    }
}

pub struct Expander<'a, LS: Languages, M: MetaLanguage + ?Sized> {
    langs: &'a mut LS,
    meta: &'a M,
    /// The ground (host) language, used to classify tags of ground tuples.
    lang: &'a str,
    /// `Some` while expanding the pattern quote of a pattern-let: ground
    /// unquotes become metavariables, collected here in source order.
    pattern_vars: Option<Vec<Box<str>>>,
}

impl<M: MetaLanguage + ?Sized, LS: Languages> Expander<'_, LS, M> {
    // TODO: simplify a ton, remove unused concepts
    fn expand(&mut self, depth: &Stage, term: &QTerm) -> Result<Arc<QTerm>> {
        match depth {
            Stage::Ground => match term {
                QTerm::Quote {
                    index,
                    lang: lang1,
                    term,
                    ..
                } => {
                    self.expand(&Stage::Sky(lang1.clone(), *index), term)
                    // self.get_meta(meta).wrap_quote(meta, expanded, emit)
                }
                QTerm::Unquote { .. } => bail!("unquote depth too high!"),
                QTerm::Tuple { tag, terms, cmds } => {
                    // `let ↖pattern↗ = value;` — a quote in binding position
                    // destructures the value instead of building a term.
                    if self.meta.pattern_tag() == Some(&**tag) {
                        if let Some(QTerm::Quote { .. }) = terms.get(1).map(AsRef::as_ref) {
                            return self.expand_pattern_let(tag, terms, cmds);
                        }
                    }
                    let arity = self.langs.get(self.lang)?.arity(tag);
                    let terms = terms
                        .iter()
                        .map(|term| {
                            // expand each child
                            let expanded = self.expand(&Stage::Ground, term)?;
                            // then wrap the result
                            let mut okind = OuterKind::None;
                            // A quote in statement position of a variadic node
                            // would otherwise build a term and silently drop
                            // it, so infer Emit. A tail-expression quote parses
                            // with the same outer tag (`expression_statement`),
                            // so also require the quoted body to be a statement
                            // — an expression body means the quote is a value.
                            if arity == Arity::Variadic {
                                if let QTerm::Quote {
                                    tag: qtag,
                                    lang: lang2,
                                    term: body,
                                    ..
                                } = &**term
                                {
                                    if self.langs.get(self.lang)?.typ(qtag) == InnerKind::Stmt {
                                        if let QTerm::Tuple { tag: btag, .. } = &**body {
                                            if self.langs.get(lang2)?.typ(btag) == InnerKind::Stmt {
                                                okind = OuterKind::Emit;
                                            }
                                        }
                                    }
                                }
                            }
                            self.meta.wrap_child(expanded, okind)
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(tuple(tag, &terms, cmds))
                } // QTerm::Lift(_gap) => todo!(),
                  // QTerm::Reduce(_gap) => todo!(),
            },
            Stage::Sky(lang1, d) => match term {
                QTerm::Quote {
                    tag,
                    index,
                    lang: lang2,
                    term,
                    cmds,
                } => {
                    let term = self.expand(&Stage::Sky(lang1.clone(), d + index), term)?;
                    self.meta
                        .expand_quote(lang1, tag, *index, lang2, &term, cmds)
                }
                QTerm::Unquote {
                    tag,
                    index,
                    lang: lang2,
                    term,
                    cmds,
                } => {
                    ensure!(index <= d, "unquote depth too high!");
                    let new_depth = d - index;

                    if new_depth == 0 {
                        // Inside a pattern quote a ground unquote is a
                        // metavariable binder, not a splice: record its name
                        // and splice an `mvar` marker (see `crate::qmatch`).
                        if let Some(vars) = &mut self.pattern_vars {
                            let name = pattern_var_name(term)?;
                            vars.push(name.clone());
                            return self.meta.pattern_var(&name);
                        }
                        self.expand(&Stage::Ground, term)
                    } else {
                        let qterm = self.expand(&Stage::Sky(lang1.clone(), new_depth), term)?;
                        self.meta
                            .expand_unquote(lang1, tag, *index, lang2, &qterm, cmds)
                    }
                }
                QTerm::Tuple { tag, terms, cmds } => {
                    let arity = self.langs.get(lang1)?.arity(tag);
                    let expanded = terms
                        .iter()
                        .map(|term| {
                            // expand each child
                            let expanded = self.expand(depth, term)?;
                            // then wrap the result
                            let mut okind = Default::default();
                            if arity == Arity::Variadic {
                                okind = OuterKind::Emit;
                                if let QTerm::Unquote {
                                    index, term: child, ..
                                } = &**term
                                {
                                    if index == d {
                                        if let QTerm::Tuple { tag, .. } = &**child {
                                            if self.langs.get(lang1)?.typ(tag) == InnerKind::Stmt {
                                                okind = OuterKind::Splice;
                                            }
                                        }
                                    }
                                }
                            }
                            self.meta.wrap_child(expanded, okind)
                        })
                        .collect::<Result<Vec<_>>>()?;
                    self.meta.expand_tuple(lang1, tag, &expanded, cmds, arity)
                } // QTerm::Lift(gap) => self.expand_lift(lang1, gap),
                  // QTerm::Reduce(gap) => self.expand_run(lang1, gap),
            },
        }
    }

    /// Expand `let ↖pattern↗ = value;` (issue #18). The pattern quote is
    /// expanded with its ground unquotes replaced by `mvar` markers (their
    /// names collected via `pattern_vars`), and the statement is rewritten to
    /// destructure the result of matching the pattern against the value:
    /// `let [a, b] = qmatch_n(&<pattern>, &<value>);` (see `crate::qmatch`).
    fn expand_pattern_let(
        &mut self,
        tag: &str,
        terms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        // the value is the expression after `=`
        let eq = terms
            .iter()
            .position(
                |t| matches!(&**t, QTerm::Tuple { tag, terms, .. } if &**tag == "=" && terms.is_empty()),
            )
            .ok_or_else(|| miette!("pattern let without `= value`"))?;
        let val = eq + 1;
        ensure!(val < terms.len(), "pattern let without `= value`");

        // expand the pattern quote with ground unquotes as metavariables
        ensure!(
            self.pattern_vars.is_none(),
            "pattern let nested inside another pattern"
        );
        self.pattern_vars = Some(Vec::new());
        let pattern = self.expand(&Stage::Ground, &terms[1]);
        let names = self.pattern_vars.take().unwrap();
        let pattern = pattern?;
        for (i, name) in names.iter().enumerate() {
            ensure!(
                !names[..i].contains(name),
                "pattern let binds metavariable `{name}` more than once"
            );
        }

        let value = self.expand(&Stage::Ground, &terms[val])?;
        let (binder, call) = self.meta.pattern_let(&names, &pattern, &value)?;

        let terms = terms
            .iter()
            .enumerate()
            .map(|(i, term)| match i {
                1 => Ok(binder.clone()),
                _ if i == val => Ok(call.clone()),
                _ => self.expand(&Stage::Ground, term),
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(tuple(tag, &terms, cmds))
    }
}

/// The metavariable name of a ground unquote inside a pattern quote: its body
/// must be a plain identifier.
fn pattern_var_name(term: &QTerm) -> Result<Box<str>> {
    let text = term.coparse();
    let name = text.trim();
    let mut chars = name.chars();
    let ident = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_');
    ensure!(
        ident,
        "pattern metavariable must be a plain identifier, got {name:?}"
    );
    Ok(name.into())
}

/**************************************************************/

/// Resolve `lang` through an alias map; non-aliases pass through unchanged.
fn canonical<'a>(aliases: &'a BTreeMap<Box<str>, Box<str>>, lang: &'a str) -> &'a str {
    aliases.get(lang).map_or(lang, AsRef::as_ref)
}

#[derive(Default)]
pub struct DictLanguages {
    langs: BTreeMap<Box<str>, Box<dyn Language<Post = Box<dyn LanguagePost>>>>,
    /// alias → canonical key in `langs`, so aliases share one instance
    aliases: BTreeMap<Box<str>, Box<str>>,
}

impl Languages for DictLanguages {
    type Language = Box<dyn Language<Post = Box<dyn LanguagePost>>>;

    fn get(&self, lang: &str) -> Result<&Self::Language> {
        self.langs
            .get(canonical(&self.aliases, lang))
            .ok_or_else(|| miette!("Language {lang} not found"))
    }
    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::Language> {
        self.langs
            .get_mut(canonical(&self.aliases, lang))
            .ok_or_else(|| miette!("Language {lang} not found"))
    }
}

impl DictLanguages {
    pub fn add(&mut self, lang: &str, language: <Self as Languages>::Language) {
        self.langs.insert(lang.into(), language);
    }

    pub fn add_alias(&mut self, alias: &str, canonical: &str) {
        self.aliases.insert(alias.into(), canonical.into());
    }
}

#[derive(Default)]
pub struct DictMetaLanguages {
    metas: BTreeMap<Box<str>, Box<dyn MetaLanguage>>,
    /// alias → canonical key in `metas`, so aliases share one instance
    aliases: BTreeMap<Box<str>, Box<str>>,
}

impl MetaLanguages for DictMetaLanguages {
    type MetaLanguage = Box<dyn MetaLanguage>;

    fn get(&self, lang: &str) -> Result<&Self::MetaLanguage> {
        self.metas
            .get(canonical(&self.aliases, lang))
            .ok_or_else(|| miette!("Language {lang} not found"))
    }
    fn get_mut(&mut self, lang: &str) -> Result<&mut Self::MetaLanguage> {
        self.metas
            .get_mut(canonical(&self.aliases, lang))
            .ok_or_else(|| miette!("Language {lang} not found"))
    }
}

impl DictMetaLanguages {
    pub fn add(&mut self, lang: &str, meta_language: <Self as MetaLanguages>::MetaLanguage) {
        self.metas.insert(lang.into(), meta_language);
    }

    pub fn add_alias(&mut self, alias: &str, canonical: &str) {
        self.aliases.insert(alias.into(), canonical.into());
    }
}

pub type DictMulti = Multi<DictLanguages, DictMetaLanguages>;

impl DictMulti {
    pub fn add_lang(&mut self, lang: &str, language: <DictLanguages as Languages>::Language) {
        self.langs.add(lang, language);
    }

    pub fn add_meta(
        &mut self,
        lang: &str,
        meta: <DictMetaLanguages as MetaLanguages>::MetaLanguage,
    ) {
        self.metas.add(lang, meta);
    }

    /// Register `alias` as an alternate name for `canonical` in both the
    /// language and meta-language registries. Lookups under the alias resolve
    /// to the canonical entry's instance (no entry there → "not found").
    pub fn add_alias(&mut self, alias: &str, canonical: &str) {
        self.langs.add_alias(alias, canonical);
        self.metas.add_alias(alias, canonical);
    }
}

/**************************************************************/

// Singletons

#[derive(Default)]
pub struct Singleton<T>(T);

impl<L: Language> Languages for Singleton<L> {
    type Language = L;

    fn get(&self, _lang: &str) -> Result<&Self::Language> {
        Ok(&self.0)
    }
    fn get_mut(&mut self, _lang: &str) -> Result<&mut Self::Language> {
        Ok(&mut self.0)
    }
}

impl<M: MetaLanguage> MetaLanguages for Singleton<M> {
    type MetaLanguage = M;

    fn get(&self, _lang: &str) -> Result<&Self::MetaLanguage> {
        Ok(&self.0)
    }
    fn get_mut(&mut self, _lang: &str) -> Result<&mut Self::MetaLanguage> {
        Ok(&mut self.0)
    }
}

pub type Single<M, L> = Multi<Singleton<M>, Singleton<L>>;
