use crate::{
    DictionaryError, InlineValue, Literal, Term, TermId, XSD_BOOLEAN_IRI, XSD_INTEGER_IRI,
};
use std::collections::HashMap;

pub trait Dictionary {
    type Error;

    fn encode(&mut self, term: &Term) -> Result<TermId, Self::Error>;
    fn decode(&self, id: TermId) -> Result<Term, Self::Error>;
}

#[derive(Debug, Default)]
pub struct InMemoryDictionary {
    term_to_id: HashMap<Term, TermId>,
    id_to_term: HashMap<TermId, Term>,
    next_id: u64,
}

impl InMemoryDictionary {
    pub fn new() -> Self {
        Self { term_to_id: HashMap::new(), id_to_term: HashMap::new(), next_id: 1 }
    }

    pub fn len(&self) -> usize {
        self.term_to_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.term_to_id.is_empty()
    }
}

impl Dictionary for InMemoryDictionary {
    type Error = DictionaryError;

    fn encode(&mut self, term: &Term) -> Result<TermId, Self::Error> {
        if let Some(inline) = try_inline_term(term)? {
            return Ok(inline);
        }

        if let Some(id) = self.term_to_id.get(term).copied() {
            return Ok(id);
        }

        let id =
            TermId::dictionary(self.next_id).map_err(|_| DictionaryError::TermIdSpaceExhausted)?;
        self.next_id = self.next_id.checked_add(1).ok_or(DictionaryError::TermIdSpaceExhausted)?;
        self.term_to_id.insert(term.clone(), id);
        self.id_to_term.insert(id, term.clone());
        Ok(id)
    }

    fn decode(&self, id: TermId) -> Result<Term, Self::Error> {
        if id.is_inline() {
            return decode_inline_term(id);
        }

        self.id_to_term.get(&id).cloned().ok_or(DictionaryError::UnknownTermId(id))
    }
}

fn try_inline_term(term: &Term) -> Result<Option<TermId>, DictionaryError> {
    let Term::Literal(literal) = term else {
        return Ok(None);
    };
    if literal.language().is_some() {
        return Ok(None);
    }

    match literal.datatype().as_str() {
        XSD_BOOLEAN_IRI => match literal.lexical_form() {
            "true" => Ok(Some(TermId::inline_bool(true))),
            "false" => Ok(Some(TermId::inline_bool(false))),
            _ => Ok(None),
        },
        XSD_INTEGER_IRI => {
            let parsed: i64 = match literal.lexical_form().parse() {
                Ok(value) => value,
                Err(_) => return Ok(None),
            };
            if parsed.to_string() != literal.lexical_form() {
                return Ok(None);
            }
            Ok(Some(TermId::inline_i60(parsed)?))
        }
        crate::XSD_STRING_IRI => {
            let text = literal.lexical_form();
            if text.len() <= 7 { Ok(Some(TermId::inline_short_string(text)?)) } else { Ok(None) }
        }
        _ => Ok(None),
    }
}

fn decode_inline_term(id: TermId) -> Result<Term, DictionaryError> {
    let term = match id.decode_inline()? {
        InlineValue::Bool(value) => {
            let lexical = if value { "true" } else { "false" };
            Term::Literal(Literal::new_typed_unchecked(
                lexical,
                crate::model::xsd_boolean_datatype(),
            ))
        }
        InlineValue::I64(value) => Term::Literal(Literal::new_typed_unchecked(
            value.to_string(),
            crate::model::xsd_integer_datatype(),
        )),
        InlineValue::ShortString(text) => Term::Literal(Literal::new_simple(text)),
    };
    Ok(term)
}

#[cfg(test)]
mod tests {
    use super::{Dictionary, InMemoryDictionary};
    use crate::{BlankNode, Literal, NamedNode, Term};
    use proptest::prelude::*;

    #[test]
    fn dictionary_round_trip_non_inline_term() {
        let mut dictionary = InMemoryDictionary::new();
        let term = Term::from(BlankNode::new("node_1").expect("valid blank node"));

        let id = dictionary.encode(&term).expect("encode term");
        assert!(id.as_dictionary().is_some());
        let decoded = dictionary.decode(id).expect("decode term");
        assert_eq!(decoded, term);
    }

    #[test]
    fn dictionary_reuses_existing_ids() {
        let mut dictionary = InMemoryDictionary::new();
        let term = Term::from(NamedNode::new("https://example.com/resource").expect("valid IRI"));

        let id_1 = dictionary.encode(&term).expect("first encode");
        let id_2 = dictionary.encode(&term).expect("second encode");
        assert_eq!(id_1, id_2);
        assert_eq!(dictionary.len(), 1);
    }

    #[test]
    fn dictionary_inlines_boolean_literal() {
        let mut dictionary = InMemoryDictionary::new();
        let term = Term::from(
            Literal::new_typed(
                "true",
                NamedNode::new(crate::XSD_BOOLEAN_IRI).expect("valid datatype IRI"),
            )
            .expect("typed literal"),
        );

        let id = dictionary.encode(&term).expect("encode term");
        assert!(id.is_inline());
        let decoded = dictionary.decode(id).expect("decode term");
        assert_eq!(decoded, term);
    }

    fn named_node_strategy() -> impl Strategy<Value = NamedNode> {
        "[a-z][a-z0-9]{0,10}".prop_map(|suffix| {
            NamedNode::new(format!("https://example.com/{suffix}"))
                .expect("strategy builds valid IRI")
        })
    }

    fn blank_node_strategy() -> impl Strategy<Value = BlankNode> {
        "[a-zA-Z_][a-zA-Z0-9_.-]{0,10}"
            .prop_map(|label| BlankNode::new(label).expect("strategy builds valid blank node"))
    }

    fn literal_strategy() -> impl Strategy<Value = Literal> {
        let simple = "[ -~]{0,12}".prop_map(Literal::new_simple);
        let bools = proptest::bool::ANY.prop_map(|flag| {
            let lexical = if flag { "true" } else { "false" };
            Literal::new_typed(
                lexical,
                NamedNode::new(crate::XSD_BOOLEAN_IRI).expect("constant IRI"),
            )
            .expect("typed literal")
        });
        let ints = (-500_000_i64..500_000_i64).prop_map(|value| {
            Literal::new_typed(
                value.to_string(),
                NamedNode::new(crate::XSD_INTEGER_IRI).expect("constant IRI"),
            )
            .expect("typed literal")
        });
        prop_oneof![simple, bools, ints]
    }

    fn term_strategy() -> impl Strategy<Value = Term> {
        prop_oneof![
            named_node_strategy().prop_map(Term::from),
            blank_node_strategy().prop_map(Term::from),
            literal_strategy().prop_map(Term::from),
        ]
    }

    proptest! {
        #[test]
        fn encode_decode_round_trip(term in term_strategy()) {
            let mut dictionary = InMemoryDictionary::new();
            let id = dictionary.encode(&term).expect("encode term");
            let decoded = dictionary.decode(id).expect("decode term");
            prop_assert_eq!(decoded, term);
        }
    }
}
