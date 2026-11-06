//! What an emergency question returns (no model involved).
//! cargo run -p kb-cli --example emergency_probe -- <package dir> "<question>"
struct NoModel;
impl kb_agent::ChatModel for NoModel {
    fn chat(&self, _s: &str, _u: &str) -> Result<String, kb_agent::AgentError> {
        Err(kb_agent::AgentError::Model("not an emergency question: the model would be used".into()))
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = std::path::Path::new(&a[1]);
    let m: kb_packages::Manifest = serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let mut lib = kb_retrieval::Library::new();
    lib.mount(&m.id, dir).unwrap();
    let sources: std::collections::HashMap<(String, String), kb_packages::SourceRecord> =
        m.sources.iter().map(|s| ((m.id.clone(), s.id.clone()), s.clone())).collect();
    match kb_agent::ask(&lib, &sources, &NoModel, &a[2], &Default::default(), &Default::default()) {
        Ok(ans) => {
            println!("class={:?} notes={:?}", ans.class, ans.notes.iter().take(2).collect::<Vec<_>>());
            println!("{}", ans.text.unwrap_or_default().chars().take(1500).collect::<String>());
        }
        Err(e) => println!("{e}"),
    }
}
