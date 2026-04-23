use proptest::prelude::*;
use rdf_model::{GraphName, Literal, NamedNode, Object, Predicate, Quad, Subject};
use rdf_store::{QuadPattern, RedbQuadStorage};
use std::collections::HashSet;
use tempfile::tempdir;

#[derive(Clone, Debug)]
enum Op {
    Insert(usize),
    Remove(usize),
    Query(usize),
}

fn nn(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.com/{local}")).expect("valid IRI")
}

fn pool() -> Vec<Quad> {
    vec![
        Quad::new(
            Subject::NamedNode(nn("s1")),
            Predicate::new(nn("p1")),
            Object::Literal(Literal::new_simple("o1")),
            GraphName::DefaultGraph,
        ),
        Quad::new(
            Subject::NamedNode(nn("s2")),
            Predicate::new(nn("p1")),
            Object::Literal(Literal::new_simple("o2")),
            GraphName::DefaultGraph,
        ),
        Quad::new(
            Subject::NamedNode(nn("s1")),
            Predicate::new(nn("p2")),
            Object::Literal(Literal::new_simple("o3")),
            GraphName::DefaultGraph,
        ),
        Quad::new(
            Subject::NamedNode(nn("s3")),
            Predicate::new(nn("p2")),
            Object::Literal(Literal::new_simple("o4")),
            GraphName::NamedNode(nn("g1")),
        ),
    ]
}

fn patterns() -> Vec<QuadPattern> {
    vec![
        QuadPattern::default(),
        QuadPattern {
            subject: Some(Subject::NamedNode(nn("s1"))),
            predicate: None,
            object: None,
            graph_name: None,
        },
        QuadPattern {
            subject: None,
            predicate: Some(Predicate::new(nn("p1"))),
            object: None,
            graph_name: Some(GraphName::DefaultGraph),
        },
        QuadPattern {
            subject: None,
            predicate: Some(Predicate::new(nn("p2"))),
            object: None,
            graph_name: Some(GraphName::NamedNode(nn("g1"))),
        },
    ]
}

fn matches_pattern(quad: &Quad, pattern: &QuadPattern) -> bool {
    pattern.subject.as_ref().is_none_or(|s| s == quad.subject())
        && pattern.predicate.as_ref().is_none_or(|p| p == quad.predicate())
        && pattern.object.as_ref().is_none_or(|o| o == quad.object())
        && pattern.graph_name.as_ref().is_none_or(|g| g == quad.graph_name())
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0_usize..32).prop_map(Op::Insert),
        (0_usize..32).prop_map(Op::Remove),
        (0_usize..32).prop_map(Op::Query),
    ]
}

proptest! {
    #[test]
    fn model_based_sequences_match_reference_model_for_single_dataset(
        ops in prop::collection::vec(op_strategy(), 1..80)
    ) {
        let tmp = tempdir().expect("tempdir");
        let db_path = tmp.path().join("store.redb");
        let mut store = RedbQuadStorage::open(&db_path, "dataset_prop").expect("open redb store");

        let mut model = HashSet::new();
        let quad_pool = pool();
        let pattern_pool = patterns();

        for op in ops {
            match op {
                Op::Insert(i) => {
                    let quad = quad_pool[i % quad_pool.len()].clone();
                    let expected = model.insert(quad.clone());
                    let actual = store.insert(quad).expect("insert");
                    prop_assert_eq!(actual, expected);
                }
                Op::Remove(i) => {
                    let quad = quad_pool[i % quad_pool.len()].clone();
                    let expected = model.remove(&quad);
                    let actual = store.remove(&quad).expect("remove");
                    prop_assert_eq!(actual, expected);
                }
                Op::Query(i) => {
                    let pattern = pattern_pool[i % pattern_pool.len()].clone();
                    let first = store.quads_matching(&pattern).expect("query first");
                    let second = store.quads_matching(&pattern).expect("query second");
                    prop_assert_eq!(first.clone(), second);

                    let actual: HashSet<Quad> = first.into_iter().collect();
                    let expected: HashSet<Quad> = model
                        .iter()
                        .filter(|quad| matches_pattern(quad, &pattern))
                        .cloned()
                        .collect();
                    prop_assert_eq!(actual, expected);
                }
            }

            let actual_all: HashSet<Quad> = store.all_quads().expect("all quads").into_iter().collect();
            prop_assert_eq!(actual_all, model.clone());
        }
    }
}
