//! Loopback-only SPARQL and Graph Store service.
pub mod gateway;
use anyhow::{Result, ensure};
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, RawQuery, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use rdf_engine::{
    Database, GraphMutation, GraphName, NamedNode, QueryResults, QueryResultsFormat, RdfFormat,
    write_results,
};
use rdf_plugin_api::CatalogEntry;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::Semaphore;

#[derive(Clone)]
struct App {
    data: Database,
    permits: Arc<Semaphore>,
    requests: Arc<AtomicU64>,
    max_body: usize,
}
type Failure = (StatusCode, String);
fn bad(error: impl std::fmt::Display) -> Failure {
    (StatusCode::BAD_REQUEST, error.to_string())
}
fn params(query: &str) -> Result<HashMap<String, String>, Failure> {
    serde_urlencoded::from_str(query).map_err(bad)
}

include!(concat!(env!("OUT_DIR"), "/studio_assets.rs"));
async fn asset(uri: axum::http::Uri) -> Response {
    if let Some((_, mime, bytes)) = ASSETS.iter().find(|(path, _, _)| *path == uri.path()) {
        return ([("content-type", *mime)], *bytes).into_response();
    }
    if uri.path() == "/" {
        return (StatusCode::SERVICE_UNAVAILABLE, "Studio assets not built. Run npm ci --prefix apps/studio && npm run build --prefix apps/studio, then rebuild rdf-cli. The database API remains available.").into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}
pub fn router(data: Database, max_body: usize, concurrency: usize) -> Router {
    Router::new()
        .route("/", get(asset))
        .fallback(get(asset))
        .route("/api/plugins", get(plugins))
        .route("/api/plugins/{plugin}/operations/{operation}", post(plugin_operation))
        .route("/api/status", get(status))
        .route("/api/import", post(import))
        .route("/query", get(query).post(query))
        .route("/update", post(update))
        .route("/store", any(graph_store))
        .route("/health", get(|| async { "ok\n" }))
        .route("/metrics", get(metrics))
        .layer(DefaultBodyLimit::max(max_body))
        .with_state(App {
            data,
            permits: Arc::new(Semaphore::new(concurrency)),
            requests: Arc::new(AtomicU64::new(0)),
            max_body,
        })
}
pub fn serve(data: Database, bind: SocketAddr, max_body: usize, concurrency: usize) -> Result<()> {
    ensure!(bind.ip().is_loopback(), "only loopback binding is supported");
    ensure!(concurrency > 0 && max_body > 0, "limits must be positive");
    // Keep plugin resources alive until outside Tokio, including blocking HTTP clients.
    let _plugin_lifetime = data.clone();
    tokio::runtime::Runtime::new()?.block_on(async move {
        let listener = tokio::net::TcpListener::bind(bind).await?;
        eprintln!("listening on http://{}", listener.local_addr()?);
        axum::serve(listener, router(data, max_body, concurrency))
            .with_graceful_shutdown(async {
                let mut terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("install SIGTERM handler");
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            })
            .await?;
        Ok::<_, anyhow::Error>(())
    })
}
async fn work(
    app: App,
    job: impl FnOnce(Database) -> Result<Response, Failure> + Send + 'static,
) -> Result<Response, Failure> {
    let permit =
        app.permits.clone().try_acquire_owned().map_err(|_| {
            (StatusCode::SERVICE_UNAVAILABLE, "operation capacity exhausted".into())
        })?;
    app.requests.fetch_add(1, Ordering::Relaxed);
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job(app.data)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
}
fn content_type(headers: &HeaderMap) -> &str {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
}
fn sparql(
    method: &Method,
    raw: Option<String>,
    headers: &HeaderMap,
    body: Bytes,
    key: &str,
) -> Result<String, Failure> {
    let mut values = params(raw.as_deref().unwrap_or(""))?;
    if values.keys().any(|k| k != key) {
        return Err(bad("protocol dataset parameters are not supported yet"));
    }
    if method == Method::POST {
        match content_type(headers) {
            "application/x-www-form-urlencoded" => {
                values = params(std::str::from_utf8(&body).map_err(bad)?)?;
            }
            mime if mime == format!("application/sparql-{key}") => {
                return String::from_utf8(body.to_vec()).map_err(bad);
            }
            _ => {
                return Err((
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "unsupported SPARQL content type".into(),
                ));
            }
        }
    }
    values.remove(key).ok_or_else(|| bad(format!("missing {key}")))
}
fn negotiated(
    headers: &HeaderMap,
    graph: bool,
) -> Result<(&'static str, QueryResultsFormat, RdfFormat), Failure> {
    let accept = headers.get("accept").and_then(|v| v.to_str().ok()).unwrap_or("*/*");
    for item in accept.split(',') {
        let mime = item.split(';').next().unwrap_or("").trim();
        if graph {
            let format = if mime == "*/*" {
                Some(RdfFormat::Turtle)
            } else {
                RdfFormat::from_media_type(mime)
            };
            if let Some(format) = format {
                return Ok((format.media_type(), QueryResultsFormat::Json, format));
            }
        } else {
            let format = if mime == "*/*" {
                Some(QueryResultsFormat::Json)
            } else {
                QueryResultsFormat::from_media_type(mime)
            };
            if let Some(format) = format {
                return Ok((format.media_type(), format, RdfFormat::Turtle));
            }
        }
    }
    Err((StatusCode::NOT_ACCEPTABLE, "no supported result media type".into()))
}
/// SPARQL 1.1 Service Description for this dataset's server, listing its plugins in the
/// `urn:rdftool:vocab:` vocabulary. The same catalog is queryable as `urn:rdftool:plugins:provides`.
fn service_description(catalog: &[CatalogEntry]) -> String {
    use std::fmt::Write;
    let mut out = String::from(
        "@prefix sd: <http://www.w3.org/ns/sparql-service-description#> .
@prefix rdftool: <urn:rdftool:vocab:> .
<> a sd:Service ;
  sd:endpoint <> ;
  sd:supportedLanguage sd:SPARQL11Query ;
  sd:resultFormat <http://www.w3.org/ns/formats/SPARQL_Results_JSON>, <http://www.w3.org/ns/formats/SPARQL_Results_XML>, <http://www.w3.org/ns/formats/SPARQL_Results_CSV>, <http://www.w3.org/ns/formats/Turtle>",
    );
    let plugins: Vec<&CatalogEntry> = catalog.iter().filter(|row| row.kind == "plugin").collect();
    for plugin in &plugins {
        let _ = write!(out, " ;\n  rdftool:plugin <#{}>", plugin.plugin);
    }
    out.push_str(" .\n");
    for plugin in plugins {
        let _ = write!(
            out,
            "<#{id}> a rdftool:Plugin ;\n  rdftool:id {id:?} ;\n  rdftool:version {version:?}",
            id = plugin.plugin,
            version = plugin.version
        );
        for row in catalog.iter().filter(|r| r.plugin == plugin.plugin && r.kind != "plugin") {
            let predicate = match row.kind.as_str() {
                "property-function" => "propertyFunction".to_owned(),
                "virtual-graph" => "virtualGraph".to_owned(),
                other => other.to_owned(),
            };
            if row.kind == "property-function" {
                let _ = write!(out, " ;\n  rdftool:{predicate} <{}>", row.name);
            } else {
                let _ = write!(out, " ;\n  rdftool:{predicate} {:?}", row.name);
            }
        }
        out.push_str(" .\n");
    }
    out
}
async fn query(
    State(app): State<App>,
    method: Method,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    // A bare GET of the endpoint describes the service (SPARQL 1.1 Service Description), which
    // federation engines request before evaluating a SERVICE clause against it.
    if method == Method::GET
        && !raw.as_deref().is_some_and(|q| q.split('&').any(|p| p.starts_with("query=")))
    {
        return Ok(([("content-type", "text/turtle")], service_description(&app.data.catalog()))
            .into_response());
    }
    let query = sparql(&method, raw, &headers, body, "query")?;
    work(app, move |data| {
        let results = data.query(&query).map_err(bad)?;
        let (mime, table, graph) =
            negotiated(&headers, matches!(&results, QueryResults::Graph(_)))?;
        let mut bytes = Vec::new();
        write_results(results, table, graph, &mut bytes).map_err(bad)?;
        Ok(([("content-type", mime)], bytes).into_response())
    })
    .await
}
async fn update(
    State(app): State<App>,
    method: Method,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    let update = sparql(&method, raw, &headers, body, "update")?;
    work(app, move |data| {
        data.update(&update).map_err(bad)?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}
async fn metrics(State(app): State<App>) -> String {
    format!(
        "# TYPE rdf_operations_total counter\nrdf_operations_total {}\n",
        app.requests.load(Ordering::Relaxed)
    )
}

async fn status(State(app): State<App>) -> Result<Response, Failure> {
    let max_body = app.max_body;
    work(app, move |data| {
        let overview = data.overview().map_err(bad)?;
        Ok(axum::Json(serde_json::json!({
            "connected": true,
            "quads": overview.quads,
            "graphs": overview.graphs,
            "max_upload_bytes": max_body,
            "engine": "Oxigraph",
        }))
        .into_response())
    })
    .await
}

/// Raw RDF upload: parse and commit within one transaction, including named graphs.
async fn import(
    State(app): State<App>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    let values = params(raw.as_deref().unwrap_or(""))?;
    if values.keys().any(|key| key != "graph") {
        return Err(bad("unknown import parameter"));
    }
    let media_type = content_type(&headers);
    if !matches!(
        media_type,
        "text/turtle" | "application/n-triples" | "application/trig" | "application/n-quads"
    ) {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Upload Turtle, N-Triples, TriG, or N-Quads.".into(),
        ));
    }
    let format = RdfFormat::from_media_type(media_type)
        .filter(|format| {
            matches!(
                format,
                RdfFormat::Turtle | RdfFormat::NTriples | RdfFormat::TriG | RdfFormat::NQuads
            )
        })
        .ok_or_else(|| {
            (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Upload Turtle, N-Triples, TriG, or N-Quads.".into(),
            )
        })?;
    if body.is_empty() {
        return Err(bad("The uploaded file is empty."));
    }
    let graph =
        values.get("graph").map(|iri| NamedNode::new(iri.as_str())).transpose().map_err(bad)?;
    if graph.is_some() && matches!(format, RdfFormat::TriG | RdfFormat::NQuads) {
        return Err(bad("TriG and N-Quads define their own graphs; leave the target graph empty."));
    }
    work(app, move |data| {
        Ok(axum::Json(data.import(&body, format, graph).map_err(bad)?).into_response())
    })
    .await
}
async fn graph_store(
    State(app): State<App>,
    method: Method,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    let values = params(raw.as_deref().unwrap_or(""))?;
    let graph = if values.len() == 1 && values.contains_key("default") {
        GraphName::DefaultGraph
    } else if values.len() == 1 {
        GraphName::NamedNode(
            NamedNode::new(
                values.get("graph").ok_or_else(|| bad("expected graph or default parameter"))?,
            )
            .map_err(bad)?,
        )
    } else {
        return Err(bad("exactly one graph or default parameter required"));
    };
    work(app, move |data| {
        let exists = data.graph_exists(&graph).map_err(bad)?;
        if method == Method::GET {
            if !exists {
                return Err((StatusCode::NOT_FOUND, "graph does not exist".into()));
            }
            let (mime, _, format) = negotiated(&headers, true)?;
            let bytes = data.dump_graph(&graph, format).map_err(bad)?;
            return Ok(([("content-type", mime)], bytes).into_response());
        }
        if method != Method::DELETE && method != Method::POST && method != Method::PUT {
            return Err((StatusCode::METHOD_NOT_ALLOWED, "method not supported".into()));
        }
        if method == Method::DELETE && !exists {
            return Err((StatusCode::NOT_FOUND, "graph does not exist".into()));
        }
        let format = if method == Method::DELETE {
            RdfFormat::Turtle
        } else {
            RdfFormat::from_media_type(content_type(&headers))
                .filter(|f| matches!(f, RdfFormat::NTriples | RdfFormat::Turtle))
                .ok_or_else(|| {
                    (
                        StatusCode::UNSUPPORTED_MEDIA_TYPE,
                        "graph input must be Turtle or N-Triples".into(),
                    )
                })?
        };
        let mutation = if method == Method::DELETE {
            GraphMutation::Delete
        } else if method == Method::PUT {
            GraphMutation::Replace
        } else {
            GraphMutation::Append
        };
        data.mutate_graph(graph, mutation, format, &body).map_err(bad)?;
        Ok(if !exists { StatusCode::CREATED } else { StatusCode::NO_CONTENT }.into_response())
    })
    .await
}

async fn plugins(State(app): State<App>) -> Response {
    axum::Json(app.data.plugins()).into_response()
}
async fn plugin_operation(
    State(app): State<App>,
    Path((plugin, operation)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    if content_type(&headers) != "application/json" {
        return Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "expected application/json".into()));
    }
    if !app.data.has_operation(&plugin, &operation) {
        return Err((StatusCode::NOT_FOUND, "unknown plugin or operation".into()));
    }
    let input = serde_json::from_slice(&body).map_err(bad)?;
    work(app, move |data| {
        Ok(axum::Json(data.invoke(&plugin, &operation, input).map_err(bad)?).into_response())
    })
    .await
}
