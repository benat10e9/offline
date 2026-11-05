//! `kb` — the command-line front door to the runtime. The desktop app (session 2)
//! calls the same library functions; this CLI is what CI and the content factory use.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use kb_agent::{AskOptions, LlamaServer};
use kb_evidence::QuestionContext;
use kb_packages::{
    build_manifest, parse_public_key, sign_package, verify_package, KeyPair, ManifestInput, SourceRecord, VerifyMode,
};
use kb_retrieval::{build_index, read_passages, Library, PASSAGES_FILE};
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
            let passages = read_passages(&src.join(PASSAGES_FILE))?;
            let known: std::collections::HashSet<&str> = sources.iter().map(|s| s.id.as_str()).collect();
            for p in &passages {
                if !known.contains(p.source_id.as_str()) {
                    bail!("passage {} references unknown source {}", p.passage_id, p.source_id);
                }
            }
            for s in &sources {
                if let Some(n) = &s.superseded_by {
                    if !known.contains(n.as_str()) {
                        eprintln!("note: {} is superseded by {}, which is not in this package", s.id, n);
                    }
                }
            }
            if out.exists() {
                std::fs::remove_dir_all(&out)?;
            }
            std::fs::create_dir_all(&out)?;
            let n = build_index(&out, &passages)?;
            copy_file(&src.join(PASSAGES_FILE), &out.join(PASSAGES_FILE))?;
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
        Cmd::Tier { catalog } => {
            let c = kb_inference::ModelCatalog::load(&catalog)?;
            let t = kb_inference::select_tier(kb_inference::total_ram_gb(), &c);
            println!("{}", serde_json::to_string_pretty(&t)?);
        }
    }
    Ok(())
}
