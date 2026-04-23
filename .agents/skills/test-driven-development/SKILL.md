---
name: test-driven-development
description: Rust TDD workflow for RDF store development, prioritizing property, model-based, integration, and conformance tests over unit tests.
---

Use this skill when implementing or changing RDF store behavior in Rust and you need clear, test-driven completion criteria.

## Primary stance

- Default to broad behavioral tests first: property tests, model-based tests, integration tests, and conformance suites.
- Use unit tests only when they provide focused value (pure logic, edge-case parser behavior, small internal invariants).
- A feature is "done" only when behavior is proven at the store/API/query boundary, not only at helper-function level.

## Required Red-Green-Refactor loop

1. Define feature contract first
- Write a short contract: inputs, outputs, invariants, failure modes, and persistence/transaction expectations.
- Include RDF-specific semantics (graph scope, blank nodes, literal equality rules, duplicate triple behavior).

2. Write failing high-value tests before implementation
- Start with at least one failing behavior-level test in `tests/` (integration or scenario test).
- Add failing property tests for invariants likely to regress.
- If applicable, add failing conformance case (W3C-style manifest/input-output expectation).

3. Make minimal implementation to pass
- Implement only what is required to satisfy current failing tests.
- Avoid broad refactors during this phase unless required for correctness.

4. Refactor safely
- Improve structure/performance while keeping all tests green.
- Preserve and expand tests when discovering missing invariants.

5. Lock regressions
- For every bug fix, add a regression test first, watch it fail, then fix.

## Test strategy for RDF store (default order)

1. Property tests (first-class)
- Use `proptest` as the default engine.
- Test invariants over generated datasets and operation sequences.
- Keep generators domain-aware (valid IRIs/literals/graphs; explicit invalid cases for negative tests).

Core properties to cover:
- Insert/query consistency: inserted triples are queryable with correct graph semantics.
- Idempotence: inserting same triple twice does not create duplicate logical facts (unless API promises bag semantics).
- Delete correctness: after delete, removed triples are absent and unrelated triples remain.
- Round-trip stability: serialize -> parse -> canonicalize preserves graph meaning.
- Transaction behavior: commit persists changes, rollback does not.
- Index consistency: different access paths return equivalent result sets.
- Determinism: same data + query yields same logical results.

2. Model-based / state-machine tests
- Maintain a simple reference model (for example `HashSet`-backed store with naive evaluation).
- Generate operation sequences: insert/delete/query/commit/rollback.
- Compare implementation behavior against reference model after each step.

3. Integration tests (feature completion gate)
- Test through public API boundaries (storage engine + query interface).
- Prefer realistic feature scenarios over isolated internals.
- Include persistence/restart behavior when relevant.

4. Conformance tests
- Map supported RDF/SPARQL features to conformance vectors.
- Run approved spec fixtures for the supported subset and track unsupported items explicitly.

5. Unit tests (targeted, not default)
- Use for pure functions and tight edge cases where property/integration tests are expensive or unclear.
- Do not treat unit test coverage alone as feature completion.

## Practical Rust setup

- Typical crates:
  - `proptest` for property tests.
  - `insta` for snapshotting stable query outputs when useful.
  - `tempfile` for persistence/integration scenarios.
- Recommended layout:
  - `src/...` for implementation
  - `tests/properties/*.rs`
  - `tests/integration/*.rs`
  - `tests/conformance/*.rs`

Example property test shape:

```rust
proptest! {
    #[test]
    fn insert_then_query_returns_inserted_triples(ops in arb_store_ops()) {
        let mut sut = TestStore::new();
        let mut model = RefStore::new();

        for op in ops {
            apply_to_sut(&mut sut, &op);
            apply_to_model(&mut model, &op);
            prop_assert_eq!(sut.visible_triples(), model.visible_triples());
        }
    }
}
```

## Definition of done for a feature

A feature is complete only if all conditions hold:

- The feature exists in `docs/feature-checklist.md` with a clear checkbox item.
- A failing behavior-level test existed before implementation and now passes.
- Relevant property tests exist and pass for the new behavior/invariants.
- Integration tests covering real user flow pass.
- Conformance tests for supported related semantics pass (or are explicitly marked unsupported with rationale).
- At least one regression test exists for each bug discovered while implementing.
- `cargo test` passes consistently without flaky/non-deterministic failures.
- Only after required tests pass, mark that feature checkbox as complete in `docs/feature-checklist.md`.

## Decision rules while coding

- If behavior spans multiple modules, prefer integration/property tests before adding unit tests.
- If a failing case is hard to express with random data, add a deterministic scenario test first, then generalize with properties.
- If test failures are hard to diagnose, improve shrinking/generator labeling and add minimal fixtures.
- If a change is performance-motivated, keep behavior tests unchanged and add a separate benchmark check; do not weaken correctness gates.

## Output expectations when using this skill

When asked to implement a feature:

- First output: explicit test plan (properties + integration/conformance coverage) and checklist item(s) to update in `docs/feature-checklist.md`.
- Then implement in tight TDD iterations and report:
  - Which tests were added first and failed initially.
  - What code was changed to pass them.
  - Which invariants are now guarded against regression.
  - Which checklist boxes were checked after tests passed.
