# Embedding provider extensions

This crate contains the initial local-sidecar and OpenAI-compatible adapters.
The semantic-search plugin does not depend on this crate in production.
`rdf-cli` registers these adapters at application startup.

To add a provider with a different API, create a separate crate depending on
`rdf-embedding-api`. Implement `EmbeddingProvider` (`health`, `embed`) and
`ProviderFactory` (`create`). Register it at the application composition root:

```rust,ignore
let mut registry = rdf_embedding_providers::registry()?;
registry.register("my-provider", my_embedding_provider::Factory)?;
let plugin = rdf_plugin_semantic_search::SemanticSearch::new(
    dataset_identity, config, &registry,
)?;
database.register(plugin)?;
```

Then select `{"kind":"my-provider","config":{...}}` in the provider map and
reference its name from an index. No semantic-search or database-core edits are
needed. Adding compiled code requires rebuilding the application; this is not a
runtime shared-library loader. Local and OpenAI configurations and concrete
examples are in the [semantic-search guide](../semantic-search/README.md).

The factory must validate its own config. `Profile.identity` must include every
non-secret setting that affects vector compatibility, including query/document
preprocessing and revisions. `embed` must verify the effective profile, preserve
input order, and return one finite, nonzero vector of the declared dimension per
input. Search also validates returned vectors defensively. Keep credentials and
private text out of debug/error messages and profiles. Profile identity must be
stable across executions; execution device should not be included if CPU/GPU
produce the same vector space.

Factories/providers are `Send + Sync`; methods are blocking and run on the
application's blocking-operation threads. No live provider I/O is required at
factory creation. Use bounded timeouts and explicit errors. The built-ins use a
5-second connect timeout, 180-second request timeout and no redirects/retries.

OpenAI's `revision` is operator-controlled; it cannot pin hosted model weights.
The local adapter obtains its immutable model revision from the sidecar.
See [ADR 0005](../../docs/adr/0005-embedding-provider-extensions.md) for the complete
compatibility and failure contract.
