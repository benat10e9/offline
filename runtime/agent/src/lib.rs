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

    /// Stream the reply, calling `on_token` with each piece as it is generated.
    /// Default: no streaming, one call with the whole reply.
    fn chat_stream(&self, system: &str, user: &str, on_token: &mut dyn FnMut(&str)) -> Result<String, AgentError> {
        let t = self.chat(system, user)?;
        on_token(&t);
        Ok(t)
    }
}

/// Conversation context for one turn: recent exchanges, the user's saved notes, and the
/// text to search with (the question joined with the previous one for follow-ups).
#[derive(Debug, Clone, Default)]
pub struct Turn {
    /// (user question, assistant answer) pairs, oldest first.
    pub history: Vec<(String, String)>,
    /// Things the user told the app about themselves. Context, never evidence.
    pub notes: Vec<String>,
    pub retrieval_text: Option<String>,
    /// What relevance is judged against for a follow-up: the topic (the earlier question),
    /// since "what about with the door open?" alone, or joined, is too thin to judge by.
    pub topic_text: Option<String>,
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

    /// Server-sent events from llama-server's OpenAI-compatible endpoint.
    fn chat_stream(&self, system: &str, user: &str, on_token: &mut dyn FnMut(&str)) -> Result<String, AgentError> {
        use std::io::BufRead;
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "stream": true
        });
        let resp = ureq::post(&url)
            .timeout(std::time::Duration::from_secs(900))
            .send_json(body)
            .map_err(|e| AgentError::Model(e.to_string()))?;
        let mut out = String::new();
        for line in std::io::BufReader::new(resp.into_reader()).lines() {
            let line = line.map_err(|e| AgentError::Model(e.to_string()))?;
            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data == "[DONE]" {
                break;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(piece) = v["choices"][0]["delta"]["content"].as_str() {
                    out.push_str(piece);
                    on_token(piece);
                }
            }
        }
        Ok(out)
    }
}

pub const SYSTEM_PROMPT: &str = "You answer questions using ONLY the numbered source passages provided. \
Rules: (1) After every fact or instruction, write the marker of the passage that supports it, like [S1] or \
[S1][S3]; a marker at the end of a sentence covers that sentence and the uncited sentences just before it. (2) Never use knowledge that is not in the passages. If the passages do not \
cover part of the question, say so plainly instead of guessing. (3) Do not write a sources list; the application \
shows sources. (4) For safety-critical steps, keep the passage's warnings. (5) Be concise and practical. \
(6) Passages marked (for health professionals) describe care by trained staff: say which steps need a health \
professional and tell the user to get medical help; do not present those steps as things to do themselves. \
(7) When the passages describe a procedure, give every step, in order, as a numbered list (1., 2., 3.), with \
its warnings. Never stop partway through a procedure.";

/// Questions that must stay in strict, cited mode: health and injury, medicines, drinking
/// water and food safety, fuel/electrical/gas hazards, law and permits. Everything else may
/// be answered conversationally from the model's general knowledge (labelled as such).
pub fn needs_strict_mode(question: &str, ctx: &QuestionContext) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(concat!(
            r"(?i)\b(",
            // health, injury, medicine
            r"symptom|sick|ill(ness)?|disease|infect|fever|pain|ache|injur|wound|cut|bleed|blood|bruise|fractur|broken bone|sprain|",
            r"burn|blister|bite|sting|rash|allerg|asthma|breath|chok|heart|chest|stroke|seizure|faint|unconscious|dizz|vomit|diarrh|",
            r"dehydrat|pregnan|baby|infant|newborn|medic|drug|pill|dose|dosage|mg|antibiotic|insulin|diabet|vaccin|prescri|overdose|",
            r"poison|toxic|first aid|cpr|treat|diagnos|doctor|nurse|hospital|clinic|emergency|911|mental health|suicid|",
            // water and food safety
            r"drink(ing)? water|potable|purif|disinfect|boil(ing)? water|bleach|chlorine|well water|spoil|food safety|refrigerat|thaw|canning|botulism|mold|",
            // fuel, electrical, gas hazards
            r"generator|carbon monoxide|co detector|propane|natural gas|gas leak|gasoline|fuel|wiring|electrical|electrocut|voltage|breaker|outlet|",
            // law
            r"law|legal|illegal|permit|license|statute|regulation|ordinance|gun|firearm|weapon|carry|arrest|police",
            r")"
        ))
        .expect("valid regex")
    });
    ctx.asset.is_some() || re.is_match(question) || emergency_topic(question).is_some()
}

pub const GENERAL_SYSTEM_PROMPT: &str = "You are a friendly, knowledgeable assistant that runs entirely offline on the \
user's computer. Talk naturally, like a helpful expert: answer directly, explain clearly, and use Markdown (short \
paragraphs, numbered steps, bullet lists) when it helps. Keep answers focused; offer to go deeper rather than writing \
everything at once. If the question is ambiguous, ask one short clarifying question. Some numbered source passages from \
the user's offline library may be provided: when you state something taken from a passage, put its marker after it, \
like [S1]. Never invent markers. You may also use your own general knowledge without a marker. If the conversation \
turns to injuries, illness, medicines, drinking water safety, fuel or electrical hazards, or the law, tell the user to \
ask that as its own question so the library's verified sources are used.";

/// Emergency topics: procedures are shown word for word from the source, never paraphrased
/// by the model. A 4B model inverted tourniquet placement ("distal to the injury") while
/// citing a real passage; citations cannot catch an inversion. (topic, pattern, call 911)
pub const EMERGENCY_TOPICS: &[(&str, &str, bool)] = &[
    (
        "severe bleeding",
        r"(?i)\btourniquet|severe(ly)? bleed|bleeding (badly|heavily|a lot|won't stop)|stop (the )?bleeding|hemorrhag",
        true,
    ),
    ("cardiac arrest", r"(?i)\bcpr\b|not breathing|no pulse|rescue breath|chest compression|cardiac arrest", true),
    ("choking", r"(?i)chok(e|ing)|heimlich|abdominal thrust|airway (is )?blocked", true),
    ("shock", r"(?i)\b(in|into|for|treat(ing)?) shock\b|signs of shock|going into shock", true),
    ("burns", r"(?i)\bburn(s|ed)?\b", false),
    ("heat stroke", r"(?i)heat ?stroke|heat exhaustion", true),
    ("hypothermia", r"(?i)hypotherm|frostbite", true),
    ("carbon monoxide", r"(?i)carbon monoxide|\bco poisoning", true),
    ("snakebite", r"(?i)snake ?bite|bitten by a snake|venom", true),
    ("poisoning", r"(?i)poison(ed|ing)|swallowed (bleach|chemicals?|medicine|pills)|overdose", true),
    ("seizure", r"(?i)seizure|convuls", true),
    ("stroke", r"(?i)\bhaving a stroke|signs of (a )?stroke|\bstroke symptoms", true),
];

/// The emergency topic a question is about, if any.
pub fn emergency_topic(question: &str) -> Option<(&'static str, bool)> {
    static NOT_INJURY: OnceLock<Regex> = OnceLock::new();
    let not_injury = NOT_INJURY.get_or_init(|| {
        Regex::new(r"(?i)\bburn (ban|permit|pile|barrel|season)|outdoor burning").expect("valid regex")
    });
    if not_injury.is_match(question) && !question.to_lowercase().contains("burned") {
        return None;
    }
    static RES: OnceLock<Vec<(&'static str, Regex, bool)>> = OnceLock::new();
    RES.get_or_init(|| EMERGENCY_TOPICS.iter().map(|(t, p, c)| (*t, Regex::new(p).expect("valid regex"), *c)).collect())
        .iter()
        .find(|(_, re, _)| re.is_match(question))
        .map(|(t, _, c)| (*t, *c))
}

/// A directory of links (MedlinePlus topic pages: "Videos and Tutorials … (American Red
/// Cross) … PDF") rather than instructions.
fn is_link_list(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() < 4 {
        return false;
    }
    let short = lines.iter().filter(|l| l.split_whitespace().count() <= 6).count();
    let refs = lines.iter().filter(|l| l.contains("PDF") || (l.starts_with('(') && l.ends_with(')'))).count();
    short * 10 >= lines.len() * 6 && refs >= 1
}

/// How often the question's most distinctive word occurs in the passage.
fn topic_mentions(question: &str, text: &str, library: &Library, package: &str) -> usize {
    let terms = kb_evidence::content_terms(question);
    let Some((word, stem)) =
        terms.iter().max_by(|a, b| library.idf_in(package, &a.0).total_cmp(&library.idf_in(package, &b.0)))
    else {
        return 0;
    };
    let lower = text.to_lowercase();
    lower.matches(word.as_str()).count().max(lower.matches(stem.as_str()).count())
}

fn procedure_markers(text: &str) -> usize {
    text.matches('•').count()
        + text.lines().filter(|l| l.trim_start().starts_with("- ")).count()
        + max_step(text) as usize
}

/// Emergency answer without the model: the best one or two official/government procedure
/// passages, verbatim, each followed by its citation marker.
fn rank_emergency_passages(
    library: &Library,
    sources: &dyn SourceLookup,
    question: &str,
    assessment: &Assessment,
) -> Vec<(usize, Hit)> {
    // Consumer instructions: verified, public-audience passages first (a clinician guide's
    // dengue table mentioning a "tourniquet test" is not tourniquet instructions).
    let public = |h: &Hit| !sources.source(&h.package_id, &h.source_id).is_some_and(|s| s.for_clinicians());
    let pick = |verified_only: bool, public_only: bool| -> Vec<(usize, Hit)> {
        assessment
            .citable
            .iter()
            .enumerate()
            .filter(|(_, h)| !verified_only || assessment.verified_keys.contains(&h.citation_key()))
            .filter(|(_, h)| !public_only || public(h))
            .map(|(i, h)| (i, h.clone()))
            .collect()
    };
    let mut ranked = pick(true, true);
    if ranked.is_empty() {
        ranked = pick(false, true);
    }
    if ranked.is_empty() {
        ranked = pick(false, false);
    }
    // Prefer a procedure that starts at its first step, then passages that lay out steps;
    // keep retrieval order otherwise. Never lead with a fragment ("Step 3: …").
    static STEP_ONE: OnceLock<Regex> = OnceLock::new();
    let step_one = STEP_ONE.get_or_init(|| Regex::new(r"(?i)\bstep\s*1\b|(?m)^\s*1[.)]\s").expect("valid regex"));
    // Order: how well the passage matches the question (in tenths), then whether it is a
    // fragment, then how clearly it lays out steps, then retrieval order.
    // Order: not a link directory; how well it matches (in tenths); how often the topic word
    // recurs; not a fragment; how clearly it lays out steps; retrieval order.
    ranked.sort_by_key(|(i, h)| {
        let rel = (kb_evidence::relevance(question, h, library) * 10.0).round() as i32;
        let fragment = max_step(&h.text) > 0 && !step_one.is_match(&h.text);
        (
            is_link_list(&h.text),
            std::cmp::Reverse(rel),
            std::cmp::Reverse(topic_mentions(question, &h.text, library, &h.package_id).min(8)),
            fragment,
            std::cmp::Reverse(procedure_markers(&h.text).min(6)),
            *i,
        )
    });
    // Drop passages already contained in a better one (procedure expansion joins neighbours).
    let mut chosen: Vec<(usize, Hit)> = Vec::new();
    for (i, h) in ranked {
        let body = h.text.trim().to_string();
        if chosen.iter().any(|(_, c)| c.text.contains(&body) || body.contains(c.text.trim())) {
            continue;
        }
        chosen.push((i, h));
        if chosen.len() == 5 {
            break;
        }
    }
    chosen
}

pub fn card_slug(topic: &str) -> String {
    topic
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Emergency answer without the model. A curated card (an exact, reviewed section of an
/// official source) is shown verbatim. Without one, the user gets "call 911" and the closest
/// official passages to read directly; no single uncurated passage is presented as the answer.
#[allow(clippy::too_many_arguments)]
fn emergency_answer(
    library: &Library,
    sources: &dyn SourceLookup,
    question: &str,
    topic_q: &str,
    topic: &str,
    call_911: bool,
    assessment: Assessment,
    mut notes: Vec<String>,
) -> Answer {
    if call_911 {
        notes.insert(0, "If this is happening now, call 911 or your local emergency number first.".into());
    }
    let card_id = format!("card:{}", card_slug(topic));
    let card = library.package_ids().into_iter().find_map(|p| library.get(&p, &card_id).ok().flatten());
    if let Some(card) = card {
        let authoritative = sources.source(&card.package_id, &card.source_id).is_some_and(|s| {
            matches!(s.authority, kb_packages::Authority::Official | kb_packages::Authority::Government)
        });
        let source_title =
            sources.source(&card.package_id, &card.source_id).map(|s| s.title.clone()).unwrap_or_default();
        notes.push(format!("Emergency card: the exact text of {source_title}, not rewritten by the AI. Not yet clinically reviewed for this product."));
        let mut consulted = vec![card.clone()];
        consulted.extend(assessment.citable.iter().filter(|h| h.passage_id != card.passage_id).cloned());
        return Answer {
            question: question.into(),
            class: if authoritative { EvidenceClass::Verified } else { EvidenceClass::Partial },
            // Same words as the source; only inline bullets ("… • …") are laid out as a list.
            text: Some(format!(
                "**{}**\n\n{} [S1]",
                card.title,
                card.text.trim().replace(" • ", "\n- ").replace("• ", "\n- ")
            )),
            citations: vec![Citation {
                marker: 1,
                citation_key: card.citation_key(),
                package_id: card.package_id.clone(),
                passage_id: card.passage_id.clone(),
                doc_id: card.doc_id.clone(),
                title: card.title.clone(),
            }],
            consulted,
            notes,
            assessment,
            verification: None,
        };
    }
    let ranked = rank_emergency_passages(library, sources, topic_q, &assessment);
    notes.push(format!(
        "No reviewed emergency card for {topic} is installed yet, so the assistant will not summarize this. Read the official passages below directly."
    ));
    Answer {
        question: question.into(),
        class: if ranked.is_empty() { EvidenceClass::Unsupported } else { EvidenceClass::Partial },
        text: None,
        citations: vec![],
        consulted: ranked.into_iter().map(|(_, h)| h).collect(),
        notes,
        assessment,
        verification: None,
    }
}

fn step_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bstep\s*(\d{1,2})\b|(?m)^\s*(\d{1,2})[.)]\s").expect("valid regex"))
}

/// Highest step number in a text ("Step 4", or a numbered list item "4."), 0 if none.
pub fn max_step(text: &str) -> u32 {
    step_re()
        .captures_iter(text)
        .filter_map(|c| c.get(1).or(c.get(2)).and_then(|m| m.as_str().parse::<u32>().ok()))
        .filter(|n| *n <= 30)
        .max()
        .unwrap_or(0)
}

/// Conversational answer: library passages offered (cited if used), general knowledge allowed.
fn general_answer(
    question: &str,
    model: &dyn ChatModel,
    turn: &Turn,
    assessment: Assessment,
    mut notes: Vec<String>,
    on_token: &mut dyn FnMut(&str),
) -> Result<Answer, AgentError> {
    let mut prompt = build_context(turn);
    // Only passages that passed the relevance check, and fewer of them: faster first words.
    let shown: Vec<Hit> = assessment.citable.iter().take(4).cloned().collect();
    if !shown.is_empty() {
        prompt.push_str("Passages from the user's offline library that may help:\n");
        for (i, h) in shown.iter().enumerate() {
            prompt.push_str(&format!("[S{}] {}\n{}\n\n", i + 1, h.title, h.text.trim()));
        }
    }
    prompt.push_str("User: ");
    prompt.push_str(question);
    let raw = model.chat_stream(GENERAL_SYSTEM_PROMPT, &prompt, on_token)?;
    let v = verify_citations(&raw, &shown);
    if v.text.split_whitespace().count() < 2 {
        notes.push("The assistant did not produce an answer. Try rephrasing.".into());
        return Ok(Answer {
            question: question.into(),
            class: EvidenceClass::Unsupported,
            text: None,
            citations: vec![],
            consulted: shown,
            notes,
            assessment,
            verification: Some(v),
        });
    }
    notes.retain(|n| !n.starts_with("The installed library does not contain"));
    notes.push(if v.citations.is_empty() {
        "General knowledge from the assistant, not from your library. Double-check anything important.".into()
    } else {
        "Partly from your library (numbered sources) and partly the assistant's general knowledge.".into()
    });
    Ok(Answer {
        question: question.into(),
        class: EvidenceClass::General,
        text: Some(v.text.clone()),
        citations: v.citations.clone(),
        consulted: shown,
        notes,
        assessment,
        verification: Some(v),
    })
}

/// Lowest step number in a text, 0 if none.
pub fn min_step(text: &str) -> u32 {
    step_re()
        .captures_iter(text)
        .filter_map(|c| c.get(1).or(c.get(2)).and_then(|m| m.as_str().parse::<u32>().ok()))
        .filter(|n| *n <= 30)
        .min()
        .unwrap_or(0)
}

/// Procedures split across page breaks: when a passage contains numbered steps, attach
/// the following passage(s) of the same document (up to two) so the model sees the
/// whole procedure rather than a fragment that ends at "Step 2".
pub fn expand_procedures(library: &Library, hits: &mut [Hit]) {
    for h in hits.iter_mut() {
        if max_step(&h.text) == 0 {
            continue;
        }
        let Ok(doc) = library.document(&h.package_id, &h.doc_id) else { continue };
        let Some(pos) = doc.iter().position(|p| p.passage_id == h.passage_id) else { continue };
        // Starts mid-procedure ("Step 3: …")? Prepend the earlier steps from the previous passage.
        let first = min_step(&h.text);
        if first > 1 && pos > 0 {
            let prev = &doc[pos - 1];
            if max_step(&prev.text) > 0 && max_step(&prev.text) < first {
                h.text = format!("{}\n\n{}", prev.text.trim(), h.text);
            }
        }
        let mut last = max_step(&h.text);
        for next in doc.iter().skip(pos + 1).take(2) {
            let n = max_step(&next.text);
            h.text.push_str("\n\n");
            h.text.push_str(next.text.trim());
            // Stop once the next passage no longer continues the numbering.
            if n <= last {
                break;
            }
            last = n;
        }
    }
}

fn strip_markers(s: &str) -> String {
    marker_re().replace_all(s, "").to_string()
}

/// Conversation context ahead of the passages: saved notes (labelled as unverified) and
/// recent exchanges, answers shortened and without citation markers.
pub fn build_context(turn: &Turn) -> String {
    let mut s = String::new();
    if !turn.notes.is_empty() {
        s.push_str("What the user has told you about themselves (their own statements, not verified facts; never cite these as sources):\n");
        for n in &turn.notes {
            s.push_str("- ");
            s.push_str(n);
            s.push('\n');
        }
        s.push('\n');
    }
    if !turn.history.is_empty() {
        s.push_str("Earlier in this conversation (for context only; cite only the numbered passages below):\n");
        for (q, a) in turn.history.iter().rev().take(3).rev() {
            let a: String = strip_markers(a).chars().take(600).collect();
            s.push_str(&format!("User: {q}\nAssistant: {a}\n"));
        }
        s.push('\n');
    }
    s
}

/// Build the user turn: evidence notes, numbered passages, then the question.
pub fn build_prompt(question: &str, assessment: &Assessment, sources: &dyn SourceLookup) -> String {
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
        let pro = if sources.source(&h.package_id, &h.source_id).is_some_and(|s| s.for_clinicians()) {
            " (for health professionals)"
        } else {
            ""
        };
        s.push_str(&format!("[S{}] {}{}{}\n{}\n\n", i + 1, h.title, heading, pro, h.text.trim()));
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

    let uncited = uncited_sentences(&cleaned);

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

/// Sentences no citation covers. A marker covers the sentences before it in the same
/// paragraph, back to the previous marker (how people cite: "Claim. Claim. [S1]").
/// Sentences after a paragraph's last marker, or in a paragraph with none, are uncited.
fn uncited_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split("\n\n") {
        let mut pending: Vec<String> = Vec::new();
        for line in para.split('\n') {
            // Bullet and numbered-list items are separate claims: each needs its own marker.
            let is_item = line.trim_start().starts_with(['-', '*', '•'])
                || line.trim_start().chars().next().is_some_and(|c| c.is_ascii_digit());
            if is_item {
                out.append(&mut pending);
            }
            for sentence in split_sentences(line) {
                if marker_re().is_match(&sentence) {
                    pending.clear();
                } else if !is_hedge(&sentence)
                    && sentence.split_whitespace().count() >= 4
                    && !sentence.trim_end().ends_with(':')
                {
                    // A lead-in such as "Follow these steps:" introduces claims; it makes none.
                    pending.push(sentence);
                }
            }
            if is_item {
                out.append(&mut pending);
            }
        }
        out.append(&mut pending);
    }
    out
}

/// An "answer" that answers nothing: the question repeated back, or too short to state
/// anything. Citation markers on it must not make it Verified.
pub fn is_non_answer(question: &str, answer: &str) -> bool {
    let words = |t: &str| -> Vec<String> {
        strip_markers(t)
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect()
    };
    let a = words(answer);
    if a.len() < 4 {
        return true;
    }
    let q: std::collections::HashSet<String> = words(question).into_iter().collect();
    let novel = a.iter().filter(|w| !q.contains(*w)).count();
    // Nearly every word comes from the question and it ends as a question.
    strip_markers(answer).trim_end().ends_with('?') && novel * 5 < a.len()
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
    /// Show emergency procedures verbatim instead of asking the model (default on).
    pub extractive_emergencies: bool,
    /// Answer non-safety questions conversationally from general knowledge (default on).
    pub general_mode: bool,
}

impl Default for AskOptions {
    fn default() -> Self {
        Self { retrieve: 40, max_passages: 8, extractive_emergencies: true, general_mode: true }
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
    search_with(library, sources, question, None, ctx, opts)
}

/// Search with `question`; judge relevance against `relevance_text` when given (follow-ups).
pub fn search_with(
    library: &Library,
    sources: &dyn SourceLookup,
    question: &str,
    relevance_text: Option<&str>,
    ctx: &QuestionContext,
    opts: &AskOptions,
) -> Result<Assessment, AgentError> {
    // Search on what the question is about, not how it is phrased: "How do I…" in
    // page titles otherwise outranks the passages that answer the question.
    let terms: Vec<String> = kb_evidence::content_terms(question).into_iter().map(|(w, _)| w).collect();
    let query = if terms.is_empty() { question.to_string() } else { terms.join(" ") };
    let hits = library.search(&query, opts.retrieve)?;
    let mut a = assess(relevance_text.unwrap_or(question), ctx, &hits, sources, library);
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
    ask_with(library, sources, model, question, ctx, opts, &Turn::default(), &mut |_| {})
}

/// One conversational turn: retrieval uses `turn.retrieval_text` (follow-ups include the
/// previous question), the prompt carries notes and recent history, tokens stream through
/// `on_token`, and the final answer is verified exactly as in `ask`.
#[allow(clippy::too_many_arguments)]
pub fn ask_with(
    library: &Library,
    sources: &dyn SourceLookup,
    model: &dyn ChatModel,
    question: &str,
    ctx: &QuestionContext,
    opts: &AskOptions,
    turn: &Turn,
    on_token: &mut dyn FnMut(&str),
) -> Result<Answer, AgentError> {
    let search_text = turn.retrieval_text.as_deref().unwrap_or(question);
    let mut assessment = search_with(library, sources, search_text, turn.topic_text.as_deref(), ctx, opts)?;
    expand_procedures(library, &mut assessment.citable);
    let mut notes = assessment.notes.clone();
    if assessment
        .citable
        .iter()
        .any(|h| sources.source(&h.package_id, &h.source_id).is_some_and(|s| s.for_clinicians()))
    {
        notes.push("Some sources here are written for health professionals. Steps that need trained care are not things to attempt yourself; get medical help.".into());
    }

    // Emergencies: official text verbatim, no model (instant, and nothing can be inverted).
    let topic_q = turn.topic_text.as_deref().map(|t| format!("{t} {question}")).unwrap_or_else(|| question.to_string());
    if let Some((topic, call_911)) = emergency_topic(&topic_q).filter(|_| opts.extractive_emergencies) {
        return Ok(emergency_answer(library, sources, question, &topic_q, topic, call_911, assessment, notes));
    }
    // Non-safety topics: conversational, general knowledge allowed, labelled as such.
    if opts.general_mode && !needs_strict_mode(&topic_q, ctx) {
        return general_answer(question, model, turn, assessment, notes, on_token);
    }
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

    let prompt = build_context(turn) + &build_prompt(question, &assessment, sources);
    let raw = model.chat_stream(SYSTEM_PROMPT, &prompt, on_token)?;
    let v = verify_citations(&raw, &assessment.citable);

    let mut class = assessment.class;
    let mut text = Some(v.text.clone());
    if is_non_answer(question, &v.text) {
        // e.g. the model repeated the question with a citation attached: nothing was answered.
        return Ok(Answer {
            question: question.into(),
            class: EvidenceClass::Unsupported,
            text: None,
            citations: vec![],
            consulted: assessment.citable.clone(),
            notes: {
                let mut n = notes;
                n.push("The assistant could not produce an answer from the passages it found. The closest passages are listed below.".into());
                n
            },
            assessment,
            verification: Some(v),
        });
    }
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
        // Procedural completeness: cited sources that number their steps to N must be
        // answered through step N. A cited but partial procedure is not Verified.
        let source_steps = v
            .citations
            .iter()
            .filter_map(|c| assessment.citable.get(c.marker - 1))
            .map(|h| max_step(&h.text))
            .max()
            .unwrap_or(0);
        let answer_steps = max_step(&v.text);
        if source_steps >= 2 && answer_steps < source_steps {
            if class == EvidenceClass::Verified {
                class = EvidenceClass::Partial;
            }
            notes.push(format!(
                "The source describes a procedure with {source_steps} steps; this answer may leave some out. Open the cited source for the complete instructions."
            ));
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
    fn a_marker_covers_preceding_sentences_in_its_paragraph() {
        // Real 4B answer shape (generator question).
        let v = verify_citations(
            "No, you should never run a generator inside a garage, even if the door is open. \
             It can lead to carbon monoxide poisoning [S1].",
            &[hit("a")],
        );
        assert!(v.uncited_sentences.is_empty(), "{:?}", v.uncited_sentences);
        // A claim after the last marker is still uncited.
        let v = verify_citations("Boil water for one minute [S1]. Then add a pinch of salt for taste.", &[hit("a")]);
        assert_eq!(v.uncited_sentences, vec!["Then add a pinch of salt for taste."]);
        // A marker does not reach into the previous paragraph.
        let v = verify_citations("Keep the generator outside at all times.\n\nUse a CO alarm [S1].", &[hit("a")]);
        assert_eq!(v.uncited_sentences, vec!["Keep the generator outside at all times."]);
        // Lead-ins make no claim.
        let v = verify_citations(
            "To signal for rescue, follow these steps:\n- Use a mirror to flash aircraft [S1]",
            &[hit("a")],
        );
        assert!(v.uncited_sentences.is_empty(), "{:?}", v.uncited_sentences);
        // Each list item needs its own marker.
        let v = verify_citations(
            "- Cool the burn with water for twenty minutes\n- Cover it with clean cloth [S1]",
            &[hit("a")],
        );
        assert_eq!(v.uncited_sentences, vec!["- Cool the burn with water for twenty minutes"]);
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
                audience: None,
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
    fn clinician_sources_are_labelled_and_noted() {
        struct Capture(std::sync::Mutex<String>);
        impl ChatModel for Capture {
            fn chat(&self, _s: &str, u: &str) -> Result<String, AgentError> {
                *self.0.lock().unwrap() = u.to_string();
                Ok("Do not run a generator in a garage because of carbon monoxide [S1].".into())
            }
        }
        let (_d, lib, mut src) = setup();
        src.get_mut(&("pkg".to_string(), "fema".to_string())).unwrap().audience = Some("clinician".into());
        let m = Capture(std::sync::Mutex::new(String::new()));
        let a = ask(
            &lib,
            &src,
            &m,
            "Can I run my generator in the garage?",
            &QuestionContext::default(),
            &AskOptions::default(),
        )
        .unwrap();
        assert!(m.0.lock().unwrap().contains("(for health professionals)"));
        assert!(a.notes.iter().any(|n| n.contains("health professionals")));
    }

    fn procedure_library(
    ) -> (tempfile::TempDir, Library, std::collections::HashMap<(String, String), kb_packages::SourceRecord>) {
        let dir = tempfile::tempdir().unwrap();
        let p = |id: &str, text: &str| kb_retrieval::Passage {
            passage_id: id.into(),
            doc_id: "cert".into(),
            source_id: "cert".into(),
            title: "CERT Basic Training Participant Manual".into(),
            heading: String::new(),
            text: text.into(),
        };
        kb_retrieval::build_index(
            dir.path(),
            &[
                p("c1", "A tourniquet stops severe bleeding from an arm or leg. How to use a tourniquet: Step 1: Place it as high as possible on the injured limb. Step 2: Pull the strap through the buckle."),
                p("c2", "August 2019 Page 3-4. Step 3: Twist the windlass until the bleeding stops. Step 4: Lock the windlass in place and note the time the tourniquet was applied."),
                p("c3", "Unit 4 covers light search and rescue operations in damaged buildings."),
            ],
        )
        .unwrap();
        let mut lib = Library::new();
        lib.mount("pkg", dir.path()).unwrap();
        let mut src = std::collections::HashMap::new();
        src.insert(
            ("pkg".to_string(), "cert".to_string()),
            kb_packages::SourceRecord {
                id: "cert".into(),
                title: "CERT".into(),
                publisher: "FEMA".into(),
                license: "LicenseRef-PublicDomain-USGov".into(),
                authority: kb_packages::Authority::Government,
                revision: None,
                superseded_by: None,
                applies_to: vec![],
                jurisdictions: vec![],
                url: None,
                audience: None,
            },
        );
        (dir, lib, src)
    }

    #[test]
    fn procedure_split_across_passages_is_joined_and_checked() {
        struct Capture(std::sync::Mutex<String>, &'static str);
        impl ChatModel for Capture {
            fn chat(&self, _s: &str, u: &str) -> Result<String, AgentError> {
                *self.0.lock().unwrap() = u.to_string();
                Ok(self.1.to_string())
            }
        }
        let (_d, lib, src) = procedure_library();
        let q = "How do I apply a tourniquet?";

        // The model only gives the first two steps (what happened on the Intel Mac test).
        // Model path (emergency mode off) to exercise the completeness check itself.
        let opts = AskOptions { extractive_emergencies: false, ..AskOptions::default() };
        let partial = Capture(
            std::sync::Mutex::new(String::new()),
            "1. Place it high on the limb [S1].\n2. Pull the strap through the buckle [S1].",
        );
        let a = ask(&lib, &src, &partial, q, &QuestionContext::default(), &opts).unwrap();
        assert!(
            partial.0.lock().unwrap().contains("Step 4: Lock the windlass"),
            "continuation passage reaches the model"
        );
        assert_eq!(a.class, EvidenceClass::Partial, "a cited but partial procedure is not Verified");
        assert!(a.notes.iter().any(|n| n.contains("4 steps")));

        // A complete answer stays Verified.
        let full = Capture(
            std::sync::Mutex::new(String::new()),
            "1. Place it high on the limb [S1].\n2. Pull the strap through the buckle [S1].\n3. Twist the windlass until bleeding stops [S1].\n4. Lock it and note the time [S1].",
        );
        let a = ask(&lib, &src, &full, q, &QuestionContext::default(), &opts).unwrap();
        assert_eq!(a.class, EvidenceClass::Verified);
    }

    #[test]
    fn follow_up_uses_retrieval_text_history_and_notes() {
        struct Capture(std::sync::Mutex<String>);
        impl ChatModel for Capture {
            fn chat(&self, _s: &str, u: &str) -> Result<String, AgentError> {
                *self.0.lock().unwrap() = u.to_string();
                Ok("Keep it outdoors, away from windows [S1].".into())
            }
        }
        let (_d, lib, src) = setup();
        let m = Capture(std::sync::Mutex::new(String::new()));
        let turn = Turn {
            history: vec![(
                "Can I run my generator in the garage?".into(),
                "No [S1]. Carbon monoxide can kill.".into(),
            )],
            notes: vec!["We have a Generac GP6500".into()],
            retrieval_text: Some("Can I run my generator in the garage? What about with the door open?".into()),
            topic_text: Some("Can I run my generator in the garage?".into()),
        };
        let mut streamed = String::new();
        let a = ask_with(
            &lib,
            &src,
            &m,
            "What about with the door open?",
            &QuestionContext::default(),
            &AskOptions::default(),
            &turn,
            &mut |t| streamed.push_str(t),
        )
        .unwrap();
        let prompt = m.0.lock().unwrap().clone();
        assert!(prompt.contains("We have a Generac GP6500") && prompt.contains("not verified facts"));
        assert!(prompt.contains("User: Can I run my generator in the garage?"));
        assert!(!prompt.contains("No [S1]"), "markers from earlier answers are stripped");
        assert_eq!(a.class, EvidenceClass::Verified, "the follow-up retrieved the generator passage");
        assert!(streamed.contains("outdoors"));
    }

    struct Panics;
    impl ChatModel for Panics {
        fn chat(&self, _s: &str, _u: &str) -> Result<String, AgentError> {
            panic!("the model must not paraphrase an emergency procedure")
        }
    }

    #[test]
    fn emergency_without_card_lists_passages_and_never_calls_the_model() {
        let (_d, lib, src) = procedure_library();
        let a = ask(
            &lib,
            &src,
            &Panics,
            "How do I apply a tourniquet?",
            &QuestionContext::default(),
            &AskOptions::default(),
        )
        .unwrap();
        assert!(a.text.is_none(), "no uncurated passage is presented as the answer");
        assert_eq!(a.class, EvidenceClass::Partial);
        assert!(!a.consulted.is_empty());
        assert!(a.notes[0].contains("call 911"));
        assert!(a.notes.iter().any(|n| n.contains("No reviewed emergency card")));

        assert_eq!(emergency_topic("Someone is choking, what do I do?").map(|t| t.0), Some("choking"));
        assert_eq!(emergency_topic("What are signs of heat stroke?").map(|t| t.0), Some("heat stroke"));
        assert_eq!(emergency_topic("How do I treat a burn on my arm?").map(|t| t.0), Some("burns"));
        assert!(emergency_topic("Is there a burn ban in my county?").is_none());
        assert!(emergency_topic("Can I do outdoor burning of brush?").is_none());
        assert!(emergency_topic("How do I purify river water?").is_none());
        assert!(emergency_topic("Can I run a generator in my garage?").is_none());
    }

    #[test]
    fn emergency_card_is_shown_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let card_text = "One-Handed (Windlass) TQ Application: Position the tourniquet high on the limb. Twist the rod until bleeding has stopped. Lock the windlass.";
        kb_retrieval::build_index(
            dir.path(),
            &[
                kb_retrieval::Passage {
                    passage_id: "card:severe-bleeding".into(),
                    doc_id: "card:severe-bleeding".into(),
                    source_id: "cert".into(),
                    title: "Applying a tourniquet".into(),
                    heading: String::new(),
                    text: card_text.into(),
                },
                kb_retrieval::Passage {
                    passage_id: "x1".into(),
                    doc_id: "x".into(),
                    source_id: "cert".into(),
                    title: "Bleeding".into(),
                    heading: String::new(),
                    text: "Apply a tourniquet for severe bleeding from a limb when pressure fails.".into(),
                },
            ],
        )
        .unwrap();
        let mut lib = Library::new();
        lib.mount("pkg", dir.path()).unwrap();
        let (_d2, _l2, src) = procedure_library();
        let a = ask(
            &lib,
            &src,
            &Panics,
            "How do I apply a tourniquet?",
            &QuestionContext::default(),
            &AskOptions::default(),
        )
        .unwrap();
        let text = a.text.unwrap();
        assert!(text.contains(card_text), "card text verbatim: {text}");
        assert_eq!(a.citations[0].passage_id, "card:severe-bleeding");
        assert_eq!(a.consulted[0].passage_id, "card:severe-bleeding");
        assert_eq!(a.class, EvidenceClass::Verified);
        assert!(a.notes.iter().any(|n| n.contains("Emergency card")));
    }

    #[test]
    fn echoed_question_is_not_an_answer() {
        let q = "How far away from the house should it be?";
        assert!(is_non_answer(q, "How far away from the house should it be? [S4]"));
        assert!(is_non_answer(q, "[S1]"));
        assert!(!is_non_answer(q, "Keep it at least 20 feet from the house, with the exhaust pointed away [S1]."));
        assert!(!is_non_answer("Is it safe?", "No. Never run a generator in a garage [S1]."));

        struct Echo;
        impl ChatModel for Echo {
            fn chat(&self, _s: &str, _u: &str) -> Result<String, AgentError> {
                Ok("Can I run my generator in the garage? [S1]".into())
            }
        }
        let (_d, lib, src) = setup();
        let a = ask(
            &lib,
            &src,
            &Echo,
            "Can I run my generator in the garage?",
            &QuestionContext::default(),
            &AskOptions::default(),
        )
        .unwrap();
        assert_eq!(a.class, EvidenceClass::Unsupported);
        assert!(a.text.is_none());
        assert!(!a.consulted.is_empty(), "passages are still offered to the user");
    }

    #[test]
    fn strict_and_general_topics() {
        let none = QuestionContext::default();
        for q in [
            "How do I treat a burn?",
            "What dose of ibuprofen?",
            "How do I purify river water?",
            "Can I run a generator in my garage?",
            "Do I need a permit to carry a handgun?",
            "My kid has a fever",
            "Is this food spoiled?",
            "How do I wire an outlet?",
        ] {
            assert!(needs_strict_mode(q, &none), "{q}");
        }
        for q in [
            "hi",
            "thanks!",
            "How does a well pump work?",
            "Help me plan a vegetable garden",
            "Explain photosynthesis to my kid",
            "What's a good way to organize a pantry?",
            "How do I write a list comprehension in Python?",
        ] {
            assert!(!needs_strict_mode(q, &none), "{q}");
        }
        let asset = QuestionContext { asset: Some("generac:gp6500".into()), jurisdiction: None };
        assert!(
            needs_strict_mode("How often do I change the oil?", &asset),
            "equipment-specific questions stay strict"
        );
    }

    #[test]
    fn general_mode_answers_conversationally() {
        struct Chatty;
        impl ChatModel for Chatty {
            fn chat(&self, s: &str, _u: &str) -> Result<String, AgentError> {
                assert!(s.contains("runs entirely offline"), "general prompt used");
                Ok("Hi! I can help with gardening, repairs, school topics and more. What are you working on? [S9]"
                    .into())
            }
        }
        let (_d, lib, src) = setup();
        let a = ask(&lib, &src, &Chatty, "hi", &QuestionContext::default(), &AskOptions::default()).unwrap();
        assert_eq!(a.class, EvidenceClass::General);
        let text = a.text.unwrap();
        assert!(text.starts_with("Hi!") && !text.contains("[S9]"), "invented markers removed: {text}");
        assert!(a.notes.iter().any(|n| n.contains("General knowledge")));

        // Safety topics never go to general mode, even with general mode on.
        struct Strict;
        impl ChatModel for Strict {
            fn chat(&self, s: &str, _u: &str) -> Result<String, AgentError> {
                assert!(s.contains("ONLY the numbered source passages"), "strict prompt for a hazard question");
                Ok("Never run a generator in a garage; carbon monoxide can kill [S1].".into())
            }
        }
        let a = ask(
            &lib,
            &src,
            &Strict,
            "Can I run my generator in the garage?",
            &QuestionContext::default(),
            &AskOptions::default(),
        )
        .unwrap();
        assert_eq!(a.class, EvidenceClass::Verified);
    }

    #[test]
    fn step_counting() {
        assert_eq!(max_step("Step 1: x. Step 2: y."), 2);
        assert_eq!(max_step("1. a\n2. b\n3. c"), 3);
        assert_eq!(max_step("In 1990 there were 40 cases."), 0);
        assert_eq!(max_step("Call 911."), 0);
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
            "What dose of amoxicillin should a toddler take?",
            &QuestionContext::default(),
            &AskOptions::default(),
        )
        .unwrap();
        assert_eq!(a.class, EvidenceClass::Unsupported);
    }
}
