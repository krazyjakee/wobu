//! Profile existing read phases against an existing synthetic fixture; no authoring writes.
use std::{error::Error, path::PathBuf, time::Instant};
use wobu_store::{Project, project::narrative_library::LibraryQuery};

fn timed<T>(phase: &str, run: impl FnOnce() -> wobu_store::Result<T>) -> wobu_store::Result<T> {
    let start = Instant::now();
    let result = run();
    println!(
        "{}",
        serde_json::json!({"phase":phase,"ms":start.elapsed().as_secs_f64()*1000.0,"ok":result.is_ok()})
    );
    result
}
fn main() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(
        std::env::args_os().nth(1).ok_or("Pass an existing synthetic .wobu fixture")?,
    );
    let selected =
        std::env::args().nth(2).unwrap_or_else(|| "00000000000000000000000732".into()).parse()?;
    let mut project = timed("project_open", || Project::open(&root))?;
    let before = project.narrative_fingerprint()?;
    for iteration in 0..3 {
        println!(
            "{}",
            serde_json::json!({"iteration":iteration,"scope":"filesystem/store, not native UI; index writes only"})
        );
        let file = timed("selected_scene", || project.load_scene(selected))?;
        let plan = timed("reconcile_plan", || project.reconcile_plan())?;
        let observed = timed("reconcile_observe", || plan.observe())?;
        assert!(timed("reconcile_revalidate", || observed.revalidate())?);
        timed("reconcile_apply", || project.apply_reconcile(observed))?;
        timed("source_fingerprint", || project.narrative_fingerprint())?;
        let catalog = timed("scene_catalog", || project.scene_catalog())?;
        let snapshot = timed("review_capture", || project.review_snapshot(selected, None))?;
        let mut proposals = timed("review_proposals", || project.review_proposals())?;
        let view = timed("review_view_contexts", || {
            snapshot.view_with_proposals(proposals.remove(&selected).unwrap_or_default())
        })?;
        timed("review_evidence", || snapshot.evidence())?;
        timed("review_final_verify", || snapshot.verify_current(&project))?;
        let start = Instant::now();
        let query = project.library_query(&LibraryQuery {
            query: "ASHFALL-PHRASE-0731".into(),
            ..Default::default()
        })?;
        println!(
            "{}",
            serde_json::json!({"phase":"library_query","ms":start.elapsed().as_secs_f64()*1000.0,"rows":query.rows.len(),"json_bytes":serde_json::to_vec(&query)?.len()})
        );
        println!(
            "{}",
            serde_json::json!({"scenes":catalog.scenes.len(),"review_lines":view.lines.len(),"selected_bytes":serde_json::to_vec(&file.scene)?.len()})
        );
    }
    assert_eq!(before, project.narrative_fingerprint()?);
    Ok(())
}
