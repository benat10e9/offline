use super::*;
use kb_packages::{build_manifest, sign_package, KeyPair, ManifestInput};
use std::path::Path;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/acme-demo")
}

/// A drive layout with the fixture package signed by a trusted key, no model.
fn make_root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("packages/fixture-acme-demo@0.1.0");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::create_dir_all(dir.path().join("trust")).unwrap();
    let passages = kb_retrieval::read_passages(&fixture().join("passages.jsonl")).unwrap();
    kb_retrieval::build_index(&pkg, &passages).unwrap();
    std::fs::copy(fixture().join("passages.jsonl"), pkg.join("passages.jsonl")).unwrap();
    let sources: Vec<SourceRecord> =
        serde_json::from_slice(&std::fs::read(fixture().join("sources.json")).unwrap()).unwrap();
    let key = KeyPair::generate().unwrap();
    let m = build_manifest(
        &pkg,
        ManifestInput {
            id: "fixture-acme-demo".into(),
            version: "0.1.0".into(),
            title: "Fixture".into(),
            editions: vec!["consumer".into()],
            depends_on: vec![],
            sources,
        },
    )
    .unwrap();
    sign_package(&pkg, &m, &key).unwrap();
    std::fs::write(dir.path().join("trust/dev.pub"), key.public_hex()).unwrap();
    dir
}

fn start(root: &Path) -> (Arc<Engine>, Running) {
    let engine = Arc::new(Engine::open(Root::new(root)).unwrap());
    engine.start_model(&EngineOptions::default());
    let running = serve(engine.clone(), "127.0.0.1:0").unwrap();
    (engine, running)
}

fn get(url: &str) -> serde_json::Value {
    ureq::get(url).call().unwrap().into_json().unwrap()
}

fn post(url: &str, body: serde_json::Value) -> Result<serde_json::Value, u16> {
    match ureq::post(url).send_json(body) {
        Ok(r) => Ok(r.into_json().unwrap()),
        Err(ureq::Error::Status(code, _)) => Err(code),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn engine_mounts_verified_packages_and_reports_search_only_without_model() {
    let root = make_root();
    let (engine, _r) = start(root.path());
    assert_eq!(engine.packages.len(), 1);
    assert_eq!(engine.packages[0].passages, 9);
    assert!(engine.problems.is_empty());
    assert!(matches!(engine.model_status(), ModelStatus::SearchOnly { .. }));
}

#[test]
fn tampered_package_is_not_mounted() {
    let root = make_root();
    let m = root.path().join("packages/fixture-acme-demo@0.1.0/manifest.json");
    let s = std::fs::read_to_string(&m).unwrap().replace("\"Fixture\"", "\"Tampered\"");
    std::fs::write(&m, s).unwrap();
    let engine = Engine::open(Root::new(root.path())).unwrap();
    assert!(engine.packages.is_empty());
    assert_eq!(engine.problems.len(), 1);
}

#[test]
fn http_endpoints() {
    let root = make_root();
    let (_e, r) = start(root.path());
    let base = r.url.trim_end_matches('/').to_string();

    let html = ureq::get(&format!("{base}/")).call().unwrap();
    assert!(html.header("Content-Security-Policy").unwrap().contains("default-src 'self'"));
    assert!(html.into_string().unwrap().contains("Offline Library"));

    let status = get(&format!("{base}/api/status"));
    assert_eq!(status["packages"][0]["id"], "fixture-acme-demo");
    assert_eq!(status["model"]["state"], "search_only");

    let s = post(
        &format!("{base}/api/search"),
        serde_json::json!({"question": "How often should I change the engine oil?", "asset": "Acme:GX-2000"}),
    )
    .unwrap();
    assert_eq!(s["class"], "verified", "asset is normalised to lowercase: {s}");

    // Ask with no model falls back to library search and says why.
    let a = post(&format!("{base}/api/ask"), serde_json::json!({"question": "Can I run a generator in the garage?"}))
        .unwrap();
    assert_eq!(a["mode"], "search_only");
    assert_eq!(a["assessment"]["class"], "verified");

    let d = get(&format!("{base}/api/document?package=fixture-acme-demo&doc=acme-gx2000-revB"));
    assert_eq!(d["passages"].as_array().unwrap().len(), 2);
    assert_eq!(d["source"]["revision"], "Rev B");

    assert_eq!(post(&format!("{base}/api/ask"), serde_json::json!({"question": "  "})).unwrap_err(), 400);
    r.stop();
}

#[test]
fn rejects_requests_from_other_hosts() {
    let root = make_root();
    let (_e, r) = start(root.path());
    let err = ureq::get(&format!("{}api/status", r.url)).set("Host", "evil.example").call().unwrap_err();
    assert!(matches!(err, ureq::Error::Status(403, _)));
}

#[test]
fn footprint_counts_areas() {
    let root = make_root();
    let f = footprint(&Root::new(root.path()));
    assert!(f["packages"] > 0);
    assert!(f.contains_key("trust"));
}

#[test]
fn percent_decoding() {
    assert_eq!(percent_decode("a%20b+c%2Fd"), "a b c/d");
    assert_eq!(percent_decode("bad%zz%"), "bad%zz%");
}

#[test]
fn finds_root_from_nested_executable() {
    let root = make_root();
    let exe = root.path().join("Offline Library.app/Contents/MacOS/offline-library");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    if std::env::var("KB_ROOT").is_err() {
        assert_eq!(find_root(&exe).unwrap().dir, root.path());
        assert!(find_root(Path::new("/nonexistent/x/y")).is_none());
    }
}
