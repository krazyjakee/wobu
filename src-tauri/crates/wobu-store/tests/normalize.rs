//! Temporary: rewrite the fixture through the real writer.
use std::path::Path;
#[test]
#[ignore]
fn normalize_fixture() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/Ashfall.wobu");
    for e in walkdir::WalkDir::new(root.join("nodes")).into_iter().filter_map(Result::ok) {
        if e.path().extension().is_none_or(|x| x != "md") { continue }
        let text = std::fs::read_to_string(e.path()).unwrap();
        let node = wobu_store::markdown::from_markdown(&text, e.path()).unwrap();
        let out = wobu_store::markdown::to_markdown(&node).unwrap();
        if out != text {
            println!("normalised {}", e.path().display());
            std::fs::write(e.path(), out).unwrap();
        }
    }
}
