//! Show what the model would receive for a question after procedure expansion.
//! cargo run -p kb-cli --example procedure_probe -- <package dir> "<question>"
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut lib = kb_retrieval::Library::new();
    lib.mount("m", std::path::Path::new(&a[1])).unwrap();
    let terms: Vec<String> = kb_evidence::content_terms(&a[2]).into_iter().map(|(w, _)| w).collect();
    let mut hits = lib.search(&terms.join(" "), 8).unwrap();
    hits.truncate(3);
    kb_agent::expand_procedures(&lib, &mut hits);
    for h in hits {
        println!("--- {} | {} | max_step={}", h.source_id, h.title, kb_agent::max_step(&h.text));
        println!("{}", h.text.chars().take(1400).collect::<String>());
    }
}
