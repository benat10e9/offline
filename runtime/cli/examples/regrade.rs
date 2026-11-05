//! Re-check stored model answers with the current citation verifier (no model needed).
//! cargo run -p kb-cli --example regrade -- eval-report.json
fn main() {
    let path = std::env::args().nth(1).expect("eval report path");
    let r: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let (mut answered, mut fully_cited) = (0, 0);
    let marker = regex::Regex::new(r"\[S(\d+)\]").unwrap();
    for x in r["results"].as_array().unwrap() {
        let Some(text) = x["answer"].as_str() else { continue };
        answered += 1;
        let max = marker.captures_iter(text).filter_map(|c| c[1].parse::<usize>().ok()).max().unwrap_or(0);
        let shown: Vec<kb_retrieval::Hit> = (0..max.max(1))
            .map(|i| kb_retrieval::Hit {
                package_id: "p".into(),
                passage_id: i.to_string(),
                doc_id: String::new(),
                source_id: String::new(),
                title: String::new(),
                heading: String::new(),
                text: String::new(),
                score: 0.0,
            })
            .collect();
        let v = kb_agent::verify_citations(text, &shown);
        if v.uncited_sentences.is_empty() {
            fully_cited += 1;
        } else {
            println!(
                "{:<12} {} uncited: {:?}",
                x["id"].as_str().unwrap(),
                v.uncited_sentences.len(),
                v.uncited_sentences.first().map(|s| s.chars().take(90).collect::<String>())
            );
        }
    }
    println!("\n{fully_cited}/{answered} answers fully covered by citations under the current rule");
}
