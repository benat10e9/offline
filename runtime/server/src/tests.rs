use super::*;
use kb_packages::{build_manifest, sign_package, KeyPair, ManifestInput};
use std::path::Path;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/acme-demo")
}

/// A drive layout with the fixture package signed by a trusted key, no model.
pub(crate) fn make_root() -> tempfile::TempDir {
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

/// Adds a signed map package (fake tiles file, three places) to a root made by make_root().
fn add_map_package(root: &Path) {
    let pkg = root.join("packages/maps-test@0.1.0");
    let maps = pkg.join("maps");
    std::fs::create_dir_all(maps.join("assets/fonts/Noto Sans Regular")).unwrap();
    let tiles: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(maps.join("test.pmtiles"), &tiles).unwrap();
    std::fs::write(maps.join("assets/fonts/Noto Sans Regular/0-255.pbf"), b"glyphs").unwrap();
    std::fs::write(maps.join("map.json"), r#"{"region":"test","tiles":"test.pmtiles","bounds":[-98,30,-97,31]}"#)
        .unwrap();
    std::fs::write(
        maps.join("gazetteer.jsonl"),
        [
            r#"{"name":"Austin","kind":"city","rank":10,"lat":30.27,"lon":-97.74,"context":""}"#,
            r#"{"name":"Austin State Hospital","kind":"hospital","rank":9,"lat":30.31,"lon":-97.73,"context":"Austin"}"#,
            r#"{"name":"Round Rock Fire Station 1","kind":"fire station","rank":7,"lat":30.5,"lon":-97.67,"context":"Round Rock"}"#,
        ]
        .join("\n"),
    )
    .unwrap();
    let key = KeyPair::generate().unwrap();
    let m = build_manifest(
        &pkg,
        ManifestInput {
            id: "maps-test".into(),
            version: "0.1.0".into(),
            title: "Test map".into(),
            editions: vec!["consumer".into()],
            depends_on: vec![],
            sources: vec![],
        },
    )
    .unwrap();
    sign_package(&pkg, &m, &key).unwrap();
    std::fs::write(root.join("trust/maps.pub"), key.public_hex()).unwrap();
}

#[test]
fn map_packages_places_and_ranges() {
    let root = make_root();
    add_map_package(root.path());
    let (engine, r) = start(root.path());
    assert_eq!(engine.maps.len(), 1);
    assert_eq!(engine.packages.len(), 2, "text package still mounts beside the map package");

    let names: Vec<String> = engine.search_places("austin", 5).into_iter().map(|p| p.name).collect();
    assert_eq!(names[0], "Austin", "exact city match ranks first: {names:?}");
    assert!(names.contains(&"Austin State Hospital".to_string()));
    let names: Vec<String> = engine.search_places("fire round", 5).into_iter().map(|p| p.name).collect();
    assert_eq!(names, vec!["Round Rock Fire Station 1"]);
    assert!(engine.search_places("  ", 5).is_empty());

    let base = r.url.trim_end_matches('/').to_string();
    let maps = get(&format!("{base}/api/maps"));
    assert_eq!(maps[0]["package_id"], "maps-test");
    let places = get(&format!("{base}/api/places?q=hospital"));
    assert_eq!(places[0]["name"], "Austin State Hospital");

    let resp = ureq::get(&format!("{base}/maps/maps-test/test.pmtiles")).set("Range", "bytes=10-19").call().unwrap();
    assert_eq!(resp.status(), 206);
    assert_eq!(resp.header("Content-Range").unwrap(), "bytes 10-19/1000");
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_reader(), &mut body).unwrap();
    assert_eq!(body, (10..20u32).map(|i| (i % 251) as u8).collect::<Vec<_>>());

    let full = ureq::get(&format!("{base}/maps/maps-test/test.pmtiles")).call().unwrap();
    assert_eq!(full.status(), 200);
    let glyphs =
        ureq::get(&format!("{base}/maps/maps-test/assets/fonts/Noto%20Sans%20Regular/0-255.pbf")).call().unwrap();
    assert_eq!(glyphs.into_string().unwrap(), "glyphs");

    for bad in ["../manifest.json", "assets/../../manifest.json", "%2e%2e/manifest.json"] {
        let e = ureq::get(&format!("{base}/maps/maps-test/{bad}")).call().unwrap_err();
        assert!(matches!(e, ureq::Error::Status(400 | 404, _)), "{bad} must be refused");
    }
    assert!(matches!(
        ureq::get(&format!("{base}/maps/maps-test/test.pmtiles")).set("Range", "bytes=5000-6000").call().unwrap_err(),
        ureq::Error::Status(416, _)
    ));
    let js = ureq::get(&format!("{base}/vendor/maplibre-gl-csp.js")).call().unwrap();
    assert!(js.header("Content-Type").unwrap().starts_with("text/javascript"));
    r.stop();
}
