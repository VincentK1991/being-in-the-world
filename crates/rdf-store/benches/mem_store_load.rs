use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use rdf_model::{GraphName, Literal, NamedNode, Object, Predicate, Quad, Subject};
use rdf_store::MemQuadStorage;

const QUAD_COUNT: usize = 1_000_000;

fn quad_at(index: usize) -> Quad {
    let subject = NamedNode::new(format!("http://example.com/s/{index}"))
        .expect("bench generates valid subject IRI");
    let predicate = NamedNode::new(format!("http://example.com/p/{index}"))
        .expect("bench generates valid predicate IRI");
    let object = Literal::new_simple(format!("value-{index}"));

    Quad::new(
        Subject::NamedNode(subject),
        Predicate::new(predicate),
        Object::Literal(object),
        GraphName::DefaultGraph,
    )
}

fn bench_load_million_quads(c: &mut Criterion) {
    c.bench_function("mem_store_load_1m_quads", |b| {
        b.iter_batched(
            MemQuadStorage::new,
            |mut store| {
                for i in 0..QUAD_COUNT {
                    store.insert(quad_at(i)).expect("bench insert must succeed");
                }
                assert_eq!(store.len(), QUAD_COUNT);
            },
            BatchSize::LargeInput,
        )
    });
}

criterion_group!(benches, bench_load_million_quads);
criterion_main!(benches);
