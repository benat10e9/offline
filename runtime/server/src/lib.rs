//! The application engine plus a small local HTTP server that serves the UI.
//!
//! The desktop shell (Tauri) starts this server in-process and points its window at
//! it. The same server runs headless for browser mode (`kb serve`) and appliances.
//! It binds to 127.0.0.1 unless explicitly told otherwise.

use kb_agent::{AskOptions, LlamaServer};
use kb_evidence::QuestionContext;
use kb_inference::{ModelCatalog, ServerConfig, Supervisor, Tier};
use kb_packages::{parse_public_key, verify_package, SourceRecord, VerifyMode};
use kb_retrieval::Library;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The drive (or install folder) layout. See spec/ssd-layout.md.
#[derive(Debug, Clone)]
pub struct Root {
    pub dir: PathBuf,
}

impl Root {
    /// Paths are made absolute: llama-server runs with its own folder as the working
    /// directory, so relative model and binary paths would resolve to the wrong place.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir: PathBuf = dir.into();
        let dir =
            std::fs::canonicalize(&dir).or_else(|_| std::env::current_dir().map(|cwd| cwd.join(&dir))).unwrap_or(dir);
        Self { dir }
    }
    pub fn packages(&self) -> PathBuf {
        self.dir.join("packages")
    }
    pub fn trust(&self) -> PathBuf {
        self.dir.join("trust")
    }
    pub fn models(&self) -> PathBuf {
        self.dir.join("models")
    }
    pub fn bin(&self) -> PathBuf {
        self.dir.join("bin")
    }
}

/// Find the drive root by walking up from the running executable to the first
/// folder containing both `packages/` and `trust/`. `KB_ROOT` overrides.
pub fn find_root(exe: &std::path::Path) -> Option<Root> {
    if let Ok(r) = std::env::var("KB_ROOT") {
        return Some(Root::new(r));
    }
    let mut dir = exe.parent();
    while let Some(d) = dir {
        if d.join("packages").is_dir() && d.join("trust").is_dir() {
            return Some(Root::new(d));
        }
        dir = d.parent();
    }
    None
}

/// Flags for every llama-server we start: no built-in web UI, and no visible
/// "thinking" (reasoning text is uncited and would fail citation checks).
pub fn llama_server_args() -> Vec<String> {
    ["--no-webui", "--chat-template-kwargs", r#"{"enable_thinking":false}"#, "--reasoning-format", "deepseek"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// A drive provisioned for development carries this marker; it allows models that
/// have not passed safety evaluation, and the UI warns on every answer.
pub const DEV_MARKER: &str = "DEV_ALLOW_UNEVALUATED";

#[derive(Debug, Clone, Serialize)]
pub struct PackageInfo {
    pub id: String,
    pub version: String,
    pub title: String,
    pub passages: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageProblem {
    pub path: String,
    pub error: String,
}

/// What the model is doing, as shown to the user.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelStatus {
    /// Below the RAM floor or no approved model: library search works, answers are not generated.
    SearchOnly {
        reason: String,
    },
    Starting {
        model: String,
    },
    Ready {
        model: String,
        backend: String,
        unevaluated: bool,
    },
    Failed {
        model: String,
        error: String,
    },
}

enum ModelSlot {
    Status(ModelStatus),
    Running { sup: Supervisor, model: String, unevaluated: bool },
}

pub struct Engine {
    pub root: Root,
    pub library: Library,
    pub sources: HashMap<(String, String), SourceRecord>,
    pub packages: Vec<PackageInfo>,
    pub problems: Vec<PackageProblem>,
    pub maps: Vec<MapInfo>,
    pub places: Vec<Place>,
    /// The customer's signed drive profile, if present and valid.
    pub profile: Option<kb_packages::DriveProfile>,
    /// Package manifests' source lists, for the Library view.
    pub library_info: Vec<LibraryPackage>,
    model: Mutex<ModelSlot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryPackage {
    pub id: String,
    pub version: String,
    pub title: String,
    pub sources: Vec<SourceRecord>,
}

/// A map in a package: `maps/map.json` plus the files beside it.
#[derive(Debug, Clone, Serialize)]
pub struct MapInfo {
    pub package_id: String,
    #[serde(skip)]
    pub dir: PathBuf,
    pub meta: serde_json::Value,
    pub routing: bool,
    #[serde(skip)]
    router: Arc<Mutex<RouterSlot>>,
}

#[derive(Default)]
struct RouterSlot {
    router: Option<Arc<kb_routing::Router>>,
    last_used: Option<Instant>,
}

impl std::fmt::Debug for RouterSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouterSlot").field("loaded", &self.router.is_some()).finish()
    }
}

/// A statewide road graph uses about 1.3 GB of RAM; drop it when nobody is navigating.
pub const ROUTER_IDLE_UNLOAD: Duration = Duration::from_secs(600);

impl MapInfo {
    /// Load the road graph on first use (a statewide graph takes several seconds).
    pub fn router(&self) -> Option<Arc<kb_routing::Router>> {
        let mut slot = self.router.lock().ok()?;
        if slot.router.is_none() {
            slot.router = kb_routing::Router::load(&self.dir.join(kb_routing::ROUTING_DIR)).ok().map(Arc::new);
        }
        slot.last_used = Some(Instant::now());
        slot.router.clone()
    }

    pub fn router_loaded(&self) -> bool {
        self.router.lock().map(|s| s.router.is_some()).unwrap_or(false)
    }

    /// Free the graph if unused for `idle`. Returns true if it was unloaded.
    pub fn unload_router_if_idle(&self, idle: Duration) -> bool {
        let Ok(mut slot) = self.router.lock() else { return false };
        if slot.router.is_some() && slot.last_used.is_some_and(|t| t.elapsed() >= idle) {
            slot.router = None;
            return true;
        }
        false
    }
}

/// A searchable place from a map package's gazetteer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Place {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub rank: u32,
    pub lat: f64,
    pub lon: f64,
    #[serde(default)]
    pub context: String,
}

#[derive(Debug, Clone, Default)]
pub struct EngineOptions {
    /// Allow models whose catalog entry is not `safety_approved`. Development only;
    /// the UI shows a warning on every answer while this is on.
    pub allow_unevaluated: bool,
    /// Override the detected RAM (testing and benchmarks).
    pub ram_gb: Option<u32>,
    /// Don't start the model at all.
    pub no_model: bool,
    /// Start this catalog entry regardless of RAM (benchmarks).
    pub force_model: Option<String>,
}

impl Engine {
    /// Verify (signature only) and mount every package under `root/packages`.
    /// Packages that fail verification are skipped and reported, never mounted.
    pub fn open(root: Root) -> anyhow::Result<Self> {
        let keys = std::fs::read_dir(root.trust())
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "pub"))
                    .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                    .filter_map(|s| parse_public_key(&s).ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut library = Library::new();
        let mut sources = HashMap::new();
        let mut packages = Vec::new();
        let mut problems = Vec::new();
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.packages())
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        dirs.sort();
        let mut maps = Vec::new();
        let mut places = Vec::new();
        let mut library_info = Vec::new();
        for dir in dirs {
            let result =
                verify_package(&dir, &keys, VerifyMode::SignatureOnly).map_err(anyhow::Error::from).and_then(|m| {
                    if dir.join(kb_retrieval::INDEX_DIR).is_dir() {
                        library.mount(&m.id, &dir)?;
                    }
                    Ok(m)
                });
            match result {
                Ok(m) => {
                    library_info.push(LibraryPackage {
                        id: m.id.clone(),
                        version: m.version.clone(),
                        title: m.title.clone(),
                        sources: m.sources.clone(),
                    });
                    let map_json = dir.join("maps").join("map.json");
                    if let Ok(bytes) = std::fs::read(&map_json) {
                        if let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                            let routing = dir.join("maps").join(kb_routing::ROUTING_DIR).join("meta.json").is_file();
                            maps.push(MapInfo {
                                package_id: m.id.clone(),
                                dir: dir.join("maps"),
                                meta,
                                routing,
                                router: Arc::new(Mutex::new(RouterSlot::default())),
                            });
                            places.extend(load_places(&dir.join("maps").join("gazetteer.jsonl")));
                        }
                    }
                    for s in &m.sources {
                        sources.insert((m.id.clone(), s.id.clone()), s.clone());
                    }
                    packages.push(PackageInfo { id: m.id, version: m.version, title: m.title, passages: 0 });
                }
                Err(e) => problems.push(PackageProblem { path: dir.display().to_string(), error: e.to_string() }),
            }
        }
        // A profile that fails verification is ignored (and reported), never half-trusted.
        let profile = if root.dir.join(kb_packages::PROFILE_FILE).exists() {
            match kb_packages::read_profile(&root.dir, &keys) {
                Ok(p) => Some(p),
                Err(e) => {
                    problems.push(PackageProblem { path: kb_packages::PROFILE_FILE.into(), error: e.to_string() });
                    None
                }
            }
        } else {
            None
        };
        let stats = library.stats();
        for p in &mut packages {
            p.passages = stats.get(&p.id).copied().unwrap_or(0);
        }
        Ok(Self {
            root,
            library,
            sources,
            packages,
            problems,
            maps,
            places,
            profile,
            library_info,
            model: Mutex::new(ModelSlot::Status(ModelStatus::SearchOnly { reason: "Model not started".into() })),
        })
    }

    /// Decide the tier and start llama-server. Blocking; call from a background thread.
    pub fn start_model(&self, opts: &EngineOptions) {
        if opts.no_model {
            self.set_status(ModelStatus::SearchOnly { reason: "Started in library-only mode".into() });
            return;
        }
        let catalog_path = self.root.models().join("models.json");
        let mut catalog = match ModelCatalog::load(&catalog_path) {
            Ok(c) => c,
            Err(_) => {
                self.set_status(ModelStatus::SearchOnly { reason: "No AI model is installed on this drive".into() });
                return;
            }
        };
        if opts.allow_unevaluated {
            for m in &mut catalog.models {
                m.safety_approved = true;
            }
        }
        let ram = opts.ram_gb.unwrap_or_else(kb_inference::total_ram_gb);
        if let Some(id) = &opts.force_model {
            catalog.models.retain(|m| &m.id == id);
            for m in &mut catalog.models {
                m.min_ram_gb = 0;
            }
            if catalog.models.is_empty() {
                self.set_status(ModelStatus::Failed { model: id.clone(), error: "not in models.json".into() });
                return;
            }
        }
        let model = match kb_inference::select_tier(ram, &catalog) {
            Tier::SearchOnly { ram_gb } => {
                self.set_status(ModelStatus::SearchOnly {
                    reason: format!("This computer has {ram_gb} GB of memory; the AI assistant needs more. Library search works fully."),
                });
                return;
            }
            Tier::Generate { model, .. } => model,
        };
        let unevaluated = opts.allow_unevaluated;
        self.set_status(ModelStatus::Starting { model: model.id.clone() });
        let cfg = ServerConfig {
            model_path: self.root.models().join(&model.file),
            context: model.context,
            gpu_layers: 99,
            startup_timeout: Duration::from_secs(300),
            extra_args: llama_server_args(),
        };
        match Supervisor::start(&self.root.bin(), cfg) {
            Ok(sup) => {
                *self.model.lock().unwrap() = ModelSlot::Running { sup, model: model.id, unevaluated };
            }
            Err(e) => self.set_status(ModelStatus::Failed { model: model.id, error: e.to_string() }),
        }
    }

    /// Stop llama-server. Call on app exit: some shells exit without running destructors.
    pub fn shutdown(&self) {
        if let Ok(mut g) = self.model.lock() {
            *g = ModelSlot::Status(ModelStatus::SearchOnly { reason: "Stopped".into() });
        }
    }

    fn set_status(&self, s: ModelStatus) {
        *self.model.lock().unwrap() = ModelSlot::Status(s);
    }

    pub fn model_status(&self) -> ModelStatus {
        match self.model.try_lock() {
            Ok(g) => match &*g {
                ModelSlot::Status(s) => s.clone(),
                ModelSlot::Running { sup, model, unevaluated } => {
                    ModelStatus::Ready { model: model.clone(), backend: sup.backend.clone(), unevaluated: *unevaluated }
                }
            },
            // Locked = a question is being answered right now.
            Err(_) => ModelStatus::Ready { model: "busy".into(), backend: String::new(), unevaluated: false },
        }
    }

    /// Base URL of a healthy llama-server, restarting it if it died. None = search only.
    pub fn with_model<T>(&self, f: impl FnOnce(&str) -> T) -> Option<(T, bool)> {
        let mut g = self.model.lock().unwrap();
        if let ModelSlot::Running { sup, unevaluated, .. } = &mut *g {
            if sup.ensure_running(2) {
                let url = sup.base_url();
                let flag = *unevaluated;
                return Some((f(&url), flag));
            }
        }
        None
    }

    pub fn search(&self, req: &QuestionRequest) -> anyhow::Result<kb_evidence::Assessment> {
        Ok(kb_agent::search_only(&self.library, &self.sources, &req.question, &req.context(), &AskOptions::default())?)
    }

    pub fn ask(&self, req: &QuestionRequest) -> anyhow::Result<AskResponse> {
        let ctx = req.context();
        let result = self.with_model(|url| {
            let m = LlamaServer::new(url);
            kb_agent::ask(&self.library, &self.sources, &m, &req.question, &ctx, &AskOptions::default())
        });
        match result {
            Some((answer, unevaluated)) => Ok(AskResponse::Answer { answer: answer?, unevaluated_model: unevaluated }),
            None => Ok(AskResponse::SearchOnly { assessment: self.search(req)?, status: self.model_status() }),
        }
    }
}

fn load_places(path: &std::path::Path) -> Vec<Place> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn fold(s: &str) -> String {
    s.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect()
}

impl Engine {
    /// Rank places by how well the name matches, then by kind (cities, hospitals first).
    pub fn search_places(&self, query: &str, limit: usize) -> Vec<Place> {
        let q = fold(query);
        let q = q.split_whitespace().collect::<Vec<_>>().join(" ");
        if q.is_empty() {
            return Vec::new();
        }
        let tokens: Vec<&str> = q.split(' ').collect();
        let mut scored: Vec<(u32, &Place)> = self
            .places
            .iter()
            .filter_map(|p| {
                let name = fold(&p.name);
                let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
                let hay = format!("{name} {} {}", fold(&p.kind), fold(&p.context));
                let base = if name == q {
                    100
                } else if name.starts_with(&q) {
                    60
                } else if tokens.iter().all(|t| hay.split_whitespace().any(|w| w.starts_with(t))) {
                    30
                } else {
                    return None;
                };
                Some((base + p.rank, p))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.len().cmp(&b.1.name.len())));
        scored.into_iter().take(limit).map(|(_, p)| p.clone()).collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuestionRequest {
    pub question: String,
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(default)]
    pub jurisdiction: Option<String>,
}

impl QuestionRequest {
    pub fn context(&self) -> QuestionContext {
        let clean = |s: &Option<String>| s.as_ref().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        QuestionContext { asset: clean(&self.asset).map(|a| a.to_lowercase()), jurisdiction: clean(&self.jurisdiction) }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AskResponse {
    Answer { answer: kb_agent::Answer, unevaluated_model: bool },
    SearchOnly { assessment: kb_evidence::Assessment, status: ModelStatus },
}

/// Bytes used under the root, by top-level area. Used by `kb bench` and the status page.
pub fn footprint(root: &Root) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let Ok(rd) = std::fs::read_dir(&root.dir) else { return out };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let size: u64 = walkdir::WalkDir::new(e.path())
            .into_iter()
            .flatten()
            .filter(|f| f.file_type().is_file())
            .filter_map(|f| f.metadata().ok())
            .map(|m| m.len())
            .sum();
        out.insert(name, size);
    }
    out
}

// ---------- Benchmark ----------

#[derive(Debug, Clone, Serialize)]
pub struct BenchRun {
    pub model: String,
    pub backend: Option<String>,
    pub load_seconds: f64,
    pub status: ModelStatus,
    pub raw: Vec<kb_agent::ChatStats>,
    pub answers: Vec<BenchAnswer>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchAnswer {
    pub question: String,
    pub class: kb_evidence::EvidenceClass,
    pub citations: usize,
    pub seconds: f64,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    pub os: String,
    pub arch: String,
    pub ram_gb: u32,
    pub footprint_bytes: BTreeMap<String, u64>,
    pub packages: Vec<PackageInfo>,
    pub runs: Vec<BenchRun>,
}

/// Fixed prompts measure raw speed; library questions measure the whole pipeline.
pub const BENCH_RAW_PROMPT: &str =
    "In about 150 words, explain why stored water should be kept in clean, sealed containers away from sunlight.";

pub const BENCH_QUESTIONS: &[&str] = &[
    "How do I stop severe bleeding from a leg wound?",
    "How long should I boil water to make it safe to drink?",
    "Can I run a generator in my garage if the door is open?",
];

/// Start each catalog model in turn (or only `only`), measure it, stop it.
pub fn bench(root: &Root, only: Option<&str>, allow_unevaluated: bool) -> anyhow::Result<BenchReport> {
    let ram_gb = kb_inference::total_ram_gb();
    let mut report = BenchReport {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        ram_gb,
        footprint_bytes: footprint(root),
        packages: vec![],
        runs: vec![],
    };
    let catalog = ModelCatalog::load(&root.models().join("models.json")).ok();
    let ids: Vec<String> = match (only, &catalog) {
        (Some(id), _) => vec![id.to_string()],
        (None, Some(c)) => c.models.iter().map(|m| m.id.clone()).collect(),
        (None, None) => vec![],
    };
    for id in ids {
        let engine = Engine::open(root.clone())?;
        report.packages = engine.packages.clone();
        let t = Instant::now();
        let started = Instant::now();
        eprintln!("[{id}] loading model (Intel CPUs: up to a minute)...");
        engine.start_model(&EngineOptions { allow_unevaluated, force_model: Some(id.clone()), ..Default::default() });
        let load_seconds = t.elapsed().as_secs_f64();
        eprintln!("[{id}] model ready in {load_seconds:.1}s. Speed test 1/2...");
        let status = engine.model_status();
        let mut run =
            BenchRun { model: id, backend: None, load_seconds, status: status.clone(), raw: vec![], answers: vec![] };
        if let ModelStatus::Ready { backend, .. } = &status {
            run.backend = Some(backend.clone());
            for i in 0..2 {
                if let Some((Ok(stats), _)) = engine.with_model(|url| {
                    LlamaServer::new(url).chat_with_stats("You are a concise assistant.", BENCH_RAW_PROMPT)
                }) {
                    eprintln!(
                        "[{}] speed test {}/2: {:.1} tokens/s generated ({:.0}s, {:.0}s elapsed)",
                        run.model,
                        i + 1,
                        stats.generated_tokens_per_second.unwrap_or(0.0),
                        stats.wall_seconds,
                        started.elapsed().as_secs_f64()
                    );
                    run.raw.push(stats);
                }
            }
            for (qi, q) in BENCH_QUESTIONS.iter().enumerate() {
                let t = Instant::now();
                let done: f64 = run.answers.iter().map(|a| a.seconds).sum();
                let eta = if qi == 0 {
                    String::new()
                } else {
                    format!(", about {:.0}s left", done / qi as f64 * (BENCH_QUESTIONS.len() - qi) as f64)
                };
                eprintln!(
                    "[{}] question {}/{}: {q} ({:.0}s elapsed{eta})",
                    run.model,
                    qi + 1,
                    BENCH_QUESTIONS.len(),
                    started.elapsed().as_secs_f64()
                );
                let req = QuestionRequest { question: q.to_string(), asset: None, jurisdiction: None };
                if let Ok(AskResponse::Answer { answer, .. }) = engine.ask(&req) {
                    run.answers.push(BenchAnswer {
                        question: q.to_string(),
                        class: answer.class,
                        citations: answer.citations.len(),
                        seconds: t.elapsed().as_secs_f64(),
                        text: answer.text,
                    });
                }
            }
        }
        report.runs.push(run);
        drop(engine); // stops llama-server before the next model
    }
    if report.packages.is_empty() {
        report.packages = Engine::open(root.clone())?.packages;
    }
    Ok(report)
}

// ---------- HTTP ----------

const INDEX_HTML: &str = include_str!("../ui/index.html");
const APP_JS: &str = include_str!("../ui/app.js");
const APP_CSS: &str = include_str!("../ui/app.css");
const VENDOR: &[(&str, &str, &[u8])] = &[
    ("maplibre-gl-csp.js", "text/javascript; charset=utf-8", include_bytes!("../ui/vendor/maplibre-gl-csp.js")),
    (
        "maplibre-gl-csp-worker.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/vendor/maplibre-gl-csp-worker.js"),
    ),
    ("maplibre-gl.css", "text/css; charset=utf-8", include_bytes!("../ui/vendor/maplibre-gl.css")),
    ("pmtiles.js", "text/javascript; charset=utf-8", include_bytes!("../ui/vendor/pmtiles.js")),
    ("basemaps.js", "text/javascript; charset=utf-8", include_bytes!("../ui/vendor/basemaps.js")),
];

pub struct Running {
    pub port: u16,
    pub url: String,
    server: Arc<tiny_http::Server>,
}

impl Running {
    pub fn stop(&self) {
        self.server.unblock();
    }
}

/// Start serving on `bind` (use "127.0.0.1:0" for a free local port). Returns immediately.
pub fn serve(engine: Arc<Engine>, bind: &str) -> anyhow::Result<Running> {
    if engine.maps.iter().any(|m| m.routing) {
        let e = Arc::downgrade(&engine);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(60));
            let Some(engine) = e.upgrade() else { break };
            for m in &engine.maps {
                m.unload_router_if_idle(ROUTER_IDLE_UNLOAD);
            }
        });
    }
    let server = Arc::new(tiny_http::Server::http(bind).map_err(|e| anyhow::anyhow!("bind {bind}: {e}"))?);
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
    let s2 = server.clone();
    std::thread::spawn(move || {
        for req in s2.incoming_requests() {
            let engine = engine.clone();
            std::thread::spawn(move || handle(&engine, req));
        }
    });
    Ok(Running { port, url: format!("http://127.0.0.1:{port}/"), server })
}

fn header(k: &str, v: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("valid header")
}

fn respond(req: tiny_http::Request, code: u16, ctype: &str, body: Vec<u8>) {
    let resp = tiny_http::Response::from_data(body)
        .with_status_code(code)
        .with_header(header("Content-Type", ctype))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"))
        .with_header(header(
            "Content-Security-Policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; \
             connect-src 'self'; worker-src 'self' blob:; child-src 'self' blob:",
        ));
    let _ = req.respond(resp);
}

fn json(req: tiny_http::Request, code: u16, v: &impl Serialize) {
    respond(req, code, "application/json", serde_json::to_vec(v).unwrap_or_default());
}

fn error(req: tiny_http::Request, code: u16, msg: &str) {
    json(req, code, &serde_json::json!({ "error": msg }));
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "json" => "application/json",
        "png" => "image/png",
        "pbf" => "application/x-protobuf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Serve a file under `base` honouring a single `Range: bytes=a-b` header (PMTiles reads by range).
fn serve_file(req: tiny_http::Request, base: &std::path::Path, rel: &str) {
    use std::io::{Seek, SeekFrom};
    let rel = percent_decode(rel);
    let ok = !rel.is_empty()
        && rel.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
        && !rel.contains('\\')
        && !rel.contains(':');
    if !ok {
        return error(req, 400, "Bad path");
    }
    let path = base.join(&rel);
    let Ok(mut f) = std::fs::File::open(&path) else { return error(req, 404, "Not found") };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let range = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .and_then(|h| h.value.as_str().strip_prefix("bytes=").map(|s| s.to_string()));
    let ctype = content_type(&rel);
    let common = vec![
        header("Content-Type", ctype),
        header("Accept-Ranges", "bytes"),
        header("Cache-Control", "max-age=3600"),
        header("X-Content-Type-Options", "nosniff"),
    ];
    if let Some(r) = range {
        let (a, b) = r.split_once('-').unwrap_or(("", ""));
        let (start, end) = match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(s), Ok(e)) => (s, e.min(len.saturating_sub(1))),
            (Ok(s), Err(_)) => (s, len.saturating_sub(1)),
            (Err(_), Ok(n)) => (len.saturating_sub(n), len.saturating_sub(1)),
            _ => return error(req, 416, "Bad range"),
        };
        if start >= len || start > end {
            let resp = tiny_http::Response::empty(416).with_header(header("Content-Range", &format!("bytes */{len}")));
            let _ = req.respond(resp);
            return;
        }
        if f.seek(SeekFrom::Start(start)).is_err() {
            return error(req, 500, "Seek failed");
        }
        let n = end - start + 1;
        let mut headers = common;
        headers.push(header("Content-Range", &format!("bytes {start}-{end}/{len}")));
        let resp = tiny_http::Response::new(tiny_http::StatusCode(206), headers, f.take(n), Some(n as usize), None);
        let _ = req.respond(resp);
    } else {
        let resp = tiny_http::Response::new(tiny_http::StatusCode(200), common, f, Some(len as usize), None);
        let _ = req.respond(resp);
    }
}

fn read_body(req: &mut tiny_http::Request) -> Option<QuestionRequest> {
    let mut s = String::new();
    req.as_reader().take(64 * 1024).read_to_string(&mut s).ok()?;
    let r: QuestionRequest = serde_json::from_str(&s).ok()?;
    (!r.question.trim().is_empty() && r.question.len() <= 2000).then_some(r)
}

fn handle(engine: &Engine, mut req: tiny_http::Request) {
    // Only same-machine pages may call the API (blocks other websites in the browser).
    let host_ok = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Host"))
        .map(|h| {
            let v = h.value.as_str();
            v.starts_with("127.0.0.1:") || v.starts_with("localhost:")
        })
        .unwrap_or(false);
    if !host_ok {
        return error(req, 403, "This server only answers requests from this computer.");
    }
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("/").to_string();
    let method = req.method().clone();
    let started = Instant::now();
    match (method, path.as_str()) {
        (tiny_http::Method::Get, "/") | (tiny_http::Method::Get, "/index.html") => {
            respond(req, 200, "text/html; charset=utf-8", INDEX_HTML.as_bytes().to_vec())
        }
        (tiny_http::Method::Get, "/app.js") => {
            respond(req, 200, "text/javascript; charset=utf-8", APP_JS.as_bytes().to_vec())
        }
        (tiny_http::Method::Get, "/app.css") => {
            respond(req, 200, "text/css; charset=utf-8", APP_CSS.as_bytes().to_vec())
        }
        (tiny_http::Method::Get, p) if p.starts_with("/vendor/") => {
            let name = &p["/vendor/".len()..];
            match VENDOR.iter().find(|(n, _, _)| *n == name) {
                Some((_, ctype, bytes)) => respond(req, 200, ctype, bytes.to_vec()),
                None => error(req, 404, "Not found"),
            }
        }
        (tiny_http::Method::Get, p) if p.starts_with("/maps/") => {
            let rest = &p["/maps/".len()..];
            let (pkg, rel) = rest.split_once('/').unwrap_or((rest, ""));
            match engine.maps.iter().find(|m| m.package_id == pkg) {
                Some(m) => serve_file(req, &m.dir, rel),
                None => error(req, 404, "No such map"),
            }
        }
        (tiny_http::Method::Get, "/api/maps") => json(req, 200, &engine.maps),
        (tiny_http::Method::Get, "/api/profile") => json(req, 200, &engine.profile),
        (tiny_http::Method::Get, "/api/library") => json(req, 200, &engine.library_info),
        (tiny_http::Method::Get, "/api/route") => {
            let point = |k: &str| -> Option<[f64; 2]> {
                let v = query_param(&url, k)?;
                let (a, b) = v.split_once(',')?;
                let (lat, lon) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
                ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some([lat, lon])
            };
            let (Some(from), Some(to)) = (point("from"), point("to")) else {
                return error(req, 400, "Give ?from=lat,lon&to=lat,lon");
            };
            let router = engine.maps.iter().filter(|m| m.routing).find_map(|m| m.router());
            match router {
                None => error(req, 404, "No routing data is installed on this drive."),
                Some(r) => match r.route(from, to) {
                    Some(route) => json(req, 200, &serde_json::json!({ "route": route, "notes": r.meta.notes })),
                    None => error(
                        req,
                        404,
                        "No road route found between these points (one may be more than 5 km from a road).",
                    ),
                },
            }
        }
        (tiny_http::Method::Get, "/api/places") => {
            let q = query_param(&url, "q").unwrap_or_default();
            let limit = query_param(&url, "limit").and_then(|l| l.parse().ok()).unwrap_or(12usize).min(50);
            json(req, 200, &engine.search_places(&q, limit))
        }
        (tiny_http::Method::Get, "/api/status") => json(
            req,
            200,
            &serde_json::json!({
                "packages": engine.packages,
                "problems": engine.problems,
                "maps": engine.maps.len(),
                "places": engine.places.len(),
                "model": engine.model_status(),
            }),
        ),
        (tiny_http::Method::Post, "/api/search") => match read_body(&mut req) {
            Some(q) => match engine.search(&q) {
                Ok(a) => json(req, 200, &a),
                Err(e) => error(req, 500, &e.to_string()),
            },
            None => error(req, 400, "Send JSON with a non-empty \"question\" under 2,000 characters."),
        },
        (tiny_http::Method::Post, "/api/ask") => match read_body(&mut req) {
            Some(q) => match engine.ask(&q) {
                Ok(a) => {
                    let mut v = serde_json::to_value(&a).unwrap_or_default();
                    v["elapsed_ms"] = serde_json::json!(started.elapsed().as_millis() as u64);
                    json(req, 200, &v)
                }
                Err(e) => error(req, 500, &e.to_string()),
            },
            None => error(req, 400, "Send JSON with a non-empty \"question\" under 2,000 characters."),
        },
        (tiny_http::Method::Get, "/api/document") => {
            let (Some(pkg), Some(doc)) = (query_param(&url, "package"), query_param(&url, "doc")) else {
                return error(req, 400, "Give ?package=…&doc=…");
            };
            match engine.library.document(&pkg, &doc) {
                Ok(passages) => {
                    let source =
                        passages.first().and_then(|p| engine.sources.get(&(pkg.clone(), p.source_id.clone()))).cloned();
                    json(req, 200, &serde_json::json!({ "passages": passages, "source": source }))
                }
                Err(e) => error(req, 404, &e.to_string()),
            }
        }
        _ => error(req, 404, "Not found"),
    }
}

pub mod eval;

#[cfg(test)]
mod tests;
