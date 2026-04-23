use crate::TermId;
use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ModelError {
    #[error("invalid IRI: {0}")]
    InvalidIri(String),
    #[error("invalid blank node label: {0}")]
    InvalidBlankNodeLabel(String),
    #[error("invalid language tag: {0}")]
    InvalidLanguageTag(String),
    #[error("rdf:langString requires a language tag")]
    MissingLanguageTag,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum TermIdError {
    #[error("dictionary id {0} is out of range")]
    InvalidDictionaryId(u64),
    #[error("value {0} is out of range for inline i60")]
    IntegerOutOfRange(i64),
    #[error("short string length {0} exceeds 7 bytes")]
    ShortStringTooLong(usize),
    #[error("invalid UTF-8 bytes in inline short string")]
    InvalidUtf8ShortString,
    #[error("term id is not inlined: {0}")]
    NotInline(u64),
    #[error("unsupported inline tag: {0:#x}")]
    UnsupportedInlineTag(u8),
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum DictionaryError {
    #[error("dictionary exhausted term id space")]
    TermIdSpaceExhausted,
    #[error("no term found for id {0:?}")]
    UnknownTermId(TermId),
    #[error(transparent)]
    TermId(#[from] TermIdError),
}
