use crate::ModelError;
use oxiri::Iri;
use std::fmt::{self, Display, Formatter};

pub const XSD_STRING_IRI: &str = "http://www.w3.org/2001/XMLSchema#string";
pub const XSD_BOOLEAN_IRI: &str = "http://www.w3.org/2001/XMLSchema#boolean";
pub const XSD_INTEGER_IRI: &str = "http://www.w3.org/2001/XMLSchema#integer";
pub const RDF_LANG_STRING_IRI: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NamedNode {
    iri: String,
}

impl NamedNode {
    pub fn new(iri: impl Into<String>) -> Result<Self, ModelError> {
        let iri = iri.into();
        Iri::parse(iri.as_str()).map_err(|_| ModelError::InvalidIri(iri.clone()))?;
        Ok(Self { iri })
    }

    pub fn as_str(&self) -> &str {
        &self.iri
    }

    pub(crate) fn from_static(iri: &'static str) -> Self {
        Self { iri: String::from(iri) }
    }
}

impl Display for NamedNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.iri)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BlankNode {
    label: String,
}

impl BlankNode {
    pub fn new(label: impl Into<String>) -> Result<Self, ModelError> {
        let label = label.into();
        if !is_valid_blank_node_label(&label) {
            return Err(ModelError::InvalidBlankNodeLabel(label));
        }
        Ok(Self { label })
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

impl Display for BlankNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "_:{}", self.label)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Literal {
    lexical_form: String,
    datatype: NamedNode,
    language: Option<String>,
}

impl Literal {
    pub fn new_simple(lexical_form: impl Into<String>) -> Self {
        Self { lexical_form: lexical_form.into(), datatype: xsd_string_datatype(), language: None }
    }

    pub fn new_typed(
        lexical_form: impl Into<String>,
        datatype: NamedNode,
    ) -> Result<Self, ModelError> {
        if datatype.as_str() == RDF_LANG_STRING_IRI {
            return Err(ModelError::MissingLanguageTag);
        }
        Ok(Self { lexical_form: lexical_form.into(), datatype, language: None })
    }

    pub(crate) fn new_typed_unchecked(
        lexical_form: impl Into<String>,
        datatype: NamedNode,
    ) -> Self {
        Self { lexical_form: lexical_form.into(), datatype, language: None }
    }

    pub fn new_language_tagged(
        lexical_form: impl Into<String>,
        language: impl AsRef<str>,
    ) -> Result<Self, ModelError> {
        let language = normalize_language_tag(language.as_ref())?;
        Ok(Self {
            lexical_form: lexical_form.into(),
            datatype: rdf_lang_string_datatype(),
            language: Some(language),
        })
    }

    pub fn lexical_form(&self) -> &str {
        &self.lexical_form
    }

    pub fn datatype(&self) -> &NamedNode {
        &self.datatype
    }

    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }
}

impl Display for Literal {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if let Some(lang) = &self.language {
            write!(f, "\"{}\"@{}", self.lexical_form, lang)
        } else {
            write!(f, "\"{}\"^^{}", self.lexical_form, self.datatype)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Predicate {
    named_node: NamedNode,
}

impl Predicate {
    pub fn new(named_node: NamedNode) -> Self {
        Self { named_node }
    }

    pub fn as_named_node(&self) -> &NamedNode {
        &self.named_node
    }
}

impl Display for Predicate {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.named_node, f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    NamedNode(NamedNode),
    BlankNode(BlankNode),
    #[cfg(feature = "rdf-star")]
    QuotedTriple(Box<QuotedTriple>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Object {
    NamedNode(NamedNode),
    BlankNode(BlankNode),
    Literal(Literal),
    #[cfg(feature = "rdf-star")]
    QuotedTriple(Box<QuotedTriple>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum GraphName {
    DefaultGraph,
    NamedNode(NamedNode),
    BlankNode(BlankNode),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Triple {
    subject: Subject,
    predicate: Predicate,
    object: Object,
}

impl Triple {
    pub fn new(subject: Subject, predicate: Predicate, object: Object) -> Self {
        Self { subject, predicate, object }
    }

    pub fn subject(&self) -> &Subject {
        &self.subject
    }

    pub fn predicate(&self) -> &Predicate {
        &self.predicate
    }

    pub fn object(&self) -> &Object {
        &self.object
    }
}

#[cfg(feature = "rdf-star")]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QuotedTriple {
    subject: Subject,
    predicate: Predicate,
    object: Object,
}

#[cfg(feature = "rdf-star")]
impl QuotedTriple {
    pub fn new(subject: Subject, predicate: Predicate, object: Object) -> Self {
        Self { subject, predicate, object }
    }

    pub fn subject(&self) -> &Subject {
        &self.subject
    }

    pub fn predicate(&self) -> &Predicate {
        &self.predicate
    }

    pub fn object(&self) -> &Object {
        &self.object
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Quad {
    subject: Subject,
    predicate: Predicate,
    object: Object,
    graph_name: GraphName,
}

impl Quad {
    pub fn new(
        subject: Subject,
        predicate: Predicate,
        object: Object,
        graph_name: GraphName,
    ) -> Self {
        Self { subject, predicate, object, graph_name }
    }

    pub fn subject(&self) -> &Subject {
        &self.subject
    }

    pub fn predicate(&self) -> &Predicate {
        &self.predicate
    }

    pub fn object(&self) -> &Object {
        &self.object
    }

    pub fn graph_name(&self) -> &GraphName {
        &self.graph_name
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Term {
    NamedNode(NamedNode),
    BlankNode(BlankNode),
    Literal(Literal),
    #[cfg(feature = "rdf-star")]
    QuotedTriple(Box<QuotedTriple>),
}

impl From<NamedNode> for Predicate {
    fn from(value: NamedNode) -> Self {
        Self::new(value)
    }
}

impl From<NamedNode> for Subject {
    fn from(value: NamedNode) -> Self {
        Subject::NamedNode(value)
    }
}

impl From<BlankNode> for Subject {
    fn from(value: BlankNode) -> Self {
        Subject::BlankNode(value)
    }
}

impl From<NamedNode> for Object {
    fn from(value: NamedNode) -> Self {
        Object::NamedNode(value)
    }
}

impl From<BlankNode> for Object {
    fn from(value: BlankNode) -> Self {
        Object::BlankNode(value)
    }
}

impl From<Literal> for Object {
    fn from(value: Literal) -> Self {
        Object::Literal(value)
    }
}

impl From<NamedNode> for GraphName {
    fn from(value: NamedNode) -> Self {
        GraphName::NamedNode(value)
    }
}

impl From<BlankNode> for GraphName {
    fn from(value: BlankNode) -> Self {
        GraphName::BlankNode(value)
    }
}

impl From<NamedNode> for Term {
    fn from(value: NamedNode) -> Self {
        Term::NamedNode(value)
    }
}

impl From<BlankNode> for Term {
    fn from(value: BlankNode) -> Self {
        Term::BlankNode(value)
    }
}

impl From<Literal> for Term {
    fn from(value: Literal) -> Self {
        Term::Literal(value)
    }
}

#[cfg(feature = "rdf-star")]
impl From<QuotedTriple> for Subject {
    fn from(value: QuotedTriple) -> Self {
        Subject::QuotedTriple(Box::new(value))
    }
}

#[cfg(feature = "rdf-star")]
impl From<QuotedTriple> for Object {
    fn from(value: QuotedTriple) -> Self {
        Object::QuotedTriple(Box::new(value))
    }
}

#[cfg(feature = "rdf-star")]
impl From<QuotedTriple> for Term {
    fn from(value: QuotedTriple) -> Self {
        Term::QuotedTriple(Box::new(value))
    }
}

pub(crate) fn xsd_string_datatype() -> NamedNode {
    NamedNode::from_static(XSD_STRING_IRI)
}

pub(crate) fn xsd_boolean_datatype() -> NamedNode {
    NamedNode::from_static(XSD_BOOLEAN_IRI)
}

pub(crate) fn xsd_integer_datatype() -> NamedNode {
    NamedNode::from_static(XSD_INTEGER_IRI)
}

pub(crate) fn rdf_lang_string_datatype() -> NamedNode {
    NamedNode::from_static(RDF_LANG_STRING_IRI)
}

fn is_valid_blank_node_label(label: &str) -> bool {
    let mut chars = label.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }

    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn normalize_language_tag(tag: &str) -> Result<String, ModelError> {
    if tag.is_empty() {
        return Err(ModelError::InvalidLanguageTag(String::from(tag)));
    }

    let mut parts = tag.split('-');
    let Some(primary) = parts.next() else {
        return Err(ModelError::InvalidLanguageTag(String::from(tag)));
    };

    let primary_valid = !primary.is_empty()
        && primary.len() <= 8
        && primary.chars().all(|ch| ch.is_ascii_alphabetic());
    if !primary_valid {
        return Err(ModelError::InvalidLanguageTag(String::from(tag)));
    }

    let subtags_valid = parts.all(|part| {
        !part.is_empty() && part.len() <= 8 && part.chars().all(|ch| ch.is_ascii_alphanumeric())
    });
    if !subtags_valid {
        return Err(ModelError::InvalidLanguageTag(String::from(tag)));
    }

    Ok(tag.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::{
        BlankNode, GraphName, Literal, ModelError, NamedNode, Object, Predicate, Quad,
        RDF_LANG_STRING_IRI, Subject, Term, Triple, XSD_BOOLEAN_IRI,
    };

    #[test]
    fn named_node_validates_iri() {
        let node = NamedNode::new("https://example.com/resource").expect("valid IRI");
        assert_eq!(node.as_str(), "https://example.com/resource");
    }

    #[test]
    fn named_node_rejects_invalid_iri() {
        assert!(matches!(NamedNode::new("not a valid iri"), Err(ModelError::InvalidIri(_))));
    }

    #[test]
    fn blank_node_validates_label() {
        assert!(BlankNode::new("a_1.b-c").is_ok());
        assert!(BlankNode::new("1bad").is_err());
    }

    #[test]
    fn language_tagged_literal_sets_rdf_langstring_datatype() {
        let literal = Literal::new_language_tagged("hello", "EN-US").expect("valid language");
        assert_eq!(literal.language(), Some("en-us"));
        assert_eq!(literal.datatype().as_str(), RDF_LANG_STRING_IRI);
    }

    #[test]
    fn typed_literal_rejects_lang_string_without_tag() {
        let datatype = NamedNode::new(RDF_LANG_STRING_IRI).expect("valid datatype IRI");
        assert_eq!(Literal::new_typed("hello", datatype), Err(ModelError::MissingLanguageTag));
    }

    #[test]
    fn simple_literal_uses_xsd_string() {
        let literal = Literal::new_simple("hello");
        assert_eq!(literal.datatype().as_str(), super::XSD_STRING_IRI);
        assert_eq!(literal.language(), None);
    }

    #[test]
    fn triple_and_quad_are_constructed() {
        let subject = Subject::from(NamedNode::new("https://example.com/s").expect("subject IRI"));
        let predicate =
            Predicate::from(NamedNode::new("https://example.com/p").expect("predicate IRI"));
        let object = Object::from(
            Literal::new_typed("true", NamedNode::new(XSD_BOOLEAN_IRI).expect("datatype IRI"))
                .expect("typed literal"),
        );

        let triple = Triple::new(subject.clone(), predicate.clone(), object.clone());
        assert_eq!(triple.subject(), &subject);

        let quad = Quad::new(triple.subject().clone(), predicate, object, GraphName::DefaultGraph);
        assert_eq!(quad.graph_name(), &GraphName::DefaultGraph);
    }

    #[test]
    fn term_conversions_work() {
        let term = Term::from(Literal::new_simple("x"));
        assert!(matches!(term, Term::Literal(_)));
    }

    #[test]
    fn iri_parser_edge_cases() {
        assert!(NamedNode::new("https://example.com/a/b?x=1#frag").is_ok());
        assert!(NamedNode::new("http://example.com/\u{00fc}ber").is_ok());
        assert!(NamedNode::new("http://example.com/space path").is_err());
        assert!(NamedNode::new("://missing-scheme").is_err());
    }
}
