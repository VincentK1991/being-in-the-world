use rdf_model::{GraphName, Literal, NamedNode, Object, Predicate, Quad, Subject};
use rdf_store::{QuadPattern, RedbQuadStorage};
use tempfile::tempdir;

fn nn(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.com/{local}")).expect("valid IRI")
}

fn quad_default(s: &str, p: &str, o: &str) -> Quad {
    Quad::new(
        Subject::NamedNode(nn(s)),
        Predicate::new(nn(p)),
        Object::Literal(Literal::new_simple(o)),
        GraphName::DefaultGraph,
    )
}

#[test]
fn redb_persists_quads_across_reopen() {
    let tmp = tempdir().expect("tempdir");
    let db_path = tmp.path().join("store.redb");

    {
        let mut store = RedbQuadStorage::open(&db_path, "dataset_a").expect("open redb store");
        assert!(store.insert(quad_default("s1", "p1", "o1")).expect("insert s1"));
        assert!(store.insert(quad_default("s2", "p2", "o2")).expect("insert s2"));
    }

    let store = RedbQuadStorage::open(&db_path, "dataset_a").expect("reopen redb store");
    let pattern = QuadPattern {
        subject: Some(Subject::NamedNode(nn("s1"))),
        predicate: None,
        object: None,
        graph_name: Some(GraphName::DefaultGraph),
    };

    let matches = store.quads_matching(&pattern).expect("query after reopen");
    assert_eq!(matches.len(), 1);
    assert_eq!(store.stats().expect("stats").quads, 2);
}

#[test]
fn redb_isolates_datasets_in_one_database() {
    let tmp = tempdir().expect("tempdir");
    let db_path = tmp.path().join("store.redb");

    {
        let mut ds_a = RedbQuadStorage::open(&db_path, "dataset_a").expect("open dataset a");
        assert!(ds_a.insert(quad_default("alice", "p", "a")).expect("insert dataset a"));
    }
    {
        let mut ds_b = RedbQuadStorage::open(&db_path, "dataset_b").expect("open dataset b");
        assert!(ds_b.insert(quad_default("bob", "p", "b")).expect("insert dataset b"));
    }

    let by_predicate = QuadPattern {
        subject: None,
        predicate: Some(Predicate::new(nn("p"))),
        object: None,
        graph_name: Some(GraphName::DefaultGraph),
    };

    let a_rows = RedbQuadStorage::open(&db_path, "dataset_a")
        .expect("reopen dataset a")
        .quads_matching(&by_predicate)
        .expect("query dataset a");
    let b_rows = RedbQuadStorage::open(&db_path, "dataset_b")
        .expect("reopen dataset b")
        .quads_matching(&by_predicate)
        .expect("query dataset b");

    assert_eq!(a_rows.len(), 1);
    assert_eq!(b_rows.len(), 1);
    assert_eq!(a_rows[0].subject(), &Subject::NamedNode(nn("alice")));
    assert_eq!(b_rows[0].subject(), &Subject::NamedNode(nn("bob")));
}

#[test]
fn redb_insert_delete_and_idempotence() {
    let tmp = tempdir().expect("tempdir");
    let db_path = tmp.path().join("store.redb");

    let mut store = RedbQuadStorage::open(&db_path, "dataset_a").expect("open redb store");
    let keep = quad_default("s1", "p", "o1");
    let drop = quad_default("s2", "p", "o2");

    assert!(store.insert(keep.clone()).expect("insert keep"));
    assert!(store.insert(drop.clone()).expect("insert drop"));
    assert!(!store.insert(drop.clone()).expect("duplicate insert"));

    assert!(store.remove(&drop).expect("remove present"));
    assert!(!store.remove(&drop).expect("remove missing"));

    let remaining = store.all_quads().expect("all quads");
    assert_eq!(remaining, vec![keep]);
}

#[test]
fn redb_empty_dataset_remove_and_scan_are_stable() {
    let tmp = tempdir().expect("tempdir");
    let db_path = tmp.path().join("store.redb");

    let mut store = RedbQuadStorage::open(&db_path, "dataset_empty").expect("open redb store");
    let missing = quad_default("s", "p", "o");

    assert!(!store.remove(&missing).expect("remove on empty dataset"));
    assert!(store.all_quads().expect("scan empty dataset").is_empty());
    assert_eq!(store.stats().expect("stats empty dataset").quads, 0);
}
