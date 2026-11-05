//! `kb eval`: grade a library (and optionally a model) against a question set.
//! See evals/README.md for the file format.

use crate::{AskResponse, Engine, QuestionRequest};
use kb_evidence::EvidenceClass;
use serde::{Deserialize, Serialize};
use std::io::BufRead;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, Deserialize)]
pub struct EvalItem {
    pub id: String,
    pub question: String,
    pub expect_class: Vec<EvidenceClass>,
    #[serde(default)]
    pub expect_sources: Vec<String>,
    #[serde(default)]
    pub must_not: Vec<String>,
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(default)]
    pub jurisdiction: Option<String>,
}

pub fn load_items(path: &Path) -> anyhow::Result<Vec<EvalItem>> {
    let f = std::fs::File::open(path)?;
    let mut out = Vec::new();
    for (i, line) in std::io::BufReader::new(f).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(&line).map_err(|e| anyhow::anyhow!("line {}: {e}", i + 1))?);
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalResult {
    pub id: String,
    pub question: String,
    pub class: EvidenceClass,
    pub class_ok: bool,
    /// None when the item has no source expectation.
    pub source_hit: Option<bool>,
    pub top_sources: Vec<String>,
    /// Generated-answer fields (with --with-model).
    pub answered: Option<bool>,
    pub citations: Option<usize>,
    pub violations: Vec<String>,
    pub seconds: f64,
    pub answer: Option<String>,
    /// The passages the answer cites, as (marker, title, text), for human review of
    /// whether each source actually supports the answer.
    pub cited: Vec<CitedPassage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CitedPassage {
    pub marker: usize,
    pub source_id: String,
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalSummary {
    pub items: usize,
    pub class_ok: usize,
    pub source_expectations: usize,
    pub source_hits: usize,
    pub out_of_scope_items: usize,
    pub out_of_scope_wrongly_verified: usize,
    pub model_graded: usize,
    pub answers_withheld: usize,
    pub safety_violations: usize,
    pub mean_seconds: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalReport {
    pub summary: EvalSummary,
    pub results: Vec<EvalResult>,
}

pub fn run(engine: &Engine, items: &[EvalItem], with_model: bool) -> anyhow::Result<EvalReport> {
    run_with_progress(engine, items, with_model, &mut |_, _| {})
}

/// Like `run`, calling `progress` after every question so long model runs can stream
/// results and survive a timeout with partial data.
pub fn run_with_progress(
    engine: &Engine,
    items: &[EvalItem],
    with_model: bool,
    progress: &mut dyn FnMut(&EvalResult, &[EvalResult]),
) -> anyhow::Result<EvalReport> {
    let mut results: Vec<EvalResult> = Vec::new();
    for it in items {
        let req = QuestionRequest {
            question: it.question.clone(),
            asset: it.asset.clone(),
            jurisdiction: it.jurisdiction.clone(),
        };
        let t = Instant::now();
        let mut cited = Vec::new();
        let (assessment, answered, citations, answer) = if with_model {
            match engine.ask(&req)? {
                AskResponse::Answer { answer, .. } => {
                    let n = answer.citations.len();
                    for c in &answer.citations {
                        if let Some(h) = answer.consulted.get(c.marker - 1) {
                            cited.push(CitedPassage {
                                marker: c.marker,
                                source_id: h.source_id.clone(),
                                title: h.title.clone(),
                                text: h.text.clone(),
                            });
                        }
                    }
                    let class = answer.class;
                    let mut a = answer.assessment.clone();
                    a.class = class; // grade the final class, after citation verification
                    (a, Some(answer.text.is_some()), Some(n), answer.text)
                }
                AskResponse::SearchOnly { assessment, .. } => (assessment, None, None, None),
            }
        } else {
            (engine.search(&req)?, None, None, None)
        };
        let seconds = t.elapsed().as_secs_f64();
        let top_sources: Vec<String> = {
            let mut v: Vec<String> = Vec::new();
            for h in &assessment.citable {
                if !v.contains(&h.source_id) {
                    v.push(h.source_id.clone());
                }
            }
            v
        };
        let source_hit =
            (!it.expect_sources.is_empty()).then(|| top_sources.iter().any(|s| it.expect_sources.contains(s)));
        let mut violations = Vec::new();
        if let Some(text) = &answer {
            for pat in &it.must_not {
                match regex::Regex::new(pat) {
                    Ok(re) if re.is_match(text) => violations.push(pat.clone()),
                    Ok(_) => {}
                    Err(e) => violations.push(format!("bad pattern {pat}: {e}")),
                }
            }
        }
        results.push(EvalResult {
            id: it.id.clone(),
            question: it.question.clone(),
            class: assessment.class,
            class_ok: it.expect_class.contains(&assessment.class),
            source_hit,
            top_sources,
            answered,
            citations,
            violations,
            seconds,
            answer,
            cited,
        });
        let last = results.last().expect("just pushed");
        progress(last, &results);
    }
    Ok(summarize(results, items))
}

/// Summary over whatever has been graded so far.
pub fn summarize(results: Vec<EvalResult>, items: &[EvalItem]) -> EvalReport {
    let oos: Vec<&EvalResult> = results
        .iter()
        .filter(|r| {
            items.iter().find(|it| it.id == r.id).is_some_and(|it| !it.expect_class.contains(&EvidenceClass::Verified))
        })
        .collect();
    let summary = EvalSummary {
        items: results.len(),
        class_ok: results.iter().filter(|r| r.class_ok).count(),
        source_expectations: results.iter().filter(|r| r.source_hit.is_some()).count(),
        source_hits: results.iter().filter(|r| r.source_hit == Some(true)).count(),
        out_of_scope_items: oos.len(),
        out_of_scope_wrongly_verified: oos.iter().filter(|r| r.class == EvidenceClass::Verified).count(),
        model_graded: results.iter().filter(|r| r.answered.is_some()).count(),
        answers_withheld: results.iter().filter(|r| r.answered == Some(false)).count(),
        safety_violations: results.iter().map(|r| r.violations.len()).sum(),
        mean_seconds: results.iter().map(|r| r.seconds).sum::<f64>() / results.len().max(1) as f64,
    };
    EvalReport { summary, results }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_eval() {
        let root = crate::tests::make_root();
        let engine = Engine::open(crate::Root::new(root.path())).unwrap();
        let items: Vec<EvalItem> = [
            r#"{"id":"a","question":"How often should I change the engine oil?","asset":"acme:gx-2000","expect_class":["verified"],"expect_sources":["acme-gx2000-revB"]}"#,
            r#"{"id":"b","question":"What is the capital of Peru?","expect_class":["unsupported"]}"#,
            r#"{"id":"c","question":"How often should I change the engine oil?","asset":"acme:gx-2000","expect_class":["verified"],"expect_sources":["acme-gx2000-revA"]}"#,
        ]
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
        let r = run(&engine, &items, false).unwrap();
        assert_eq!(r.summary.items, 3);
        assert_eq!(r.summary.class_ok, 3);
        assert_eq!(r.summary.source_expectations, 2);
        assert_eq!(r.summary.source_hits, 1, "superseded Rev A must not be a source");
        assert_eq!(r.summary.out_of_scope_wrongly_verified, 0);
    }

    #[test]
    fn eval_file_parses() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/medical-survival.jsonl");
        let items = load_items(&p).unwrap();
        assert_eq!(items.len(), 50);
        let quick = load_items(&p.with_file_name("medical-survival-quick.jsonl")).unwrap();
        assert_eq!(quick.len(), 20);
        for q in &quick {
            let full = items.iter().find(|i| i.id == q.id).unwrap_or_else(|| panic!("{} not in full set", q.id));
            assert_eq!((&full.question, &full.expect_class, &full.must_not), (&q.question, &q.expect_class, &q.must_not));
        }
        for it in &items {
            for pat in &it.must_not {
                regex::Regex::new(pat).unwrap_or_else(|e| panic!("{}: {e}", it.id));
            }
        }
    }
}
