//! Evidence classification. See `spec/evidence-classes.md`.
//!
//! The class of an answer is decided here, from facts in signed manifests
//! (document identity, equipment applicability, revision/supersession,
//! jurisdiction, authority) and from what retrieval actually returned. The
//! language model never decides it, and it cannot raise it.

use kb_packages::{Authority, SourceRecord};
use kb_retrieval::Hit;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    /// No reliable answer can be established from the installed sources.
    Unsupported,
    /// Relevant material exists, but not the exact, current, applicable document.
    Partial,
    /// An authoritative, current document that applies to this exact equipment
    /// or jurisdiction was retrieved and is citable.
    Verified,
}

/// What the question is about, supplied by the asset registry and user settings,
/// not guessed by the model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuestionContext {
    /// Normalised `make:model` of the equipment the question concerns.
    pub asset: Option<String>,
    /// Jurisdiction code such as "US-TX".
    pub jurisdiction: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionReason {
    /// Written for different equipment than the question concerns.
    WrongEquipment,
    /// Written for a different jurisdiction.
    WrongJurisdiction,
    /// A newer revision is installed; the newer one is used instead.
    SupersededByInstalled,
    /// Too little overlap with the question to count as relevant.
    NotRelevant,
    /// The manifest has no record for this source (packaging error).
    UnknownSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exclusion {
    pub citation_key: String,
    pub reason: ExclusionReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assessment {
    pub class: EvidenceClass,
    /// Hits the answer may cite, best first. The citation verifier enforces this set.
    pub citable: Vec<Hit>,
    /// Citation keys within `citable` that meet the verified bar.
    pub verified_keys: Vec<String>,
    pub excluded: Vec<Exclusion>,
    /// Plain-language notes shown to the user with the answer.
    pub notes: Vec<String>,
}

/// Looks up the source record for a hit. Implemented over mounted manifests.
pub trait SourceLookup {
    fn source(&self, package_id: &str, source_id: &str) -> Option<&SourceRecord>;
}

impl SourceLookup for HashMap<(String, String), SourceRecord> {
    fn source(&self, package_id: &str, source_id: &str) -> Option<&SourceRecord> {
        self.get(&(package_id.to_string(), source_id.to_string()))
    }
}

/// Normalise "Acme GX-2000" style input to "acme:gx-2000" when given make and model.
pub fn asset_key(make: &str, model: &str) -> String {
    format!("{}:{}", norm(make), norm(model))
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-")
}

const STOPWORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "how",
    "what",
    "when",
    "where",
    "why",
    "who",
    "does",
    "can",
    "should",
    "my",
    "our",
    "your",
    "you",
    "are",
    "was",
    "were",
    "this",
    "that",
    "from",
    "into",
    "about",
    "will",
    "would",
    "have",
    "has",
    "had",
    "not",
    "but",
    "its",
    "it's",
    "his",
    "her",
    "they",
    "them",
    "there",
    "their",
    "then",
    "than",
    "out",
    "get",
    "use",
    "using",
    "used",
    "need",
    "needs",
    "any",
    "all",
    "one",
    "which",
    "also",
    "after",
    "before",
    "while",
    "over",
    "under",
    "make",
    "made",
    "do",
    "is",
    "it",
    "to",
    "of",
    "in",
    "on",
    "a",
    "an",
    "or",
    "be",
    "if",
    "at",
    "by",
    "as",
    "i",
    "me",
    "we",
    // question framing: how a question is asked, not what it is about
    "often",
    "much",
    "many",
    "long",
    "best",
    "way",
    "ways",
    "want",
    "know",
    "tell",
    "please",
    "could",
    "really",
    "got",
    "take",
    "still",
    "ever",
    "every",
    "some",
    "kind",
    "type",
    "thing",
    "things",
    "someone",
    "something",
    "correct",
    "correctly",
    "properly",
    "right",
    "good",
    "basic",
    "safely",
    "exactly",
    "possible",
    "happen",
];

/// Light suffix stripping so "applying", "applied" and "apply" compare equal.
/// Applied until stable, so every form of a word lands on the same root.
pub fn stem(w: &str) -> String {
    let mut cur = w.to_string();
    for _ in 0..3 {
        let next = stem_once(&cur);
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

fn stem_once(w: &str) -> String {
    for (suf, rep) in [
        ("ingly", ""),
        ("ings", ""),
        ("ing", ""),
        ("edly", ""),
        ("ied", "y"),
        ("ies", "y"),
        ("ment", ""),
        ("ed", ""),
        ("es", ""),
        ("ly", ""),
        ("s", ""),
    ] {
        if let Some(base) = w.strip_suffix(suf) {
            if base.chars().count() >= 3 {
                return format!("{base}{rep}");
            }
        }
    }
    w.to_string()
}

fn words(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_alphanumeric()).map(|w| w.to_lowercase()).filter(|w| w.len() >= 3)
}

/// Question words that carry meaning: (raw lowercase word, stem).
pub fn content_terms(s: &str) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    words(s)
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .filter_map(|w| {
            let st = stem(&w);
            seen.insert(st.clone()).then_some((w, st))
        })
        .collect()
}

/// How much a word distinguishes one passage from another (rarer = higher).
pub trait TermWeights {
    fn weight(&self, word: &str) -> f32;
}

/// Every word counts the same. For tests and callers without a library.
pub struct Uniform;
impl TermWeights for Uniform {
    fn weight(&self, _word: &str) -> f32 {
        1.0
    }
}

impl TermWeights for kb_retrieval::Library {
    fn weight(&self, word: &str) -> f32 {
        self.idf(word)
    }
}

/// A word must outweigh the next by this factor to be required in every passage.
pub const ANCHOR_RATIO: f32 = 1.5;

/// Share of the question's weight a passage must cover to count as relevant.
pub const RELEVANCE_COVERAGE: f32 = 0.55;

/// Deterministic relevance: the passage must cover most of the question's
/// *distinctive* words (weighted by rarity), compared by stem.
pub fn relevance(question: &str, hit: &Hit, weights: &dyn TermWeights) -> f32 {
    let q = content_terms(question);
    if q.is_empty() {
        return 0.0;
    }
    let h: HashSet<String> = words(&format!("{} {} {}", hit.title, hit.heading, hit.text)).map(|w| stem(&w)).collect();
    let (mut total, mut covered) = (0.0f32, 0.0f32);
    let mut scored: Vec<(f32, bool)> = Vec::new();
    for (w, st) in &q {
        let wt = weights.weight(w).max(0.01);
        total += wt;
        let hit_here = h.contains(st);
        if hit_here {
            covered += wt;
        }
        scored.push((wt, hit_here));
    }
    // When one word dominates the question (the drug, the injury, the equipment), it
    // must appear: a passage about some other drug's dose is not evidence, however much
    // of the rest it shares. Near-ties (e.g. "warmth" vs "wilderness") fall back to coverage.
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    if let Some(&(top, top_hit)) = scored.first() {
        let second = scored.get(1).map(|x| x.0).unwrap_or(0.0);
        if top >= ANCHOR_RATIO * second && !top_hit {
            return 0.0;
        }
    }
    covered / total
}

pub fn is_relevant(question: &str, hit: &Hit, weights: &dyn TermWeights) -> bool {
    relevance(question, hit, weights) >= RELEVANCE_COVERAGE
}

fn jurisdiction_matches(wanted: &str, doc: &[String]) -> Option<bool> {
    if doc.is_empty() {
        return None; // not jurisdiction specific
    }
    let wanted = wanted.to_uppercase();
    Some(doc.iter().any(|j| {
        let j = j.to_uppercase();
        wanted == j || wanted.starts_with(&format!("{j}-"))
    }))
}

pub fn assess(
    question: &str,
    ctx: &QuestionContext,
    hits: &[Hit],
    sources: &dyn SourceLookup,
    weights: &dyn TermWeights,
) -> Assessment {
    let mut excluded = Vec::new();
    let mut notes = Vec::new();

    // Pass 1: drop irrelevant, unknown, wrong-equipment and wrong-jurisdiction hits.
    let mut candidates: Vec<(&Hit, &SourceRecord)> = Vec::new();
    for h in hits {
        let key = h.citation_key();
        let Some(src) = sources.source(&h.package_id, &h.source_id) else {
            excluded.push(Exclusion { citation_key: key, reason: ExclusionReason::UnknownSource });
            continue;
        };
        if !is_relevant(question, h, weights) {
            excluded.push(Exclusion { citation_key: key, reason: ExclusionReason::NotRelevant });
            continue;
        }
        if !src.applies_to.is_empty() {
            let fits = ctx.asset.as_ref().is_some_and(|a| src.applies_to.iter().any(|x| x == a));
            if !fits {
                excluded.push(Exclusion { citation_key: key, reason: ExclusionReason::WrongEquipment });
                continue;
            }
        }
        if let Some(j) = &ctx.jurisdiction {
            if jurisdiction_matches(j, &src.jurisdictions) == Some(false) {
                excluded.push(Exclusion { citation_key: key, reason: ExclusionReason::WrongJurisdiction });
                continue;
            }
        }
        candidates.push((h, src));
    }

    // Pass 2: prefer current revisions. A superseded document is dropped when its
    // replacement is among the candidates; otherwise it stays but caps the class.
    let present: HashSet<(&str, &str)> =
        candidates.iter().map(|(h, s)| (h.package_id.as_str(), s.id.as_str())).collect();
    let mut kept: Vec<(&Hit, &SourceRecord)> = Vec::new();
    let mut superseded_used = Vec::new();
    for (h, s) in candidates {
        if let Some(newer) = &s.superseded_by {
            if present.contains(&(h.package_id.as_str(), newer.as_str())) {
                excluded
                    .push(Exclusion { citation_key: h.citation_key(), reason: ExclusionReason::SupersededByInstalled });
                continue;
            }
            superseded_used.push(s.title.clone());
        }
        kept.push((h, s));
    }

    // Most authoritative first (official, government, reference, community), then score:
    // reference works fill gaps but never displace an applicable official source.
    kept.sort_by(|(ha, sa), (hb, sb)| {
        sa.authority.cmp(&sb.authority).then(hb.score.partial_cmp(&ha.score).unwrap_or(std::cmp::Ordering::Equal))
    });

    // Pass 3: which kept hits meet the verified bar?
    let verified_keys: Vec<String> = kept
        .iter()
        .filter(|(_, s)| s.superseded_by.is_none())
        .filter(|(_, s)| match (&ctx.asset, &ctx.jurisdiction) {
            (Some(a), _) => s.applies_to.iter().any(|x| x == a) && s.authority == Authority::Official,
            (None, Some(j)) => {
                jurisdiction_matches(j, &s.jurisdictions) == Some(true)
                    && matches!(s.authority, Authority::Official | Authority::Government)
            }
            (None, None) => matches!(s.authority, Authority::Official | Authority::Government),
        })
        .map(|(h, _)| h.citation_key())
        .collect();

    let class = if kept.is_empty() {
        EvidenceClass::Unsupported
    } else if !verified_keys.is_empty() {
        EvidenceClass::Verified
    } else {
        EvidenceClass::Partial
    };

    if let Some(a) = &ctx.asset {
        let has_exact = kept.iter().any(|(_, s)| s.applies_to.iter().any(|x| x == a));
        if !has_exact {
            notes.push(format!(
                "No documentation written for {a} is installed. Anything below is general guidance, not model-specific."
            ));
        }
    }
    for t in &superseded_used {
        notes.push(format!(
            "\"{t}\" has been superseded and its replacement is not installed. Check for a newer revision."
        ));
    }
    if class == EvidenceClass::Unsupported {
        notes.push("The installed library does not contain enough to answer this reliably.".to_string());
    }

    Assessment { class, citable: kept.into_iter().map(|(h, _)| h.clone()).collect(), verified_keys, excluded, notes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(id: &str, auth: Authority, applies: &[&str], juris: &[&str], superseded_by: Option<&str>) -> SourceRecord {
        SourceRecord {
            id: id.into(),
            title: format!("Title {id}"),
            publisher: "pub".into(),
            license: "LicenseRef-Test".into(),
            authority: auth,
            revision: None,
            superseded_by: superseded_by.map(String::from),
            applies_to: applies.iter().map(|s| s.to_string()).collect(),
            jurisdictions: juris.iter().map(|s| s.to_string()).collect(),
            url: None,
        }
    }

    fn hit(pid: &str, sid: &str, text: &str) -> Hit {
        Hit {
            package_id: "pkg".into(),
            passage_id: pid.into(),
            doc_id: sid.into(),
            source_id: sid.into(),
            title: String::new(),
            heading: String::new(),
            text: text.into(),
            score: 1.0,
        }
    }

    fn lookup(srcs: Vec<SourceRecord>) -> HashMap<(String, String), SourceRecord> {
        srcs.into_iter().map(|s| (("pkg".to_string(), s.id.clone()), s)).collect()
    }

    const Q: &str = "How do I change the oil filter on the generator?";
    const T: &str = "To change the oil filter on the generator, drain the oil first.";

    #[test]
    fn exact_manual_is_verified() {
        let l = lookup(vec![src("man", Authority::Official, &["acme:gx-2000"], &[], None)]);
        let ctx = QuestionContext { asset: Some("acme:gx-2000".into()), jurisdiction: None };
        let a = assess(Q, &ctx, &[hit("p1", "man", T)], &l, &Uniform);
        assert_eq!(a.class, EvidenceClass::Verified);
        assert_eq!(a.verified_keys, vec!["pkg/p1"]);
    }

    #[test]
    fn generic_material_for_specific_asset_is_partial_with_note() {
        let l = lookup(vec![src("gen", Authority::Government, &[], &[], None)]);
        let ctx = QuestionContext { asset: Some("cat:3208".into()), jurisdiction: None };
        let a = assess(Q, &ctx, &[hit("p1", "gen", T)], &l, &Uniform);
        assert_eq!(a.class, EvidenceClass::Partial);
        assert!(a.notes[0].contains("cat:3208"));
    }

    #[test]
    fn other_equipment_manual_is_excluded() {
        let l = lookup(vec![src("man", Authority::Official, &["acme:gx-2000"], &[], None)]);
        let ctx = QuestionContext { asset: Some("acme:gx-3000".into()), jurisdiction: None };
        let a = assess(Q, &ctx, &[hit("p1", "man", T)], &l, &Uniform);
        assert_eq!(a.class, EvidenceClass::Unsupported);
        assert_eq!(a.excluded[0].reason, ExclusionReason::WrongEquipment);
        // And with no asset selected, an equipment-specific manual is not applied either.
        let a = assess(Q, &QuestionContext::default(), &[hit("p1", "man", T)], &l, &Uniform);
        assert_eq!(a.class, EvidenceClass::Unsupported);
    }

    #[test]
    fn superseded_dropped_when_replacement_present() {
        let l = lookup(vec![
            src("revA", Authority::Official, &["acme:gx-2000"], &[], Some("revB")),
            src("revB", Authority::Official, &["acme:gx-2000"], &[], None),
        ]);
        let ctx = QuestionContext { asset: Some("acme:gx-2000".into()), jurisdiction: None };
        let a = assess(Q, &ctx, &[hit("old", "revA", T), hit("new", "revB", T)], &l, &Uniform);
        assert_eq!(a.class, EvidenceClass::Verified);
        assert_eq!(a.citable.len(), 1);
        assert_eq!(a.citable[0].passage_id, "new");
        assert_eq!(a.excluded[0].reason, ExclusionReason::SupersededByInstalled);
    }

    #[test]
    fn superseded_alone_caps_at_partial() {
        let l = lookup(vec![src("revA", Authority::Official, &["acme:gx-2000"], &[], Some("revB"))]);
        let ctx = QuestionContext { asset: Some("acme:gx-2000".into()), jurisdiction: None };
        let a = assess(Q, &ctx, &[hit("old", "revA", T)], &l, &Uniform);
        assert_eq!(a.class, EvidenceClass::Partial);
        assert!(a.notes.iter().any(|n| n.contains("superseded")));
    }

    #[test]
    fn jurisdiction_rules() {
        let l = lookup(vec![
            src("tx", Authority::Official, &[], &["US-TX"], None),
            src("ca", Authority::Official, &[], &["US-CA"], None),
            src("us", Authority::Government, &[], &["US"], None),
        ]);
        let q = "What is the speed limit rule for school zones?";
        let t = "The speed limit rule for school zones is set by statute.";
        let ctx = QuestionContext { asset: None, jurisdiction: Some("US-TX".into()) };
        let a = assess(q, &ctx, &[hit("1", "tx", t), hit("2", "ca", t), hit("3", "us", t)], &l, &Uniform);
        assert_eq!(a.class, EvidenceClass::Verified);
        let keys: Vec<_> = a.citable.iter().map(|h| h.passage_id.as_str()).collect();
        assert_eq!(keys, vec!["1", "3"]);
        assert!(a.excluded.iter().any(|e| e.reason == ExclusionReason::WrongJurisdiction));
    }

    #[test]
    fn community_only_is_partial_and_irrelevant_is_unsupported() {
        let l = lookup(vec![src("se", Authority::Community, &[], &[], None)]);
        let a = assess(Q, &QuestionContext::default(), &[hit("1", "se", T)], &l, &Uniform);
        assert_eq!(a.class, EvidenceClass::Partial);
        let a =
            assess(Q, &QuestionContext::default(), &[hit("1", "se", "Bread rises when yeast ferments.")], &l, &Uniform);
        assert_eq!(a.class, EvidenceClass::Unsupported);
    }

    #[test]
    fn stemming_and_weighting() {
        for (a, b) in [
            ("applying", "apply"),
            ("applied", "apply"),
            ("burns", "burn"),
            ("burning", "burn"),
            ("supplies", "supply"),
            ("treatment", "treat"),
            ("bleeding", "bleed"),
        ] {
            assert_eq!(stem(a), stem(b), "{a} vs {b}");
        }
        assert_eq!(stem("gas"), "gas");
        assert_ne!(stem("burn"), stem("bur"));
        let h = hit("p", "s", "Applying a tourniquet above the wound stops arterial bleeding.");
        assert!(is_relevant("How do I apply a tourniquet correctly?", &h, &Uniform));

        struct Rare;
        impl TermWeights for Rare {
            fn weight(&self, w: &str) -> f32 {
                if matches!(w, "python" | "comprehension") {
                    8.0
                } else {
                    1.0
                }
            }
        }
        let h = hit("p", "s", "Write down a list of supplies and keep it with your plan.");
        assert!(
            !is_relevant("How do I write a list comprehension in Python?", &h, &Rare),
            "filler overlap is not relevance"
        );
    }

    #[test]
    fn authoritative_sources_come_first() {
        let l = lookup(vec![
            src("wiki", Authority::Reference, &[], &[], None),
            src("cdc", Authority::Government, &[], &[], None),
        ]);
        let mut wiki = hit("w", "wiki", T);
        wiki.score = 50.0;
        let mut cdc = hit("c", "cdc", T);
        cdc.score = 5.0;
        let a = assess(Q, &QuestionContext::default(), &[wiki, cdc], &l, &Uniform);
        assert_eq!(a.citable[0].passage_id, "c", "government guidance outranks a higher-scoring encyclopedia passage");
        assert_eq!(a.class, EvidenceClass::Verified);
    }

    #[test]
    fn asset_key_normalises() {
        assert_eq!(asset_key(" Acme ", "GX 2000"), "acme:gx-2000");
    }
}
