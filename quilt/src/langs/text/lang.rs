use crate::lang::{FlatNode, Hole, InnerKind, Language, LanguagePost};
use crate::prelude::*;
use crate::qterm::QTerm;

/**************************************************************/

#[derive(Default)]
pub struct TextLanguage;

impl Language for TextLanguage {
    type Post = TextLanguagePost;

    fn parse_pre(&mut self, _ikind: Option<InnerKind>, _code: &[FlatNode]) -> Result<Self::Post> {
        todo!()
    }
}

#[derive(Debug)]
pub struct TextLanguagePost;

impl LanguagePost for TextLanguagePost {
    fn holes(&self) -> &[Hole] {
        todo!()
    }

    fn parse_post(&self, _plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>> {
        todo!()
    }
}

/**************************************************************/

#[derive(Default)]
pub struct DynTextLanguage;

impl Language for DynTextLanguage {
    type Post = Box<dyn LanguagePost>;

    fn parse_pre(&mut self, _ikind: Option<InnerKind>, _code: &[FlatNode]) -> Result<Self::Post> {
        todo!()
    }
}
