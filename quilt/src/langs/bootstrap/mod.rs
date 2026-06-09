use crate::multi::Single;

pub mod lang;
pub mod meta;
pub mod strlift;

pub type Bootstrap = Single<lang::BootstrapLanguage, meta::BootstrapMetaLanguage>;
