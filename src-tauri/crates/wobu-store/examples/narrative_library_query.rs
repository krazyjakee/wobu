//! Measure bounded discovery against an existing canonical scale fixture.
use std::{error::Error, path::PathBuf, time::Instant};
use wobu_store::{Project, project::narrative_library::LibraryQuery};

fn main() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("Pass an existing .wobu project")?);
    let start = Instant::now();
    let project = Project::open(&root)?;
    let open_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut results = Vec::new();
    for query in [
        LibraryQuery::default(),
        LibraryQuery { query: "ASHFALL-PHRASE-0731".into(), ..Default::default() },
        LibraryQuery { query: "harbor".into(), policy: "edited".into(), ..Default::default() },
    ] {
        let mut times = Vec::new();
        let mut bytes = 0;
        let mut rows = 0;
        for _ in 0..20 {
            let start = Instant::now();
            let page = project.library_query(&query)?;
            bytes = serde_json::to_vec(&page)?.len();
            rows = page.rows.len();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        results.push(
            serde_json::json!({"query": query, "ms": times, "response_bytes": bytes, "rows": rows}),
        );
    }
    let selected = "00000000000000000000000732".parse()?;
    let mut get_ms = Vec::new();
    for _ in 0..20 {
        let start = Instant::now();
        project.load_scene(selected)?;
        get_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"project":root,"open_ms":open_ms,"queries":results,"selected_get_ms":get_ms,"scope":"Actual filesystem/store, debug build, warm filesystem cache; serialized response bytes exclude IPC framing. No webview timing claim."})
        )?
    );
    Ok(())
}
