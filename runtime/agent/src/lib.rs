//! Answer pipeline: retrieve → assess evidence → generate → verify citations.
//!
//! Rules enforced in code, not by prompt:
//! - The model only sees passages the evidence layer marked citable.
//! - Unsupported questions are never sent to the model.
//! - Every citation marker must refer to a passage shown to the model in this turn;
//!   invalid markers are removed and reported.
//! - An answer with no valid citation is not shown as an answer.
//! - The final class can only be lowered by verification, never raised.

use kb_evidence::{assess, Assessment, EvidenceClass, QuestionContext, SourceLookup};
use kb_retrieval::{Hit, Library};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("retrieval: {0}")]
    Retrieval(#[from] kb_retrieval::RetrievalError),
    #[error("model: {0}")]
    Model(String),
}

/// A chat model. The production implementation talks to a local llama-server.
pub trait ChatModel {
    fn chat(&self, system: &str, user: &str) -> Result<String, AgentError>;
}

/// OpenAI-compatible client for llama.cpp's `llama-server` on localhost.
pub struct LlamaServer {
    pub base_url: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl LlamaServer {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), max_tokens: 1024, temperature: 0.2 }
    }
}

/// Speed figures for one generation, from llama-server's `usage` and `timings`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatStats {
    pub text: String,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    /// Prompt processing speed reported by llama-server.
    pub prompt_tokens_per_second: Option<f64>,
    /// Generation speed reported by llama-server.
    pub generated_tokens_per_second: Option<f64>,
    pub wall_seconds: f64,
}

impl LlamaServer {
    /// Same request as `chat`, returning speed statistics. Used by `kb bench`.
    pub fn chat_with_stats(&self, system: &str, user: &str) -> Result<ChatStats, AgentError> {
        let start = std::time::Instant::now();
        let resp = self.request(system, user)?;
        let wall = start.elapsed().as_secs_f64();
        let text = resp["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_string();
        let completion = resp["usage"]["completion_tokens"].as_u64();
        Ok(ChatStats {
            text,
            prompt_tokens: resp["usage"]["prompt_tokens"].as_u64(),
            completion_tokens: completion,
            prompt_tokens_per_second: resp["timings"]["prompt_per_second"].as_f64(),
            generated_tokens_per_second: resp["timings"]["predicted_per_second"]
                .as_f64()
                .or_else(|| completion.map(|c| c as f64 / wall.max(0.001))),
            wall_seconds: wall,
        })
    }

    fn request(&self, system: &str, user: &str) -> Result<serde_json::Value, AgentError> {
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "stream": false
        });
        ureq::post(&url)
            .timeout(std::time::Duration::from_secs(600))
            .send_json(body)
            .map_err(|e| AgentError::Model(e.to_string()))?
            .into_json()
            .map_err(|e| AgentError::Model(e.to_string()))
    }
}

impl ChatModel for LlamaServer {
    fn chat(&self, system: &str, user: &str) -> Result<String, AgentError> {
        let resp = self.request(system, user)?;
        resp["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AgentError::Model(format!("unexpected response: {resp}")))
    }
}

pub const SYSTEM_PROMPT: &str = "You answer questions using ONLY the numbered source passages provided. \
Rules: (1) Every sentence that states a fact or an instruction must end with the marker of the passage that \
supports it, like [S1] or [S1][S3]. (2) Never use knowledge that is not in the passages. If the passages do not \
cover part of the question, say so plainly instead of guessing. (3) Do not write a sources list; the application \
shows sources. (4) For safety-critical steps, keep the passage's warnings. (5) Be concise and practical.";

/// Build the user turn: evidence notes, numbered passages, then the question.
pub fn build_prompt(question: &str, assessment: &Assessment) -> String {
    let mut s = String::new();
    if !assessment.notes.is_empty() {
        s.push_str("Notes about the available evidence (repeat these to the user if relevant):\n");
        for n in &assessment.notes {
            s.push_str("- ");
            s.push_str(n);
            s.push('\n');
        }
        s.push('\n');
    }
    s.push_str("Source passages:\n");
    for (i, h) in assessment.citable.iter().enumerate() {
        let heading = if h.heading.is_empty() { String::new() } else { format!(" — {}", h.heading) };
        s.push_str(&format!("[S{}] {}{}\n{}\n\n", i + 1, h.title, heading, h.text.trim()));
    }
    s.push_str("Question: ");
    s.push_str(question);
    s
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub marker: usize,
    pub citation_key: String,
    pub package_id: String,
    pub passage_id: String,
    pub doc_id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    pub text: String,
    pub citations: Vec<Citation>,
    /// Markers the model wrote that point at no passage shown this turn.
    pub invalid_markers: Vec<String>,
    /// Sentences with no citation marker (excluding hedges like "I don't know").
    pub uncited_sentences: Vec<String>,
}

fn marker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[S(\d{1,3})\]").expect("valid regex"))
}

fn other_bracket_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Bracketed things that look like attempted citations in other formats.
    RE.get_or_init(|| Regex::new(r"\[(?:source|Source|ref|Ref|\d{1,3})[^\]]{0,40}\]").expect("valid regex"))
}

fn think_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Reasoning blocks some models emit; also an unterminated block at the start.
    RE.get_or_init(|| Regex::new(r"(?is)<think>.*?(?:</think>|\z)").expect("valid regex"))
}

fn sources_section_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?ims)^\s*(?:\*\*)?(?:sources|references)(?:\*\*)?\s*:.*\z").expect("valid regex"))
}

/// Check the model's markers against the passages it was shown.
pub fn verify_citations(answer: &str, shown: &[Hit]) -> Verification {
    let without_thinking = think_re().replace_all(answer, "");
    let mut text = sources_section_re().replace(&without_thinking, "").trim().to_string();
    let mut invalid = Vec::new();
    for m in other_bracket_re().find_iter(&text) {
        invalid.push(m.as_str().to_string());
    }
    text = other_bracket_re().replace_all(&text, "").to_string();

    let mut used = Vec::new();
    let cleaned = marker_re()
        .replace_all(&text, |c: &regex::Captures| {
            let n: usize = c[1].parse().unwrap_or(0);
            if n >= 1 && n <= shown.len() {
                if !used.contains(&n) {
                    used.push(n);
                }
                c[0].to_string()
            } else {
                invalid.push(c[0].to_string());
                String::new()
            }
        })
        .to_string();

    let uncited = split_sentences(&cleaned)
        .into_iter()
        .filter(|s| !marker_re().is_match(s) && !is_hedge(s) && s.split_whitespace().count() >= 4)
        .collect();

    used.sort_unstable();
    let citations = used
        .into_iter()
        .map(|n| {
            let h = &shown[n - 1];
            Citation {
                marker: n,
                citation_key: h.citation_key(),
                package_id: h.package_id.clone(),
                passage_id: h.passage_id.clone(),
                doc_id: h.doc_id.clone(),
                title: h.title.clone(),
            }
        })
        .collect();

    Verification { text: cleaned, citations, invalid_markers: invalid, uncited_sentences: uncited }
}

fn split_sentences(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        let end = matches!(c, '.' | '!' | '?' | '\n');
        // Keep trailing citation markers with the sentence they follow.
        let next_is_marker =
            chars.get(i + 1) == Some(&'[') || (chars.get(i + 1) == Some(&' ') && chars.get(i + 2) == Some(&'['));
        if end && !next_is_marker {
            let t = cur.trim().to_string();
            if !t.is_empty() {
                out.push(t);
            }
            cur.clear();
        }
    }
    let t = cur.trim().to_string();
    if !t.is_empty() {
        out.push(t);
    }
    out
}

fn is_hedge(s: &str) -> bool {
    let l = s.to_lowercase();
    [
        "not covered",
        "do not cover",
        "don't cover",
        "doesn't cover",
        "does not cover",
        "not in the",
        "no information",
        "cannot answer",
        "can't answer",
        "not enough",
        "consult",
        "not model-specific",
        "general guidance",
    ]
    .iter()
    .any(|p| l.contains(p))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Answer {
    pub question: String,
    pub class: EvidenceClass,
    /// The verified answer text, or None when nothing groundable was produced.
    pub text: Option<String>,
    pub citations: Vec<Citation>,
    /// Passages shown to the model (and to the user as "sources consulted").
    pub consulted: Vec<Hit>,
    pub notes: Vec<String>,
    pub assessment: Assessment,
    pub verification: Option<Verification>,
}

#[derive(Debug, Clone)]
pub struct AskOptions {
    pub retrieve: usize,
    pub max_passages: usize,
}

impl Default for AskOptions {
    fn default() -> Self {
        Self { retrieve: 20, max_passages: 8 }
    }
}

/// Search only, no model: used on machines below the safe model tier and as the fallback.
pub fn search_only(
    library: &Library,
    sources: &dyn SourceLookup,
    question: &str,
    ctx: &QuestionContext,
    opts: &AskOptions,
) -> Result<Assessment, AgentError> {
    let hits = library.search(question, opts.retrieve)?;
    let mut a = assess(question, ctx, &hits, sources);
    a.citable.truncate(opts.max_passages);
    Ok(a)
}

pub fn ask(
    library: &Library,
    sources: &dyn SourceLookup,
    model: &dyn ChatModel,
    question: &str,
    ctx: &QuestionContext,
    opts: &AskOptions,
) -> Result<Answer, AgentError> {
    let assessment = search_only(library, sources, question, ctx, opts)?;
    let mut notes = assessment.notes.clone();

    if assessment.class == EvidenceClass::Unsupported {
        return Ok(Answer {
            question: question.into(),
            class: EvidenceClass::Unsupported,
            text: None,
            citations: vec![],
            consulted: vec![],
            notes,
            assessment,
            verification: None,
        });
    }

    let prompt = build_prompt(question, &assessment);
    let raw = model.chat(SYSTEM_PROMPT, &prompt)?;
    let v = verify_citations(&raw, &assessment.citable);

    let mut class = assessment.class;
    let mut text = Some(v.text.clone());
    if v.citations.is_empty() {
        class = EvidenceClass::Unsupported;
        text = None;
        notes.push("The model's answer could not be tied to any source passage, so it is not shown. The passages consulted are listed below.".into());
    } else {
        // Only verified passages can make the answer verified.
        let cites_verified = v.citations.iter().any(|c| assessment.verified_keys.contains(&c.citation_key));
        if class == EvidenceClass::Verified && !cites_verified {
            class = EvidenceClass::Partial;
        }
        if class == EvidenceClass::Verified && !v.uncited_sentences.is_empty() {
            class = EvidenceClass::Partial;
            notes.push("Some statements in this answer are not tied to a source passage.".into());
        }
        if !v.invalid_markers.is_empty() {
            notes.push(format!(
                "{} citation(s) pointed at sources that were not consulted and were removed.",
                v.invalid_markers.len()
            ));
        }
    }

    Ok(Answer {
        question: question.into(),
        class,
        text,
        citations: v.citations.clone(),
        consulted: assessment.citable.clone(),
        notes,
        assessment,
        verification: Some(v),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str) -> Hit {
        Hit {
            package_id: "p".into(),
            passage_id: id.into(),
            doc_id: "d".into(),
            source_id: "s".into(),
            title: "T".into(),
            heading: String::new(),
            text: "x".into(),
            score: 1.0,
        }
    }

    #[test]
    fn valid_and_invalid_markers() {
        let shown = vec![hit("a"), hit("b")];
        let v = verify_citations("Boil water for one minute [S1]. Bleach works too [S2][S7]. Also see [3].", &shown);
        assert_eq!(v.citations.iter().map(|c| c.marker).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(v.invalid_markers.len(), 2);
        assert!(!v.text.contains("[S7]"));
        assert!(!v.text.contains("[3]"));
    }

    #[test]
    fn strips_model_sources_list_and_finds_uncited() {
        let shown = vec![hit("a")];
        let v = verify_citations(
            "Shut off the fuel valve before servicing [S1]. Then wait thirty minutes for the engine to cool.\n\nSources: CDC, FEMA",
            &shown,
        );
        assert!(!v.text.contains("FEMA"));
        assert_eq!(v.uncited_sentences, vec!["Then wait thirty minutes for the engine to cool."]);
    }

    #[test]
    fn strips_reasoning_blocks() {
        let v = verify_citations("<think>maybe bleach?</think>Boil water for one minute [S1].", &[hit("a")]);
        assert_eq!(v.text, "Boil water for one minute [S1].");
        let v = verify_citations("<think>never closed", &[hit("a")]);
        assert_eq!(v.text, "");
    }

    #[test]
    fn hedges_are_not_counted_as_uncited() {
        let v = verify_citations(
            "Drain the oil [S1]. The passages do not cover torque values for this model.",
            &[hit("a")],
        );
        assert!(v.uncited_sentences.is_empty());
    }

    struct Fixed(&'static str);
    impl ChatModel for Fixed {
        fn chat(&self, _s: &str, _u: &str) -> Result<String, AgentError> {
            Ok(self.0.to_string())
        }
    }

    fn setup() -> (tempfile::TempDir, Library, std::collections::HashMap<(String, String), kb_packages::SourceRecord>) {
        let dir = tempfile::tempdir().unwrap();
        kb_retrieval::build_index(
            dir.path(),
            &[kb_retrieval::Passage {
                passage_id: "p1".into(),
                doc_id: "d1".into(),
                source_id: "fema".into(),
                title: "Generator safety".into(),
                heading: String::new(),
                text: "Never run a portable generator indoors or in a garage; carbon monoxide can kill.".into(),
            }],
        )
        .unwrap();
        let mut lib = Library::new();
        lib.mount("pkg", dir.path()).unwrap();
        let mut src = std::collections::HashMap::new();
        src.insert(
            ("pkg".to_string(), "fema".to_string()),
            kb_packages::SourceRecord {
                id: "fema".into(),
                title: "Generator safety".into(),
                publisher: "FEMA".into(),
                license: "LicenseRef-PublicDomain-USGov".into(),
                authority: kb_packages::Authority::Government,
                revision: None,
                superseded_by: None,
                applies_to: vec![],
                jurisdictions: vec![],
                url: None,
            },
        );
        (dir, lib, src)
    }

    #[test]
    fn end_to_end_verified() {
        let (_d, lib, src) = setup();
        let m = Fixed("Do not run a generator indoors or in a garage because of carbon monoxide [S1].");
        let a = ask(
            &lib,
            &src,
            &m,
            "Can I run my generator in the garage?",
            &QuestionContext::default(),
            &AskOptions::default(),
        )
        .unwrap();
        assert_eq!(a.class, EvidenceClass::Verified);
        assert_eq!(a.citations[0].citation_key, "pkg/p1");
    }

    #[test]
    fn uncited_model_answer_is_withheld() {
        let (_d, lib, src) = setup();
        let m = Fixed("Sure, garages are fine if the door is open.");
        let a = ask(
            &lib,
            &src,
            &m,
            "Can I run my generator in the garage?",
            &QuestionContext::default(),
            &AskOptions::default(),
        )
        .unwrap();
        assert_eq!(a.class, EvidenceClass::Unsupported);
        assert!(a.text.is_none());
    }

    #[test]
    fn unsupported_never_calls_model() {
        struct Panics;
        impl ChatModel for Panics {
            fn chat(&self, _s: &str, _u: &str) -> Result<String, AgentError> {
                panic!("model must not be called")
            }
        }
        let (_d, lib, src) = setup();
        let a = ask(
            &lib,
            &src,
            &Panics,
            "What is the airspeed of a swallow?",
            &QuestionContext::default(),
            &AskOptions::default(),
        )
        .unwrap();
        assert_eq!(a.class, EvidenceClass::Unsupported);
    }
}
