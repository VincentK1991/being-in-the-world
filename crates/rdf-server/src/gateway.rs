//! Dataset gateway (ADR 0008): one `rdftool serve` child per dataset, routed by name.
//!
//! `/d/<dataset>/...` is forwarded to that dataset's server, started on first use. Every other
//! path goes to the default dataset, so existing clients keep working. `/api/datasets` lists,
//! creates and deletes datasets. Each child is an ordinary single-dataset server, so engine
//! locking, plugins and limits are unchanged and tenants never share a process.
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, RawQuery, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, delete, get},
};
use rdf_store::engine::{self, Dataset, DatasetEntry, valid_dataset_name};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

/// Runs after a dataset is deleted so plugins can drop derived state held outside the store.
pub type DeleteHook = Arc<dyn Fn(&DatasetEntry) -> Result<()> + Send + Sync>;

pub struct GatewayConfig {
    pub root: PathBuf,
    /// The `rdftool` binary to start for each dataset (normally the current executable).
    pub executable: PathBuf,
    pub bind: SocketAddr,
    pub default_dataset: String,
    pub semantic_config: Option<PathBuf>,
    pub inference_config: Option<PathBuf>,
    /// Optional per-dataset plugin settings: `<dir>/<dataset>/semantic.json` and `inference.json`
    /// replace the global configuration for that dataset when present.
    pub tenant_config_dir: Option<PathBuf>,
    pub max_body: usize,
    pub concurrency: usize,
    pub on_delete: DeleteHook,
}

struct Running {
    port: u16,
    process: Child,
}
struct Gateway {
    config: GatewayConfig,
    client: reqwest::blocking::Client,
    children: Mutex<HashMap<String, Running>>,
}
type Failure = (StatusCode, String);
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
fn fail(status: StatusCode, message: impl std::fmt::Display) -> Failure {
    (status, message.to_string())
}

impl Gateway {
    fn entries(&self) -> Result<Vec<DatasetEntry>> {
        engine::list_datasets(&self.config.root)
    }
    fn exists(&self, name: &str) -> Result<bool> {
        Ok(self.entries()?.iter().any(|entry| entry.name == name))
    }
    fn child_args(&self, name: &str, port: u16) -> Vec<String> {
        let tenant = |file: &str| {
            self.config
                .tenant_config_dir
                .as_ref()
                .map(|dir| dir.join(name).join(file))
                .filter(|path| path.is_file())
        };
        let mut args = Vec::new();
        for (flag, specific, global) in [
            ("--semantic-config", tenant("semantic.json"), &self.config.semantic_config),
            ("--inference-config", tenant("inference.json"), &self.config.inference_config),
        ] {
            if let Some(path) = specific.or_else(|| global.clone()) {
                args.extend([flag.to_owned(), path.display().to_string()]);
            }
        }
        args.extend([
            "serve".into(),
            "--db".into(),
            self.config.root.display().to_string(),
            "--dataset".into(),
            name.into(),
            "--bind".into(),
            format!("127.0.0.1:{port}"),
            "--max-body-bytes".into(),
            self.config.max_body.to_string(),
            "--concurrency".into(),
            self.config.concurrency.to_string(),
        ]);
        args
    }

    /// `<tenant dir>/<dataset>/{virtual,federation}.json` enable those plugins for one dataset; without a
    /// tenant file the child inherits the gateway's own `RDF_VIRTUAL_CONFIG` / `RDF_FEDERATION_CONFIG`.
    fn child_env(&self, name: &str) -> Vec<(&'static str, PathBuf)> {
        let Some(dir) = &self.config.tenant_config_dir else { return Vec::new() };
        [
            ("RDF_VIRTUAL_CONFIG", "virtual.json"),
            ("RDF_FEDERATION_CONFIG", "federation.json"),
            ("RDF_REFRESH_CONFIG", "refresh.json"),
        ]
        .into_iter()
        .map(|(variable, file)| (variable, dir.join(name).join(file)))
        .filter(|(_, path)| path.is_file())
        .collect()
    }

    /// Returns the loopback port of a running server for `name`, starting one if needed.
    fn ensure_running(&self, name: &str) -> Result<u16> {
        let mut children = lock(&self.children);
        if let Some(running) = children.get_mut(name) {
            if running.process.try_wait()?.is_none() {
                return Ok(running.port);
            }
            children.remove(name);
        }
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let mut process = Command::new(&self.config.executable)
            .args(self.child_args(name, port))
            .envs(self.child_env(name))
            .stdin(Stdio::null())
            .spawn()
            .context("starting dataset server")?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = process.try_wait()? {
                bail!("dataset server for {name} exited during startup ({status})");
            }
            let ready = self
                .client
                .get(format!("http://127.0.0.1:{port}/health"))
                .timeout(Duration::from_secs(2))
                .send()
                .is_ok_and(|response| response.status().is_success());
            if ready {
                break;
            }
            if Instant::now() > deadline {
                let _ = process.kill();
                let _ = process.wait();
                bail!("dataset server for {name} did not become ready within 60s");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        children.insert(name.to_owned(), Running { port, process });
        Ok(port)
    }

    /// Stops the server for `name` (SIGTERM, then SIGKILL) so its database lock is released.
    fn stop(&self, name: &str) {
        let Some(mut running) = lock(&self.children).remove(name) else { return };
        let _ = Command::new("kill").args(["-TERM", &running.process.id().to_string()]).status();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if matches!(running.process.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = running.process.kill();
        let _ = running.process.wait();
    }
    fn stop_all(&self) {
        let names: Vec<String> = lock(&self.children).keys().cloned().collect();
        for name in names {
            self.stop(&name);
        }
    }

    fn running_port(&self, name: &str) -> Option<u16> {
        let mut children = lock(&self.children);
        let running = children.get_mut(name)?;
        running.process.try_wait().ok()?.is_none().then_some(running.port)
    }
    fn describe(&self, entry: &DatasetEntry) -> Value {
        let port = self.running_port(&entry.name);
        let status: Option<Value> = port.and_then(|port| {
            self.client
                .get(format!("http://127.0.0.1:{port}/api/status"))
                .timeout(Duration::from_secs(3))
                .send()
                .ok()?
                .bytes()
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        });
        json!({
            "name": entry.name,
            "id": entry.id,
            "running": port.is_some(),
            "default": entry.name == self.config.default_dataset,
            "quads": status.as_ref().and_then(|s| s["quads"].as_u64()),
            "graphs": status.as_ref().and_then(|s| s["graphs"].as_array()).map(Vec::len),
        })
    }

    fn forward(
        &self,
        name: &str,
        method: Method,
        path: &str,
        query: Option<&str>,
        headers: &HeaderMap,
        body: Bytes,
    ) -> Result<Response, Failure> {
        let port = self.ensure_running(name).map_err(|e| fail(StatusCode::BAD_GATEWAY, e))?;
        let url = match query {
            Some(query) => format!("http://127.0.0.1:{port}{path}?{query}"),
            None => format!("http://127.0.0.1:{port}{path}"),
        };
        let mut request = self.client.request(method, url).timeout(Duration::from_secs(3600));
        for header in ["content-type", "accept"] {
            if let Some(value) = headers.get(header) {
                request = request.header(header, value);
            }
        }
        let upstream = request.body(body).send().map_err(|e| {
            fail(StatusCode::BAD_GATEWAY, format!("dataset server unavailable: {e}"))
        })?;
        let status = upstream.status();
        let mut out = HeaderMap::new();
        for header in ["content-type", "content-disposition"] {
            if let Some(value) = upstream.headers().get(header) {
                out.insert(header, value.clone());
            }
        }
        let bytes = upstream.bytes().map_err(|e| fail(StatusCode::BAD_GATEWAY, e))?;
        Ok((status, out, bytes).into_response())
    }
}

type Shared = Arc<Gateway>;
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, Failure> + Send + 'static,
) -> Result<T, Failure> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e))?
}
fn internal(error: impl std::fmt::Display) -> Failure {
    fail(StatusCode::INTERNAL_SERVER_ERROR, error)
}

async fn list(State(gw): State<Shared>) -> Result<Json<Value>, Failure> {
    blocking(move || {
        let datasets: Vec<Value> =
            gw.entries().map_err(internal)?.iter().map(|entry| gw.describe(entry)).collect();
        Ok(Json(json!({"default": gw.config.default_dataset, "datasets": datasets})))
    })
    .await
}
async fn create(State(gw): State<Shared>, body: Bytes) -> Result<Response, Failure> {
    let request: Value = serde_json::from_slice(&body).map_err(|_| {
        fail(StatusCode::BAD_REQUEST, "expected JSON like {\"name\": \"tenant-1\"}")
    })?;
    let name = request["name"].as_str().unwrap_or_default().to_owned();
    if !valid_dataset_name(&name) {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "dataset name must be 1-64 letters, digits, '-', '_' or '.', and not start with '.'",
        ));
    }
    blocking(move || {
        if gw.exists(&name).map_err(internal)? {
            return Err(fail(StatusCode::CONFLICT, format!("dataset already exists: {name}")));
        }
        gw.ensure_running(&name).map_err(|e| fail(StatusCode::BAD_GATEWAY, e))?;
        let entry = gw
            .entries()
            .map_err(internal)?
            .into_iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| internal("dataset was not created"))?;
        Ok((StatusCode::CREATED, Json(gw.describe(&entry))).into_response())
    })
    .await
}
async fn remove(
    State(gw): State<Shared>,
    Path(name): Path<String>,
) -> Result<Json<Value>, Failure> {
    blocking(move || {
        if name == gw.config.default_dataset {
            return Err(fail(StatusCode::CONFLICT, "the default dataset cannot be deleted"));
        }
        if !gw.exists(&name).map_err(internal)? {
            return Err(fail(StatusCode::NOT_FOUND, format!("dataset does not exist: {name}")));
        }
        gw.stop(&name);
        let entry = engine::delete_dataset(&gw.config.root, &name)
            .map_err(|e| fail(StatusCode::CONFLICT, format!("{e:#}")))?;
        let warning = (gw.config.on_delete)(&entry).err().map(|e| format!("{e:#}"));
        Ok(Json(json!({"deleted": name, "cleanup_warning": warning})))
    })
    .await
}

async fn named(
    State(gw): State<Shared>,
    method: Method,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    Path(parts): Path<Vec<String>>,
    body: Bytes,
) -> Result<Response, Failure> {
    let (name, rest) = parts.split_first().ok_or_else(|| internal("missing dataset"))?;
    let name = name.clone();
    let path = format!("/{}", rest.join("/"));
    blocking(move || {
        if !gw.exists(&name).map_err(internal)? {
            return Err(fail(StatusCode::NOT_FOUND, format!("dataset does not exist: {name}")));
        }
        gw.forward(&name, method, &path, query.as_deref(), &headers, body)
    })
    .await
}
async fn named_root(
    state: State<Shared>,
    method: Method,
    headers: HeaderMap,
    query: RawQuery,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<Response, Failure> {
    named(state, method, headers, query, Path(vec![name]), body).await
}
async fn default_dataset(
    State(gw): State<Shared>,
    method: Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    blocking(move || {
        let name = gw.config.default_dataset.clone();
        gw.forward(&name, method, uri.path(), uri.query(), &headers, body)
    })
    .await
}

fn router(gateway: Shared, max_body: usize) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok\n" }))
        .route("/api/datasets", get(list).post(create))
        .route("/api/datasets/{name}", delete(remove))
        .route("/d/{name}", any(named_root))
        .route("/d/{name}/{*rest}", any(named_with_rest))
        .fallback(default_dataset)
        .layer(DefaultBodyLimit::max(max_body))
        .with_state(gateway)
}
async fn named_with_rest(
    state: State<Shared>,
    method: Method,
    headers: HeaderMap,
    query: RawQuery,
    Path((name, rest)): Path<(String, String)>,
    body: Bytes,
) -> Result<Response, Failure> {
    named(state, method, headers, query, Path(vec![name, rest]), body).await
}

pub fn run(config: GatewayConfig) -> Result<()> {
    ensure!(config.bind.ip().is_loopback(), "only loopback binding is supported");
    ensure!(valid_dataset_name(&config.default_dataset), "invalid default dataset name");
    ensure!(config.concurrency > 0 && config.max_body > 0, "limits must be positive");
    // The default dataset always exists so the dataset list and unprefixed routes work.
    drop(Dataset::open(&config.root, &config.default_dataset, true)?);
    let (bind, max_body) = (config.bind, config.max_body);
    // Created outside Tokio so the blocking client is also dropped outside it.
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let gateway = Arc::new(Gateway { config, client, children: Mutex::new(HashMap::new()) });
    let served = gateway.clone();
    let result = tokio::runtime::Runtime::new()?.block_on(async move {
        let listener = tokio::net::TcpListener::bind(bind).await?;
        eprintln!("gateway listening on http://{}", listener.local_addr()?);
        axum::serve(listener, router(served, max_body))
            .with_graceful_shutdown(async {
                let mut terminate =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("install SIGTERM handler");
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            })
            .await?;
        Ok::<_, anyhow::Error>(())
    });
    gateway.stop_all();
    result
}
