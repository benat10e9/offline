//! Knowledge package format: a directory containing `manifest.json`, `manifest.sig`
//! and the files the manifest lists. See `spec/package-format.md`.
//!
//! Verification never consults the clock or the network: a package is valid if its
//! manifest is signed by a trusted key and every listed file hashes correctly.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

pub const MANIFEST_FILE: &str = "manifest.json";
pub const SIGNATURE_FILE: &str = "manifest.sig";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum PackageError {
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("manifest is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported package format version {0}")]
    FormatVersion(u32),
    #[error("signature file is malformed")]
    BadSignatureEncoding,
    #[error("manifest signature does not match any trusted key")]
    UntrustedSignature,
    #[error("file listed in manifest is missing: {0}")]
    MissingFile(String),
    #[error("file hash mismatch: {0}")]
    HashMismatch(String),
    #[error("file size mismatch: {0}")]
    SizeMismatch(String),
    #[error("file present but not listed in manifest: {0}")]
    UnlistedFile(String),
    #[error("unsafe path in manifest: {0}")]
    UnsafePath(String),
    #[error("key is malformed")]
    BadKey,
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> PackageError + '_ {
    move |source| PackageError::Io { path: path.to_path_buf(), source }
}

/// How authoritative a source is. Used by the evidence classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    /// Manufacturer or issuing agency's own document for the exact item or jurisdiction.
    Official,
    /// Government or standards-body guidance of general applicability.
    Government,
    /// Reputable reference work (encyclopedia, textbook).
    Reference,
    /// Community content (Q&A, wikis).
    Community,
}

/// A source document contained in the package, with the facts the evidence
/// classifier needs to decide whether it applies to a question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: String,
    pub title: String,
    pub publisher: String,
    /// SPDX identifier or LicenseRef-* for the source's license.
    pub license: String,
    pub authority: Authority,
    /// Document revision or edition label, e.g. "Rev B" or "2024".
    #[serde(default)]
    pub revision: Option<String>,
    /// ID of the source that replaces this one, if it has been superseded.
    #[serde(default)]
    pub superseded_by: Option<String>,
    /// Exact equipment identifiers this document is written for, normalised
    /// as `make:model` (lowercase). Empty = general applicability.
    #[serde(default)]
    pub applies_to: Vec<String>,
    /// Jurisdiction codes, e.g. "US", "US-TX". Empty = not jurisdiction specific.
    #[serde(default)]
    pub jurisdictions: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// Who the source is written for: "public" (default) or "clinician". Clinician
    /// material is labelled in prompts and answers so steps needing a professional are named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<String>,
}

impl SourceRecord {
    pub fn for_clinicians(&self) -> bool {
        self.audience.as_deref() == Some("clinician")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub id: String,
    pub version: String,
    pub title: String,
    /// Editions this package may be installed in: consumer, police, military, ems.
    pub editions: Vec<String>,
    /// Package IDs this package needs, as `id@version`.
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub sources: Vec<SourceRecord>,
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub fn source(&self, id: &str) -> Option<&SourceRecord> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// Canonical bytes that are signed. Struct field order is fixed; files are sorted.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PackageError> {
        let mut m = self.clone();
        m.files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(serde_json::to_vec_pretty(&m)?)
    }
}

/// Fields the publisher supplies; files are computed by `build_manifest`.
#[derive(Debug, Clone)]
pub struct ManifestInput {
    pub id: String,
    pub version: String,
    pub title: String,
    pub editions: Vec<String>,
    pub depends_on: Vec<String>,
    pub sources: Vec<SourceRecord>,
}

pub fn sha256_file(path: &Path) -> Result<(String, u64), PackageError> {
    let mut f = fs::File::open(path).map_err(io_err(path))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(io_err(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), total))
}

fn rel_path(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .expect("walked path is under root")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn is_meta(rel: &str) -> bool {
    rel == MANIFEST_FILE || rel == SIGNATURE_FILE
}

fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, PackageError> {
    let p = Path::new(rel);
    let only_normal = p.components().all(|c| matches!(c, Component::Normal(_)));
    if rel.is_empty() || p.is_absolute() || !only_normal || rel.contains("/./") || rel.contains('\\') {
        return Err(PackageError::UnsafePath(rel.to_string()));
    }
    Ok(root.join(p))
}

fn package_files(dir: &Path) -> Result<Vec<String>, PackageError> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(dir).sort_by_file_name() {
        let entry = entry.map_err(|e| PackageError::Io { path: dir.to_path_buf(), source: e.into() })?;
        if entry.file_type().is_file() {
            let rel = rel_path(dir, entry.path());
            if !is_meta(&rel) {
                out.push(rel);
            }
        }
    }
    Ok(out)
}

/// Hash every file under `dir` (except manifest/signature) into a manifest.
pub fn build_manifest(dir: &Path, input: ManifestInput) -> Result<Manifest, PackageError> {
    let mut files = Vec::new();
    for rel in package_files(dir)? {
        let (sha256, bytes) = sha256_file(&dir.join(&rel))?;
        files.push(FileEntry { path: rel, sha256, bytes });
    }
    Ok(Manifest {
        format_version: FORMAT_VERSION,
        id: input.id,
        version: input.version,
        title: input.title,
        editions: input.editions,
        depends_on: input.depends_on,
        sources: input.sources,
        files,
    })
}

pub struct KeyPair {
    pub signing: SigningKey,
}

impl KeyPair {
    pub fn generate() -> Result<Self, PackageError> {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).map_err(|_| PackageError::BadKey)?;
        Ok(Self { signing: SigningKey::from_bytes(&seed) })
    }
    pub fn from_secret_hex(s: &str) -> Result<Self, PackageError> {
        let bytes: [u8; 32] =
            hex::decode(s.trim()).map_err(|_| PackageError::BadKey)?.try_into().map_err(|_| PackageError::BadKey)?;
        Ok(Self { signing: SigningKey::from_bytes(&bytes) })
    }
    pub fn secret_hex(&self) -> String {
        hex::encode(self.signing.to_bytes())
    }
    pub fn public_hex(&self) -> String {
        hex::encode(self.signing.verifying_key().to_bytes())
    }
}

pub fn parse_public_key(s: &str) -> Result<VerifyingKey, PackageError> {
    let bytes: [u8; 32] =
        hex::decode(s.trim()).map_err(|_| PackageError::BadKey)?.try_into().map_err(|_| PackageError::BadKey)?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| PackageError::BadKey)
}

/// Write `manifest.json` and `manifest.sig` into `dir`.
pub fn sign_package(dir: &Path, manifest: &Manifest, key: &KeyPair) -> Result<(), PackageError> {
    let bytes = manifest.canonical_bytes()?;
    let sig = key.signing.sign(&bytes);
    let mpath = dir.join(MANIFEST_FILE);
    fs::write(&mpath, &bytes).map_err(io_err(&mpath))?;
    let spath = dir.join(SIGNATURE_FILE);
    let body = format!("ed25519 {} {}\n", key.public_hex(), hex::encode(sig.to_bytes()));
    fs::write(&spath, body).map_err(io_err(&spath))?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    /// Signature only. Fast; used at every launch.
    SignatureOnly,
    /// Signature plus every file's size and SHA-256. Used for install, update and repair.
    Full,
}

/// Verify a package directory against trusted public keys. Returns the manifest.
pub fn verify_package(dir: &Path, trusted: &[VerifyingKey], mode: VerifyMode) -> Result<Manifest, PackageError> {
    let mpath = dir.join(MANIFEST_FILE);
    let bytes = fs::read(&mpath).map_err(io_err(&mpath))?;
    let spath = dir.join(SIGNATURE_FILE);
    let sig_text = fs::read_to_string(&spath).map_err(io_err(&spath))?;
    let parts: Vec<&str> = sig_text.split_whitespace().collect();
    if parts.len() != 3 || parts[0] != "ed25519" {
        return Err(PackageError::BadSignatureEncoding);
    }
    let sig_bytes: [u8; 64] = hex::decode(parts[2])
        .map_err(|_| PackageError::BadSignatureEncoding)?
        .try_into()
        .map_err(|_| PackageError::BadSignatureEncoding)?;
    let sig = Signature::from_bytes(&sig_bytes);
    if !trusted.iter().any(|k| k.verify(&bytes, &sig).is_ok()) {
        return Err(PackageError::UntrustedSignature);
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    if manifest.format_version != FORMAT_VERSION {
        return Err(PackageError::FormatVersion(manifest.format_version));
    }
    for f in &manifest.files {
        safe_join(dir, &f.path)?;
    }
    if mode == VerifyMode::Full {
        let listed: BTreeSet<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        for rel in package_files(dir)? {
            if !listed.contains(rel.as_str()) {
                return Err(PackageError::UnlistedFile(rel));
            }
        }
        for f in &manifest.files {
            let p = safe_join(dir, &f.path)?;
            if !p.is_file() {
                return Err(PackageError::MissingFile(f.path.clone()));
            }
            let (sha, bytes) = sha256_file(&p)?;
            if bytes != f.bytes {
                return Err(PackageError::SizeMismatch(f.path.clone()));
            }
            if sha != f.sha256 {
                return Err(PackageError::HashMismatch(f.path.clone()));
            }
        }
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ManifestInput {
        ManifestInput {
            id: "test-pkg".into(),
            version: "1.0.0".into(),
            title: "Test".into(),
            editions: vec!["consumer".into()],
            depends_on: vec![],
            sources: vec![],
        }
    }

    fn make_pkg() -> (tempfile::TempDir, KeyPair) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("index")).unwrap();
        fs::write(dir.path().join("index/a.bin"), b"hello").unwrap();
        fs::write(dir.path().join("passages.jsonl"), b"{}\n").unwrap();
        let key = KeyPair::generate().unwrap();
        let m = build_manifest(dir.path(), input()).unwrap();
        sign_package(dir.path(), &m, &key).unwrap();
        (dir, key)
    }

    fn pk(key: &KeyPair) -> Vec<VerifyingKey> {
        vec![parse_public_key(&key.public_hex()).unwrap()]
    }

    #[test]
    fn roundtrip_verifies() {
        let (dir, key) = make_pkg();
        let m = verify_package(dir.path(), &pk(&key), VerifyMode::Full).unwrap();
        assert_eq!(m.files.len(), 2);
        assert_eq!(m.files[0].path, "index/a.bin");
    }

    #[test]
    fn secret_key_roundtrip() {
        let k = KeyPair::generate().unwrap();
        let k2 = KeyPair::from_secret_hex(&k.secret_hex()).unwrap();
        assert_eq!(k.public_hex(), k2.public_hex());
    }

    #[test]
    fn wrong_key_rejected() {
        let (dir, _) = make_pkg();
        let other = KeyPair::generate().unwrap();
        let err = verify_package(dir.path(), &pk(&other), VerifyMode::SignatureOnly).unwrap_err();
        assert!(matches!(err, PackageError::UntrustedSignature));
    }

    #[test]
    fn tampered_file_rejected() {
        let (dir, key) = make_pkg();
        fs::write(dir.path().join("index/a.bin"), b"hellO").unwrap();
        let err = verify_package(dir.path(), &pk(&key), VerifyMode::Full).unwrap_err();
        assert!(matches!(err, PackageError::HashMismatch(_)));
        // Signature-only mode does not read content.
        assert!(verify_package(dir.path(), &pk(&key), VerifyMode::SignatureOnly).is_ok());
    }

    #[test]
    fn tampered_manifest_rejected() {
        let (dir, key) = make_pkg();
        let p = dir.path().join(MANIFEST_FILE);
        let s = fs::read_to_string(&p).unwrap().replace("\"Test\"", "\"Evil\"");
        fs::write(&p, s).unwrap();
        let err = verify_package(dir.path(), &pk(&key), VerifyMode::SignatureOnly).unwrap_err();
        assert!(matches!(err, PackageError::UntrustedSignature));
    }

    #[test]
    fn extra_and_missing_files_rejected() {
        let (dir, key) = make_pkg();
        fs::write(dir.path().join("injected.txt"), b"x").unwrap();
        assert!(matches!(
            verify_package(dir.path(), &pk(&key), VerifyMode::Full).unwrap_err(),
            PackageError::UnlistedFile(_)
        ));
        fs::remove_file(dir.path().join("injected.txt")).unwrap();
        fs::remove_file(dir.path().join("passages.jsonl")).unwrap();
        assert!(matches!(
            verify_package(dir.path(), &pk(&key), VerifyMode::Full).unwrap_err(),
            PackageError::MissingFile(_)
        ));
    }

    #[test]
    fn path_traversal_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(safe_join(dir.path(), "../etc/passwd").is_err());
        assert!(safe_join(dir.path(), "/etc/passwd").is_err());
        assert!(safe_join(dir.path(), "a/./b").is_err());
        assert!(safe_join(dir.path(), "a\\b").is_err());
        assert!(safe_join(dir.path(), "a/b").is_ok());
    }
}
