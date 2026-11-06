//! `kb` — the command-line front door to the runtime. The desktop app (session 2)
//! calls the same library functions; this CLI is what CI and the content factory use.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use kb_agent::{AskOptions, LlamaServer};
use kb_evidence::QuestionContext;
use kb_packages::{
    build_manifest, parse_public_key, sign_package, verify_package, KeyPair, ManifestInput, SourceRecord, VerifyMode,
};
use kb_retrieval::{Library, PASSAGES_FILE};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Parser)]
#[command(name = "kb", version, about = "Offline knowledge packages: build, sign, verify, search, ask")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate an Ed25519 publisher key pair (<name>.key secret, <name>.pub public).
    Keygen {
        #[arg(long)]
        out: PathBuf,
    },
    /// Build a package directory from a factory source folder and sign it.
    Build {
        /// Folder with package.json, sources.json and passages.jsonl.
        #[arg(long)]
        src: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Secret key file from `kb keygen`.
        #[arg(long)]
        key: PathBuf,
    },
    /// Verify a package against trusted public keys.
    Verify {
        #[arg(long = "pkg", required = true)]
        pkgs: Vec<PathBuf>,
        #[arg(long = "trust", required = true)]
        trust: Vec<PathBuf>,
        /// Hash every file, not just the signature.
        #[arg(long)]
        full: bool,
    },
    /// Search mounted packages and print the evidence assessment as JSON.
    Search {
        #[command(flatten)]
        lib: LibArgs,
        query: String,
    },
    /// Ask a question; requires a running llama-server (--model-url) or --bin-root + --model.
    Ask {
        #[command(flatten)]
        lib: LibArgs,
        #[arg(long)]
        model_url: Option<String>,
        #[arg(long)]
        bin_root: Option<PathBuf>,
        #[arg(long)]
        model: Option<PathBuf>,
        question: String,
    },
    /// Run the app engine and web UI from a drive or install folder (browser mode).
    Serve {
        /// Root of the drive layout (packages/, trust/, models/, bin/).
        #[arg(long)]
        root: PathBuf,
        #[arg(long, default_value_t = 8765)]
        port: u16,
        /// Development only: allow models that have not passed safety evaluation.
        #[arg(long)]
        allow_unevaluated: bool,
        /// Library search only; don't start the model.
        #[arg(long)]
        no_model: bool,
    },
    /// Measure disk footprint, model load time and speed; writes bench-report.json.
    Bench {
        #[arg(long)]
        root: PathBuf,
        /// Only this models.json id (default: every model in the catalog).
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        allow_unevaluated: bool,
        #[arg(long, default_value = "bench-report.json")]
        out: PathBuf,
    },
    /// Grade the installed library (and optionally the model) against a question set.
    Eval {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        questions: PathBuf,
        /// Also generate and grade answers (needs a model on the drive).
        #[arg(long)]
        with_model: bool,
        #[arg(long)]
        allow_unevaluated: bool,
        #[arg(long, default_value = "eval-report.json")]
        out: PathBuf,
        /// Exit non-zero if any out-of-scope question is marked verified or any safety pattern matches.
        #[arg(long)]
        strict: bool,
        /// Only the first N questions (quick runs on slow hardware).
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Build offline routing files from an OpenStreetMap extract (.osm.pbf).
    RouteBuild {
        #[arg(long)]
        pbf: PathBuf,
        /// Output folder, normally <map package src>/maps/routing
        #[arg(long)]
        out: PathBuf,
    },
    /// Load routing files and route between two lat,lon points (timing + sanity check).
    Route {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
    },
    /// Sign a customer drive profile (profile.json) into <out>/profile.json + profile.sig.
    ProfileSign {
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Check a finished drive: packages (full hash), profile signature, model checksums,
    /// runtimes for every OS in the profile. Exit 0 only if everything is intact.
    VerifyDrive {
        #[arg(long)]
        root: PathBuf,
    },
    /// Print this machine's model tier from a models.json catalog.
    Tier {
        #[arg(long)]
        catalog: PathBuf,
    },
}

#[derive(clap::Args)]
struct LibArgs {
    #[arg(long = "pkg", required = true)]
    pkgs: Vec<PathBuf>,
    #[arg(long = "trust", required = true)]
    trust: Vec<PathBuf>,
    /// Equipment the question concerns, as make:model (from the asset registry).
    #[arg(long)]
    asset: Option<String>,
    /// Jurisdiction code, e.g. US-TX.
    #[arg(long)]
    jurisdiction: Option<String>,
}

#[derive(Deserialize)]
struct PackageMeta {
    id: String,
    version: String,
    title: String,
    editions: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
}

type Sources = HashMap<(String, String), SourceRecord>;

fn open_library(args: &LibArgs) -> Result<(Library, Sources)> {
    let keys = args
        .trust
        .iter()
        .map(|p| {
            let s = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            parse_public_key(&s).with_context(|| format!("parsing {}", p.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut lib = Library::new();
    let mut sources = Sources::new();
    for dir in &args.pkgs {
        let m = verify_package(dir, &keys, VerifyMode::SignatureOnly)
            .with_context(|| format!("verifying {}", dir.display()))?;
        lib.mount(&m.id, dir)?;
        for s in m.sources {
            sources.insert((m.id.clone(), s.id.clone()), s);
        }
    }
    Ok((lib, sources))
}

fn ctx(args: &LibArgs) -> QuestionContext {
    QuestionContext { asset: args.asset.clone(), jurisdiction: args.jurisdiction.clone() }
}

fn copy_file(from: &Path, to: &Path) -> Result<()> {
    std::fs::copy(from, to).with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
    Ok(())
}

fn verify_drive(root: &Path) -> Result<serde_json::Value> {
    let mut problems: Vec<String> = Vec::new();
    let mut ok: Vec<String> = Vec::new();
    let keys: Vec<_> = std::fs::read_dir(root.join("trust"))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "pub"))
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .filter_map(|s| parse_public_key(&s).ok())
                .collect()
        })
        .unwrap_or_default();
    if keys.is_empty() {
        problems.push("no trusted keys in trust/".into());
    }
    let profile = match kb_packages::read_profile(root, &keys) {
        Ok(p) => {
            ok.push(format!("profile {} v{}", p.profile_id, p.version));
            Some(p)
        }
        Err(e) => {
            problems.push(format!("profile: {e}"));
            None
        }
    };
    let mut installed = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root.join("packages")) {
        for e in rd.flatten().filter(|e| e.path().is_dir()) {
            match verify_package(&e.path(), &keys, VerifyMode::Full) {
                Ok(m) => {
                    ok.push(format!("package {}@{}", m.id, m.version));
                    installed.push(m.id);
                }
                Err(err) => problems.push(format!("package {}: {err}", e.path().display())),
            }
        }
    }
    if let Some(p) = &profile {
        for want in &p.packages {
            if !installed.contains(want) {
                problems.push(format!("profile lists package {want}, which is not installed"));
            }
        }
        for os in &p.os {
            let plat = match os.as_str() {
                "macos" => vec!["macos-aarch64", "macos-x86_64"],
                "windows" => vec!["windows-x86_64"],
                _ => vec!["linux-x86_64"],
            };
            let has = plat.iter().any(|d| {
                std::fs::read_dir(root.join("bin").join(d))
                    .map(|rd| rd.flatten().any(|b| b.path().join(kb_inference::binary_name_for(os)).is_file()))
                    .unwrap_or(false)
            });
            if has {
                ok.push(format!("runtime for {os}"));
            } else {
                problems.push(format!("no llama-server runtime for {os}"));
            }
        }
    }
    match kb_inference::ModelCatalog::load(&root.join("models").join("models.json")) {
        Ok(cat) => {
            for m in &cat.models {
                let path = root.join("models").join(&m.file);
                match (path.is_file(), &m.sha256) {
                    (false, _) => problems.push(format!("model file missing: {}", m.file)),
                    (true, Some(want)) => match kb_packages::sha256_file(&path) {
                        Ok((got, _)) if &got == want => ok.push(format!("model {} checksum", m.id)),
                        Ok(_) => problems.push(format!("model {} checksum mismatch", m.id)),
                        Err(e) => problems.push(format!("model {}: {e}", m.id)),
                    },
                    (true, None) => problems.push(format!("model {} has no recorded checksum", m.id)),
                }
            }
            if let Some(p) = &profile {
                if !cat.models.iter().any(|m| m.id == p.model) {
                    problems.push(format!("profile model {} not in models.json", p.model));
                }
            }
        }
        Err(_) => problems.push("models/models.json missing or invalid".into()),
    }
    Ok(serde_json::json!({ "ok": ok, "problems": problems }))
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            copy_file(&entry.path(), &dest)?;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Keygen { out } => {
            let k = KeyPair::generate()?;
            let key = out.with_extension("key");
            let pubf = out.with_extension("pub");
            if key.exists() {
                bail!("{} exists; refusing to overwrite a key", key.display());
            }
            std::fs::write(&key, k.secret_hex() + "\n")?;
            std::fs::write(&pubf, k.public_hex() + "\n")?;
            println!("secret: {}\npublic: {}", key.display(), pubf.display());
        }
        Cmd::Build { src, out, key } => {
            let meta: PackageMeta =
                serde_json::from_slice(&std::fs::read(src.join("package.json"))?).context("package.json")?;
            let sources: Vec<SourceRecord> =
                serde_json::from_slice(&std::fs::read(src.join("sources.json"))?).context("sources.json")?;
            let has_passages = src.join(PASSAGES_FILE).is_file();
            let known: std::collections::HashSet<String> = sources.iter().map(|s| s.id.clone()).collect();
            for s in &sources {
                if let Some(n) = &s.superseded_by {
                    if !known.contains(n) {
                        eprintln!("note: {} is superseded by {}, which is not in this package", s.id, n);
                    }
                }
            }
            if out.exists() {
                std::fs::remove_dir_all(&out)?;
            }
            std::fs::create_dir_all(&out)?;
            let n = if has_passages {
                let n = kb_retrieval::build_index_streaming(&out, &src.join(PASSAGES_FILE), |p| {
                    if known.contains(&p.source_id) {
                        Ok(())
                    } else {
                        Err(format!("passage {} references unknown source {}", p.passage_id, p.source_id))
                    }
                })?;
                // The text lives in the index's stored fields; passages.jsonl stays in the
                // build folder and is not shipped (it doubled package size).
                n
            } else {
                0
            };
            // Asset folders (maps/, etc.) are copied as-is and covered by the signed manifest.
            for entry in std::fs::read_dir(&src)? {
                let entry = entry?;
                let name = entry.file_name();
                if entry.file_type()?.is_dir() && name != "index" {
                    copy_dir(&entry.path(), &out.join(&name))?;
                }
            }
            if !has_passages && !out.join("maps").is_dir() {
                bail!("{} has neither passages.jsonl nor a maps/ folder", src.display());
            }
            let k = KeyPair::from_secret_hex(&std::fs::read_to_string(&key)?)?;
            let manifest = build_manifest(
                &out,
                ManifestInput {
                    id: meta.id,
                    version: meta.version,
                    title: meta.title,
                    editions: meta.editions,
                    depends_on: meta.depends_on,
                    sources,
                },
            )?;
            sign_package(&out, &manifest, &k)?;
            println!(
                "built {}@{}: {} passages, {} files, signed by {}",
                manifest.id,
                manifest.version,
                n,
                manifest.files.len(),
                k.public_hex()
            );
        }
        Cmd::Verify { pkgs, trust, full } => {
            let keys = trust
                .iter()
                .map(|p| Ok(parse_public_key(&std::fs::read_to_string(p)?)?))
                .collect::<Result<Vec<_>>>()?;
            let mode = if full { VerifyMode::Full } else { VerifyMode::SignatureOnly };
            let mut failed = false;
            for p in pkgs {
                match verify_package(&p, &keys, mode) {
                    Ok(m) => println!("OK   {}@{} ({} files)", m.id, m.version, m.files.len()),
                    Err(e) => {
                        failed = true;
                        println!("FAIL {}: {e}", p.display());
                    }
                }
            }
            if failed {
                std::process::exit(2);
            }
        }
        Cmd::Search { lib, query } => {
            let (library, sources) = open_library(&lib)?;
            let a = kb_agent::search_only(&library, &sources, &query, &ctx(&lib), &AskOptions::default())?;
            println!("{}", serde_json::to_string_pretty(&a)?);
        }
        Cmd::Ask { lib, model_url, bin_root, model, question } => {
            let (library, sources) = open_library(&lib)?;
            let _guard;
            let url = match (model_url, bin_root, model) {
                (Some(u), _, _) => u,
                (None, Some(root), Some(model)) => {
                    let sup = kb_inference::Supervisor::start(
                        &root,
                        kb_inference::ServerConfig {
                            model_path: model,
                            context: 8192,
                            gpu_layers: 99,
                            startup_timeout: Duration::from_secs(180),
                            extra_args: vec![],
                        },
                    )?;
                    eprintln!("llama-server ({}) on {}", sup.backend, sup.base_url());
                    let u = sup.base_url();
                    _guard = sup;
                    u
                }
                _ => bail!("give --model-url, or --bin-root and --model"),
            };
            let m = LlamaServer::new(url);
            let a = kb_agent::ask(&library, &sources, &m, &question, &ctx(&lib), &AskOptions::default())?;
            println!("{}", serde_json::to_string_pretty(&a)?);
        }
        Cmd::Serve { root, port, allow_unevaluated, no_model } => {
            let data_dir = kb_server::chat::default_data_dir();
            let engine = std::sync::Arc::new(
                kb_server::Engine::open(kb_server::Root::new(&root))?
                    .with_chat(kb_server::chat::ChatStore::open(&data_dir)?),
            );
            eprintln!("conversations: {}", data_dir.display());
            for p in &engine.problems {
                eprintln!("NOT MOUNTED {}: {}", p.path, p.error);
            }
            eprintln!("library: {} package(s)", engine.packages.len());
            let e2 = engine.clone();
            std::thread::spawn(move || {
                e2.start_model(&kb_server::EngineOptions { allow_unevaluated, no_model, ..Default::default() });
                eprintln!("assistant: {:?}", e2.model_status());
            });
            let running = kb_server::serve(engine, &format!("127.0.0.1:{port}"))?;
            println!("Open {}", running.url);
            loop {
                std::thread::park();
            }
        }
        Cmd::Bench { root, model, allow_unevaluated, out } => {
            let r = kb_server::bench(&kb_server::Root::new(&root), model.as_deref(), allow_unevaluated)?;
            std::fs::write(&out, serde_json::to_vec_pretty(&r)?)?;
            println!("{} {} with {} GB RAM", r.os, r.arch, r.ram_gb);
            println!("disk footprint:");
            let mut total = 0;
            for (k, v) in &r.footprint_bytes {
                total += v;
                println!("  {k:<12} {:>9.1} MB", *v as f64 / 1e6);
            }
            println!("  {:<12} {:>9.1} MB", "total", total as f64 / 1e6);
            for run in &r.runs {
                println!("\nmodel {} ({:?})", run.model, run.backend);
                println!("  load: {:.1}s  status: {:?}", run.load_seconds, run.status);
                for s in &run.raw {
                    println!(
                        "  raw: {:.1} tok/s generated, {:.1} tok/s prompt, {:.1}s wall",
                        s.generated_tokens_per_second.unwrap_or(0.0),
                        s.prompt_tokens_per_second.unwrap_or(0.0),
                        s.wall_seconds
                    );
                }
                for a in &run.answers {
                    println!("  {:>5.1}s {:?} {} cites  {}", a.seconds, a.class, a.citations, a.question);
                }
            }
            println!("\nfull report: {}", out.display());
            if r.runs.iter().any(|run| !matches!(run.status, kb_server::ModelStatus::Ready { .. })) {
                bail!("a model failed to start; see the report");
            }
        }
        Cmd::Eval { root, questions, with_model, allow_unevaluated, out, strict, limit } => {
            let engine = kb_server::Engine::open(kb_server::Root::new(&root))?;
            if with_model {
                engine.start_model(&kb_server::EngineOptions { allow_unevaluated, ..Default::default() });
                let status = engine.model_status();
                eprintln!("assistant: {status:?}");
                // Never grade search results as if they were model answers.
                if !matches!(status, kb_server::ModelStatus::Ready { .. }) {
                    bail!("--with-model was given but the model is not running: {status:?}");
                }
            }
            let mut items = kb_server::eval::load_items(&questions)?;
            if let Some(n) = limit {
                items.truncate(n);
            }
            let total = items.len();
            // Write the report after every question: a timeout still leaves usable results.
            let r = kb_server::eval::run_with_progress(&engine, &items, with_model, &mut |x, so_far| {
                eprintln!(
                    "[{}/{}] {:.0}s {:?} {} cites={:?}",
                    so_far.len(),
                    total,
                    x.seconds,
                    x.class,
                    x.id,
                    x.citations
                );
                let partial = kb_server::eval::summarize(so_far.to_vec(), &items);
                let _ = std::fs::write(&out, serde_json::to_vec_pretty(&partial).unwrap_or_default());
            })?;
            std::fs::write(&out, serde_json::to_vec_pretty(&r)?)?;
            for x in &r.results {
                let src = match x.source_hit {
                    Some(true) => "src ok ",
                    Some(false) => "SRC MISS",
                    None => "        ",
                };
                println!(
                    "{} {} {:<11} {:<14} {}{}",
                    if x.class_ok { "ok  " } else { "FAIL" },
                    src,
                    format!("{:?}", x.class).to_lowercase(),
                    x.id,
                    x.top_sources.iter().take(3).cloned().collect::<Vec<_>>().join(","),
                    if x.violations.is_empty() { String::new() } else { format!("  UNSAFE: {:?}", x.violations) }
                );
                if !x.missing.is_empty() {
                    println!("      INCOMPLETE, missing: {:?}", x.missing);
                }
            }
            let s = &r.summary;
            println!(
                "\nclass ok {}/{}  sources {}/{}  out-of-scope wrongly verified {}  withheld {}/{}  unsafe {}  incomplete {}  mean {:.2}s",
                s.class_ok, s.items, s.source_hits, s.source_expectations, s.out_of_scope_wrongly_verified,
                s.answers_withheld, s.model_graded, s.safety_violations, s.incomplete_answers, s.mean_seconds
            );
            if strict && (s.out_of_scope_wrongly_verified > 0 || s.safety_violations > 0) {
                std::process::exit(3);
            }
        }
        Cmd::RouteBuild { pbf, out } => {
            let t = std::time::Instant::now();
            let meta = kb_routing::build_from_pbf(&pbf, &out)?;
            println!("{}", serde_json::to_string_pretty(&meta)?);
            println!("built in {:.0}s", t.elapsed().as_secs_f64());
        }
        Cmd::Route { dir, from, to } => {
            let parse = |s: &str| -> Result<[f64; 2]> {
                let (a, b) = s.split_once(',').context("use lat,lon")?;
                Ok([a.trim().parse()?, b.trim().parse()?])
            };
            let t = std::time::Instant::now();
            let r = kb_routing::Router::load(&dir)?;
            let load = t.elapsed().as_secs_f64();
            let t = std::time::Instant::now();
            let route = r.route(parse(&from)?, parse(&to)?).context("no route found")?;
            println!(
                "load {:.1}s, query {:.0} ms: {:.1} mi, {:.0} min, {} points, snapped {:.0} m / {:.0} m",
                load,
                t.elapsed().as_secs_f64() * 1000.0,
                route.distance_m / 1609.344,
                route.duration_s / 60.0,
                route.coordinates.len(),
                route.snap_from_m,
                route.snap_to_m
            );
        }
        Cmd::ProfileSign { profile, key, out } => {
            let p: kb_packages::DriveProfile =
                serde_json::from_slice(&std::fs::read(&profile)?).context("profile is not a valid drive profile")?;
            let bytes = serde_json::to_vec_pretty(&p)?;
            let k = KeyPair::from_secret_hex(&std::fs::read_to_string(&key)?)?;
            std::fs::create_dir_all(&out)?;
            std::fs::write(out.join(kb_packages::PROFILE_FILE), &bytes)?;
            std::fs::write(out.join(kb_packages::PROFILE_SIG), kb_packages::signature_line(&bytes, &k))?;
            println!(
                "signed profile {} v{} ({} packages, model {})",
                p.profile_id,
                p.version,
                p.packages.len(),
                p.model
            );
        }
        Cmd::VerifyDrive { root } => {
            let report = verify_drive(&root)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report["problems"].as_array().is_none_or(|a| a.is_empty()) {
                std::process::exit(2);
            }
        }
        Cmd::Tier { catalog } => {
            let c = kb_inference::ModelCatalog::load(&catalog)?;
            let t = kb_inference::select_tier(kb_inference::total_ram_gb(), &c);
            println!("{}", serde_json::to_string_pretty(&t)?);
        }
    }
    Ok(())
}
