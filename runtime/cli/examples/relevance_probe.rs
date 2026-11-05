use kb_retrieval::Library;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut lib = Library::new();
    lib.mount("m", std::path::Path::new(&a[1])).unwrap();
    for h in lib.search(&a[2], 15).unwrap() {
        let r = kb_evidence::relevance(&a[2], &h, &lib);
        println!(
            "{:6.2} rel={:.2} {} | {} | {}",
            h.score,
            r,
            h.source_id,
            &h.title.chars().take(40).collect::<String>(),
            h.text.chars().take(70).collect::<String>().replace('\n', " ")
        );
    }
    for (w, st) in kb_evidence::content_terms(&a[2]) {
        println!("term {w} stem {st} idf {:.2}", lib.idf(&w));
    }
}
