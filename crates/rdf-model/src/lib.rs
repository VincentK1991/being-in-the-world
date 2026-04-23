//! Core RDF data model types, dictionary, and term identifiers.

#![forbid(unsafe_code)]

mod dictionary;
mod error;
mod model;
mod term_id;

pub use dictionary::{Dictionary, InMemoryDictionary};
pub use error::{DictionaryError, ModelError, TermIdError};
#[cfg(feature = "rdf-star")]
pub use model::QuotedTriple;
pub use model::{
    BlankNode, GraphName, Literal, NamedNode, Object, Predicate, Quad, RDF_LANG_STRING_IRI,
    Subject, Term, Triple, XSD_BOOLEAN_IRI, XSD_INTEGER_IRI, XSD_STRING_IRI,
};
pub use term_id::{InlineValue, TermId};
