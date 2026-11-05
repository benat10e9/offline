//! End-to-end: keygen → build → verify → tamper detection → evidence scenarios,
//! all through the real `kb` binary and the fictional fixture corpus.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn kb() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kb"))
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/acme-demo")
}

fn run(cmd: &mut Command) -> (bool, String) {
    let out = cmd.output().expect("kb runs");
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), text)
}

struct Env {
    _dir: tempfile::TempDir,
    pkg: PathBuf,
    pubkey: PathBuf,
}

fn setup() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let keybase = dir.path().join("publisher");
    let (ok, t) = run(kb().args(["keygen", "--out"]).arg(&keybase));
    assert!(ok, "{t}");
    let pkg = dir.path().join("pkg");
    let (ok, t) = run(kb()
        .args(["build", "--src"])
        .arg(fixture())
        .arg("--out")
        .arg(&pkg)
        .arg("--key")
        .arg(keybase.with_extension("key")));
    assert!(ok, "{t}");
    Env { pubkey: keybase.with_extension("pub"), pkg, _dir: dir }
}

fn search(env: &Env, q: &str, extra: &[&str]) -> Value {
    let mut c = kb();
    c.arg("search").arg("--pkg").arg(&env.pkg).arg("--trust").arg(&env.pubkey).args(extra).arg(q);
    let (ok, t) = run(&mut c);
    assert!(ok, "{t}");
    serde_json::from_str(&t).unwrap_or_else(|e| panic!("bad json ({e}): {t}"))
}

fn citable(v: &Value) -> Vec<String> {
    v["citable"].as_array().unwrap().iter().map(|h| h["passage_id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn build_verify_and_tamper() {
    let env = setup();
    let (ok, t) = run(kb().arg("verify").arg("--full").arg("--pkg").arg(&env.pkg).arg("--trust").arg(&env.pubkey));
    assert!(ok, "{t}");
    std::fs::write(env.pkg.join("passages.jsonl"), b"tampered\n").unwrap();
    let (ok, t) = run(kb().arg("verify").arg("--full").arg("--pkg").arg(&env.pkg).arg("--trust").arg(&env.pubkey));
    assert!(!ok);
    assert!(t.contains("FAIL"), "{t}");
}

#[test]
fn exact_current_manual_is_verified() {
    let env = setup();
    let v = search(&env, "How often should I change the engine oil?", &["--asset", "acme:gx-2000"]);
    assert_eq!(v["class"], "verified");
    let c = citable(&v);
    assert!(c.contains(&"gx2000b-oil-1".to_string()), "{c:?}");
    assert!(!c.contains(&"gx2000a-oil-1".to_string()), "superseded Rev A must be dropped: {c:?}");
    assert!(!c.contains(&"gx3000-oil-1".to_string()), "other model must be excluded: {c:?}");
}

#[test]
fn unknown_equipment_gets_partial_with_note() {
    let env = setup();
    let v = search(&env, "How often should I change the engine oil on the generator?", &["--asset", "cat:3208"]);
    assert_eq!(v["class"], "partial");
    assert!(v["notes"][0].as_str().unwrap().contains("cat:3208"));
    let c = citable(&v);
    assert!(c.iter().all(|id| !id.starts_with("gx")), "no Acme manual may be cited for a CAT engine: {c:?}");
}

#[test]
fn general_government_guidance_is_verified() {
    let env = setup();
    let v = search(&env, "Can I run a generator in the garage?", &[]);
    assert_eq!(v["class"], "verified");
    assert!(citable(&v).contains(&"fsa-co-1".to_string()));
}

#[test]
fn jurisdiction_filters_other_states() {
    let env = setup();
    let v = search(&env, "Can I do outdoor burning of brush?", &["--jurisdiction", "US-TX"]);
    assert_eq!(v["class"], "verified");
    let c = citable(&v);
    assert!(c.contains(&"tx-burn-1".to_string()));
    assert!(!c.contains(&"ca-burn-1".to_string()));
}

#[test]
fn community_only_is_partial() {
    let env = setup();
    let v = search(&env, "How do I reduce generator noise?", &[]);
    assert_eq!(v["class"], "partial");
}

#[test]
fn out_of_scope_is_unsupported() {
    let env = setup();
    let v = search(&env, "What is the capital of Peru?", &[]);
    assert_eq!(v["class"], "unsupported");
}

#[test]
fn untrusted_key_refuses_to_mount() {
    let env = setup();
    let other = env.pkg.parent().unwrap().join("other");
    run(kb().args(["keygen", "--out"]).arg(&other));
    let (ok, t) =
        run(kb().arg("search").arg("--pkg").arg(&env.pkg).arg("--trust").arg(other.with_extension("pub")).arg("oil"));
    assert!(!ok);
    assert!(t.contains("trusted key"), "{t}");
}
