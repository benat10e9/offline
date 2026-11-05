//! Passage retrieval over prebuilt Tantivy indexes.
//!
//! Indexes are built once, by the content factory, and shipped inside signed
//! packages. At runtime they are opened read-only; the customer's machine never
//! builds or rewrites a public index.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::BufRead;
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, BoostQuery, Occur, Query, TermQuery};
use tantivy::schema::{Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value, STORED, STRING};
use tantivy::{doc, Index, IndexReader, ReloadPolicy, TantivyDocument, Term};
use thiserror::Error;

pub const INDEX_DIR: &str = "index";
pub const PASSAGES_FILE: &str = "passages.jsonl";

#[derive(Debug, Error)]
pub enum RetrievalError {
    #[error("tantivy: {0}")]
    Tantivy(#[from] tantivy::TantivyError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("passage line {line}: {source}")]
    BadPassage { line: usize, source: serde_json::Error },
    #[error("duplicate passage id {0}")]
    DuplicatePassage(String),
    #[error("passage line {line}: {msg}")]
    Invalid { line: usize, msg: String },
    #[error("unknown package {0}")]
    UnknownPackage(String),
}

/// A retrievable unit of text with a stable identity. `passage_id` must be
/// unique within a package and must not change between package versions unless
/// the text changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Passage {
    pub passage_id: String,
    pub doc_id: String,
    pub source_id: String,
    pub title: String,
    #[serde(default)]
    pub heading: String,
    pub text: String,
}

#[derive(Clone, Copy)]
struct Fields {
    passage_id: Field,
    doc_id: Field,
    source_id: Field,
    title: Field,
    heading: Field,
    text: Field,
}

/// Full-text fields store term frequencies but no positions: we never run phrase
/// queries, and positions were a large share of a Wikipedia-sized index.
fn text_no_positions() -> TextOptions {
    TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default().set_tokenizer("default").set_index_option(IndexRecordOption::WithFreqs),
        )
        .set_stored()
}

fn schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let f = Fields {
        passage_id: b.add_text_field("passage_id", STRING | STORED),
        doc_id: b.add_text_field("doc_id", STRING | STORED),
        source_id: b.add_text_field("source_id", STRING | STORED),
        title: b.add_text_field("title", text_no_positions()),
        heading: b.add_text_field("heading", text_no_positions()),
        text: b.add_text_field("text", text_no_positions()),
    };
    (b.build(), f)
}

fn fields_of(index: &Index) -> Result<Fields, RetrievalError> {
    let s = index.schema();
    Ok(Fields {
        passage_id: s.get_field("passage_id")?,
        doc_id: s.get_field("doc_id")?,
        source_id: s.get_field("source_id")?,
        title: s.get_field("title")?,
        heading: s.get_field("heading")?,
        text: s.get_field("text")?,
    })
}

pub fn read_passages(path: &Path) -> Result<Vec<Passage>, RetrievalError> {
    let f = std::fs::File::open(path)?;
    let mut out = Vec::new();
    for (i, line) in std::io::BufReader::new(f).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let p: Passage =
            serde_json::from_str(&line).map_err(|source| RetrievalError::BadPassage { line: i + 1, source })?;
        out.push(p);
    }
    Ok(out)
}

/// Build a fresh index in `out_dir/index` from passages. Called by the content factory.
pub fn build_index(out_dir: &Path, passages: &[Passage]) -> Result<usize, RetrievalError> {
    let mut seen = std::collections::HashSet::new();
    for p in passages {
        if !seen.insert(p.passage_id.as_str()) {
            return Err(RetrievalError::DuplicatePassage(p.passage_id.clone()));
        }
    }
    let dir = out_dir.join(INDEX_DIR);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    let (schema, f) = schema();
    let index = Index::create_in_dir(&dir, schema)?;
    let mut writer = index.writer_with_num_threads(1, 50_000_000)?;
    for p in passages {
        writer.add_document(doc!(
            f.passage_id => p.passage_id.as_str(),
            f.doc_id => p.doc_id.as_str(),
            f.source_id => p.source_id.as_str(),
            f.title => p.title.as_str(),
            f.heading => p.heading.as_str(),
            f.text => p.text.as_str(),
        ))?;
    }
    writer.commit()?;
    // Merge into as few segments as possible so the shipped index is compact and stable.
    let ids = index.searchable_segment_ids()?;
    if ids.len() > 1 {
        writer.merge(&ids).wait()?;
    }
    writer.wait_merging_threads()?;
    Ok(passages.len())
}

/// Stream `passages.jsonl` into a fresh index without holding it in memory.
/// `check` sees every passage first (e.g. to validate source ids); an error stops the build.
pub fn build_index_streaming(
    out_dir: &Path,
    passages_jsonl: &Path,
    mut check: impl FnMut(&Passage) -> Result<(), String>,
) -> Result<usize, RetrievalError> {
    let dir = out_dir.join(INDEX_DIR);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    let (schema, f) = schema();
    let index = Index::create_in_dir(&dir, schema)?;
    let mut writer = index.writer_with_num_threads(2, 400_000_000)?;
    let mut seen = std::collections::HashSet::new();
    let mut n = 0usize;
    let file = std::fs::File::open(passages_jsonl)?;
    for (i, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let p: Passage =
            serde_json::from_str(&line).map_err(|source| RetrievalError::BadPassage { line: i + 1, source })?;
        check(&p).map_err(|msg| RetrievalError::Invalid { line: i + 1, msg })?;
        if !seen.insert(p.passage_id.clone()) {
            return Err(RetrievalError::DuplicatePassage(p.passage_id));
        }
        writer.add_document(doc!(
            f.passage_id => p.passage_id.as_str(),
            f.doc_id => p.doc_id.as_str(),
            f.source_id => p.source_id.as_str(),
            f.title => p.title.as_str(),
            f.heading => p.heading.as_str(),
            f.text => p.text.as_str(),
        ))?;
        n += 1;
    }
    writer.commit()?;
    let ids = index.searchable_segment_ids()?;
    if ids.len() > 1 {
        writer.merge(&ids).wait()?;
    }
    writer.wait_merging_threads()?;
    Ok(n)
}

/// A search result. `package_id` + `passage_id` is the citation key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub package_id: String,
    pub passage_id: String,
    pub doc_id: String,
    pub source_id: String,
    pub title: String,
    pub heading: String,
    pub text: String,
    pub score: f32,
}

impl Hit {
    pub fn citation_key(&self) -> String {
        citation_key(&self.package_id, &self.passage_id)
    }
}

pub fn citation_key(package_id: &str, passage_id: &str) -> String {
    format!("{package_id}/{passage_id}")
}

struct Mounted {
    index: Index,
    reader: IndexReader,
    fields: Fields,
}

/// All mounted package indexes, searched together.
#[derive(Default)]
pub struct Library {
    packages: Vec<(String, Mounted)>,
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open `package_dir/index` read-only under `package_id`.
    pub fn mount(&mut self, package_id: &str, package_dir: &Path) -> Result<(), RetrievalError> {
        let index = Index::open_in_dir(package_dir.join(INDEX_DIR))?;
        let reader = index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into()?;
        let fields = fields_of(&index)?;
        self.packages.retain(|(id, _)| id != package_id);
        self.packages.push((package_id.to_string(), Mounted { index, reader, fields }));
        Ok(())
    }

    pub fn package_ids(&self) -> Vec<String> {
        self.packages.iter().map(|(id, _)| id.clone()).collect()
    }

    /// BM25 search across every mounted package over passage text, with a light title boost.
    /// Returns up to `limit` hits per package, merged by score.
    /// Scores from different packages are merged directly; good enough for v0, revisit
    /// with a reranker once packages differ greatly in size.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>, RetrievalError> {
        let mut all = Vec::new();
        let words: Vec<String> = sanitize_query(query).split_whitespace().map(|w| w.to_lowercase()).collect();
        for (pkg, m) in &self.packages {
            // Terms OR'd over passage text plus a lightly boosted title. Passage text already
            // contains its headings, so the heading field is not scored separately.
            let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            let mut tok = m.index.tokenizer_for_field(m.fields.text)?;
            for w in &words {
                let mut stream = tok.token_stream(w);
                while let Some(t) = stream.next() {
                    for (field, boost) in [(m.fields.text, 1.0f32), (m.fields.title, 1.2)] {
                        let tq = TermQuery::new(Term::from_field_text(field, &t.text), IndexRecordOption::WithFreqs);
                        clauses.push((Occur::Should, Box::new(BoostQuery::new(Box::new(tq), boost))));
                    }
                }
            }
            if clauses.is_empty() {
                continue;
            }
            let q = BooleanQuery::new(clauses);
            let searcher = m.reader.searcher();
            let top = searcher.search(&q, &TopDocs::with_limit(limit).order_by_score())?;
            for (score, addr) in top {
                let d: TantivyDocument = searcher.doc(addr)?;
                all.push(to_hit(pkg, &m.fields, &d, score));
            }
        }
        // Up to `limit` hits from *each* package: a very large package (Wikipedia) must not
        // crowd a small authoritative one (a field manual) out of the candidate set.
        all.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        Ok(all)
    }

    /// Fetch one passage by its citation identity.
    pub fn get(&self, package_id: &str, passage_id: &str) -> Result<Option<Hit>, RetrievalError> {
        let (_, m) = self
            .packages
            .iter()
            .find(|(id, _)| id == package_id)
            .ok_or_else(|| RetrievalError::UnknownPackage(package_id.to_string()))?;
        let searcher = m.reader.searcher();
        let q = TermQuery::new(Term::from_field_text(m.fields.passage_id, passage_id), IndexRecordOption::Basic);
        let top = searcher.search(&q, &TopDocs::with_limit(1).order_by_score())?;
        match top.into_iter().next() {
            Some((score, addr)) => {
                let d: TantivyDocument = searcher.doc(addr)?;
                Ok(Some(to_hit(package_id, &m.fields, &d, score)))
            }
            None => Ok(None),
        }
    }

    /// All passages of one document, in index order. Used by the reader view.
    pub fn document(&self, package_id: &str, doc_id: &str) -> Result<Vec<Hit>, RetrievalError> {
        let (_, m) = self
            .packages
            .iter()
            .find(|(id, _)| id == package_id)
            .ok_or_else(|| RetrievalError::UnknownPackage(package_id.to_string()))?;
        let searcher = m.reader.searcher();
        let q = TermQuery::new(Term::from_field_text(m.fields.doc_id, doc_id), IndexRecordOption::Basic);
        let top = searcher.search(&q, &TopDocs::with_limit(10_000).order_by_score())?;
        let mut out = Vec::new();
        for (score, addr) in top {
            let d: TantivyDocument = searcher.doc(addr)?;
            out.push((addr, to_hit(package_id, &m.fields, &d, score)));
        }
        out.sort_by_key(|(addr, _)| (addr.segment_ord, addr.doc_id));
        Ok(out.into_iter().map(|(_, h)| h).collect())
    }

    /// Inverse document frequency of a lowercase word across all mounted packages
    /// (BM25 form). Rare, distinctive words score high; filler words score low.
    pub fn idf(&self, word: &str) -> f32 {
        let (mut n, mut df) = (0u64, 0u64);
        for (_, m) in &self.packages {
            let searcher = m.reader.searcher();
            n += searcher.num_docs();
            df += searcher.doc_freq(&Term::from_field_text(m.fields.text, word)).unwrap_or(0);
        }
        if n == 0 {
            return 1.0;
        }
        (1.0 + (n as f32 - df as f32 + 0.5) / (df as f32 + 0.5)).ln()
    }

    /// IDF of a word within one package: rarity relative to the collection a passage
    /// comes from, so installing a large encyclopedia can't change which words count
    /// as distinctive for the medical library.
    pub fn idf_in(&self, package_id: &str, word: &str) -> f32 {
        let Some((_, m)) = self.packages.iter().find(|(id, _)| id == package_id) else { return self.idf(word) };
        let searcher = m.reader.searcher();
        let n = searcher.num_docs();
        if n == 0 {
            return 1.0;
        }
        let df = searcher.doc_freq(&Term::from_field_text(m.fields.text, word)).unwrap_or(0);
        (1.0 + (n as f32 - df as f32 + 0.5) / (df as f32 + 0.5)).ln()
    }

    /// Passage counts per package, for diagnostics.
    pub fn stats(&self) -> HashMap<String, u64> {
        self.packages.iter().map(|(id, m)| (id.clone(), m.reader.searcher().num_docs())).collect()
    }
}

/// User questions are free text, not query syntax: keep letters, digits and
/// intra-word hyphens/apostrophes; everything else becomes a space. Terms are OR'd.
pub fn sanitize_query(q: &str) -> String {
    let cleaned: String =
        q.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '\'' { c } else { ' ' }).collect();
    cleaned
        .split_whitespace()
        .map(|w| w.trim_matches(|c| c == '-' || c == '\''))
        .filter(|w| !w.is_empty() && !matches!(*w, "AND" | "OR" | "NOT"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn text_of(d: &TantivyDocument, f: Field) -> String {
    d.get_first(f).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

fn to_hit(pkg: &str, f: &Fields, d: &TantivyDocument, score: f32) -> Hit {
    Hit {
        package_id: pkg.to_string(),
        passage_id: text_of(d, f.passage_id),
        doc_id: text_of(d, f.doc_id),
        source_id: text_of(d, f.source_id),
        title: text_of(d, f.title),
        heading: text_of(d, f.heading),
        text: text_of(d, f.text),
        score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: &str, doc: &str, title: &str, text: &str) -> Passage {
        Passage {
            passage_id: id.into(),
            doc_id: doc.into(),
            source_id: "src".into(),
            title: title.into(),
            heading: String::new(),
            text: text.into(),
        }
    }

    #[test]
    fn build_and_search_two_packages() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        build_index(
            a.path(),
            &[
                p("a1", "d1", "Water purification", "Boil water for one minute to kill pathogens."),
                p("a2", "d1", "Water purification", "Chemical disinfection uses unscented bleach."),
            ],
        )
        .unwrap();
        build_index(b.path(), &[p("b1", "d9", "Generator safety", "Never run a generator indoors.")]).unwrap();

        let mut lib = Library::new();
        lib.mount("water", a.path()).unwrap();
        lib.mount("power", b.path()).unwrap();

        let hits = lib.search("boil water", 5).unwrap();
        assert_eq!(hits[0].citation_key(), "water/a1");

        let hits = lib.search("generator indoors", 5).unwrap();
        assert_eq!(hits[0].package_id, "power");

        let got = lib.get("water", "a2").unwrap().unwrap();
        assert!(got.text.contains("bleach"));
        assert!(lib.get("water", "nope").unwrap().is_none());
        assert!(lib.get("missing", "a1").is_err());

        assert!(lib.idf("absentword") > lib.idf("water"), "rarer words weigh more");
        let doc = lib.document("water", "d1").unwrap();
        assert_eq!(doc.iter().map(|h| h.passage_id.as_str()).collect::<Vec<_>>(), vec!["a1", "a2"]);
    }

    #[test]
    fn lenient_query_survives_syntax() {
        let a = tempfile::tempdir().unwrap();
        build_index(a.path(), &[p("a1", "d1", "Burns", "Cool a burn under running water.")]).unwrap();
        let mut lib = Library::new();
        lib.mount("med", a.path()).unwrap();
        let hits = lib.search("burn: (\"cool AND", 3).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(sanitize_query("burn: (\"cool AND -x"), "burn cool x");
    }

    #[test]
    fn streaming_build_matches_and_validates() {
        let a = tempfile::tempdir().unwrap();
        let jl = a.path().join("p.jsonl");
        let ps =
            [p("a1", "d1", "Burns", "Cool a burn under running water."), p("a2", "d1", "Burns", "Do not use ice.")];
        std::fs::write(&jl, ps.iter().map(|x| serde_json::to_string(x).unwrap() + "\n").collect::<String>()).unwrap();
        assert_eq!(build_index_streaming(a.path(), &jl, |_| Ok(())).unwrap(), 2);
        let mut lib = Library::new();
        lib.mount("m", a.path()).unwrap();
        assert_eq!(lib.search("running water", 5).unwrap()[0].passage_id, "a1");
        let err =
            build_index_streaming(a.path(), &jl, |p| if p.passage_id == "a2" { Err("bad".into()) } else { Ok(()) });
        assert!(matches!(err, Err(RetrievalError::Invalid { line: 2, .. })));
    }

    #[test]
    fn duplicate_ids_rejected() {
        let a = tempfile::tempdir().unwrap();
        let err = build_index(a.path(), &[p("x", "d", "t", "a"), p("x", "d", "t", "b")]).unwrap_err();
        assert!(matches!(err, RetrievalError::DuplicatePassage(_)));
    }
}
