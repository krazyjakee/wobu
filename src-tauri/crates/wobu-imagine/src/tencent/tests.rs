use super::*;

use std::time::Instant;

use crate::backend::Discard;
use crate::mesh::{MeshView, Turnaround, View};

fn png(view: View) -> MeshView {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&1024u32.to_be_bytes());
    bytes.extend_from_slice(&1024u32.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
    MeshView::new(view, bytes, "image/png")
}

use crate::testing::block_on;
fn backend() -> HunyuanBackend {
    HunyuanBackend::new(
        Credentials::new("AKIDzzzzzzzzzzzz", SecretKey::new("Gu5t9xGARNpq86cd98joQYCN3Cozk1qA")),
        Region::ApSingapore,
    )
    .unwrap()
}

/// One poll answer, in the documented wire shape.
fn poll(status: &str) -> Vec<u8> {
    format!(r#"{{"Response":{{"Status":"{status}","RequestId":"7f"}}}}"#).into_bytes()
}

fn done() -> Vec<u8> {
    br#"{"Response":{"Status":"DONE","ErrorCode":"","ErrorMessage":"",
        "ResultFile3Ds":[{"Type":"OBJ","Url":"https://cos.example/a.zip?sign=x",
        "PreviewImageUrl":"https://cos.example/a.png?sign=x"}],"RequestId":"7f"}}"#
        .to_vec()
}

/// Drive [`watch`] over a recorded sequence, with no network and no waiting.
fn watched(answers: Vec<Vec<u8>>, cancel: &Cancel) -> Result<Vec<ResultFile>> {
    let mut answers = answers.into_iter();
    block_on(watch(
        Schedule::immediate(),
        || {
            let next = answers.next();
            async move {
                match next {
                    Some(body) => wire::progress(&body),
                    None => Err(Error::Unavailable { detail: "the recording ran out".into() }),
                }
            }
        },
        &mut Discard,
        cancel,
    ))
}

#[test]
fn a_whole_run_polls_through_wait_and_run_and_finishes_on_done() {
    // The documented vocabulary in the order a job sends it. Everything the
    // user sees during a mesh generation is read off these, and there is no
    // websocket to fall back on if one is misread.
    let files =
        watched(vec![poll("WAIT"), poll("WAIT"), poll("RUN"), done()], &Cancel::new()).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].kind, "OBJ");
    assert!(files[0].url.starts_with("https://cos.example/a.zip"));
}

#[test]
fn the_status_bar_is_told_once_per_phase_and_not_once_per_poll() {
    // `ProgressSink::step` asks for throttling at the source. A thirty-minute
    // job is a hundred and twenty polls across three phases, and the status
    // bar draws the last one — so a hundred and seventeen of those events are
    // a redraw of a line that did not change.
    struct Counting(Vec<String>);
    impl ProgressSink for Counting {
        fn step(&mut self, _done: u32, _total: u32, note: Option<&str>) {
            self.0.push(note.unwrap_or_default().to_owned());
        }
    }

    let answers = vec![poll("WAIT"), poll("WAIT"), poll("WAIT"), poll("RUN"), poll("RUN"), done()];
    let mut sink = Counting(Vec::new());
    let mut answers = answers.into_iter();
    block_on(watch(
        Schedule::immediate(),
        || {
            let next = answers.next().unwrap();
            async move { wire::progress(&next) }
        },
        &mut sink,
        &Cancel::new(),
    ))
    .unwrap();
    assert_eq!(
        sink.0,
        ["queued at Tencent Hunyuan3D", "generating the mesh", "downloading the mesh",]
    );
}

#[test]
fn a_status_the_provider_has_never_sent_before_keeps_the_job_alive() {
    // A renamed or added status is a release of theirs, not a failure of
    // ours. Treating it as terminal abandons a job that is generating and has
    // been paid for; treating it as success returns no mesh. It keeps
    // polling, and the deadline is what bounds that.
    let files = watched(vec![poll("QUEUING"), poll("PROCESSING"), done()], &Cancel::new()).unwrap();
    assert_eq!(files.len(), 1);
}

#[test]
fn a_job_that_never_finishes_gives_up_and_says_it_is_still_being_charged_for() {
    // The user is owed the truth here: we have stopped waiting and Tencent has
    // not stopped generating, so the money is spent either way. A bare
    // "timed out" reads as though nothing happened.
    let mut answers = std::iter::repeat_with(|| poll("RUN"));
    let schedule =
        Schedule { first: Duration::ZERO, max: Duration::ZERO, deadline: Duration::ZERO };
    let error = block_on(watch(
        schedule,
        || {
            let next = answers.next().unwrap();
            async move { wire::progress(&next) }
        },
        &mut Discard,
        &Cancel::new(),
    ))
    .unwrap_err();
    assert!(error.to_string().contains("charged either way"), "{error}");
    assert!(error.to_string().contains("generating the mesh"), "{error}");
}

#[test]
fn the_poll_gives_up_long_before_the_job_id_expires() {
    // The regression: somebody raises the deadline to be generous and it
    // passes 24 hours, at which point the last poll of a long run asks about a
    // job id that has expired and is told the job never existed — reported as
    // a vanished job rather than as our own deadline.
    assert!(
        Schedule::default().deadline < JOB_ID_LIFETIME,
        "a poll deadline past the job id's own lifetime cannot report a real answer",
    );
    assert!(Schedule::default().deadline <= JOB_ID_LIFETIME / 4, "and with room to spare");
}

#[test]
fn the_poll_interval_backs_off_and_stays_well_under_the_published_rate_limit() {
    // Twenty requests a second is the published limit and three concurrent
    // jobs is the other. Even at the shortest interval, three jobs polling
    // together is one request a second between them.
    let schedule = Schedule::default();
    assert_eq!(schedule.delay(0), Duration::from_secs(3));
    assert_eq!(schedule.delay(1), Duration::from_secs(6));
    assert_eq!(schedule.delay(9), schedule.max, "and it stops doubling");
    assert!(schedule.delay(0) >= Duration::from_secs(1));
}

#[test]
fn a_failed_job_reports_the_code_the_query_carried_and_not_a_generic_failure() {
    // A `FAIL` arrives on a successful query minutes after the submit was
    // charged, and its `ErrorCode` is the only description of what went wrong.
    let fail = br#"{"Response":{"Status":"FAIL","ErrorCode":"FailedOperation.ImageIllegal",
        "ErrorMessage":"blocked by moderation","RequestId":"7f"}}"#;
    let error = watched(vec![poll("RUN"), fail.to_vec()], &Cancel::new()).unwrap_err();
    assert!(matches!(error, Error::Refused { .. }), "{error}");
    assert!(error.to_string().contains("blocked by moderation"), "{error}");
}

#[test]
fn a_done_job_with_an_empty_file_list_is_told_apart_from_a_failed_one() {
    // A silent empty result and a stated failure send the user to two
    // different places — one is a bug report and the other is a prompt to
    // edit. Both have been billed.
    let empty = br#"{"Response":{"Status":"DONE","ResultFile3Ds":[],"RequestId":"7f"}}"#;
    assert!(matches!(watched(vec![empty.to_vec()], &Cancel::new()), Err(Error::NoMesh)));
}

#[test]
fn a_cancellation_is_noticed_during_a_wait_rather_than_at_the_end_of_one() {
    // The failure this guards: a fifteen-second poll interval and a user who
    // pressed Stop, watching a spinner for the rest of it. The token is raced
    // against the sleep, so a real interval is interrupted rather than served
    // out — which is what the timing assertion below is checking.
    let cancel = Cancel::new();
    cancel.cancel();
    let started = Instant::now();
    let outcome = block_on(watch(
        Schedule {
            first: Duration::from_secs(30),
            max: Duration::from_secs(30),
            deadline: Duration::from_secs(60),
        },
        || async { panic!("a cancelled watch must not poll the provider") },
        &mut Discard,
        &cancel,
    ));
    assert!(matches!(outcome, Err(Error::Cancelled)));
    assert!(started.elapsed() < Duration::from_secs(5), "it waited out the interval");
}

#[test]
fn a_cancelled_job_never_submits_one_and_is_never_billed() {
    // The queue can cancel between admitting a job and starting it. Here the
    // cost is money: a submit that has gone out is charged whatever happens
    // afterwards, and there is no cancel action at the provider.
    let cancel = Cancel::new();
    cancel.cancel();
    let request = MeshRequest::from_prompt(DEFAULT_MODEL, "a wrought iron lantern");
    let outcome = block_on(backend().generate(&request, &mut Discard, &cancel));
    assert!(matches!(outcome.result, Err(Error::Cancelled)));
    assert_eq!(outcome.usage, MeshUsage::free());
}

#[test]
fn a_request_we_would_not_send_costs_nothing_and_never_opens_a_connection() {
    // The order matters: the body is built and checked before anything is
    // signed, so every `Unsupported` is unbilled by construction. A backend
    // that submitted first would charge for the round trip that tells it the
    // face count was out of range.
    let request = MeshRequest::from_prompt(DEFAULT_MODEL, "p").with_face_count(10);
    let outcome = block_on(backend().generate(&request, &mut Discard, &Cancel::new()));
    assert_eq!(outcome.usage, MeshUsage::free());
    assert_eq!(outcome.result.unwrap_err().code(), "internal");
}

#[test]
fn a_poll_is_signed_for_the_region_the_submit_used_and_not_the_backends_current_one() {
    // The correctness bug the issue calls out by name. A job submitted to
    // Singapore and polled in Frankfurt is not an error — it is
    // `FailedOperation.JobNotFound`, which is the same answer as a job that
    // never existed, on a call that has been billed.
    //
    // Driven through `poll_call`, which is the function `query` actually
    // uses. It takes the ticket and no `&self`, so a backend pointed
    // somewhere else — which is what this asserts — has no way to influence
    // the answer.
    let backend = backend();
    assert_eq!(backend.region(), Region::ApSingapore);

    let ticket = JobTicket::new("1338-abc", Endpoint::new(Region::EuFrankfurt));
    let body = wire::query_body(ticket.job_id());
    let call = poll_call(&ticket, &body);
    assert_eq!(call.region, "eu-frankfurt", "the ticket's region, not the backend's");
    assert_eq!(call.action, endpoint::QUERY);

    // And the region reaches the wire as a header, which is the only place it
    // appears: `wire::query_body` names only the job. It is sent *unsigned*,
    // as a common parameter, so a mismatch would still produce a perfectly
    // well-formed request — which is why this is checked here rather than
    // being left to a signature failure to catch.
    let signed = sign(
        &call,
        &Credentials::new("AKIDz", SecretKey::new("Gu5t9xGARNpq86cd98joQYCN3Cozk1qA")),
        1551113065,
    );
    let region: Vec<&str> = signed
        .headers()
        .filter(|(name, _)| *name == "X-TC-Region")
        .map(|(_, value)| value)
        .collect();
    assert_eq!(region, ["eu-frankfurt"]);
    assert!(!body.contains("frankfurt"), "the body carries no region at all");
}

#[test]
fn every_call_carries_the_content_type_that_was_signed() {
    // `sign.rs`'s warning, applied: an HTTP client that picks its own
    // `Content-Type` sends a request different from the one that was signed,
    // and the answer is an `AuthFailure.SignatureFailure` that reads as a bad
    // key. Which is why the body is a string handed to `body()` rather than a
    // value handed to `json()`.
    let signed = sign(
        &Endpoint::new(Region::EuFrankfurt).call(endpoint::SUBMIT, "{}"),
        &Credentials::new("AKIDz", SecretKey::new("k")),
        1551113065,
    );
    let sent: Vec<(&str, &str)> = signed.headers().collect();
    assert!(sent.contains(&("Content-Type", CONTENT_TYPE)));
    assert!(sent.iter().any(|(name, value)| *name == "X-TC-Version" && *value == "2023-09-01"));
    assert!(
        sent.iter()
            .any(|(name, value)| *name == "X-TC-Action" && *value == "SubmitHunyuanTo3DProJob")
    );
}

#[test]
fn capabilities_differ_per_model_in_both_directions() {
    // 3.1 takes twice the views and loses two generate modes. One answer for
    // the backend would have to be the worse of the two, which throws away the
    // multi-view input that is the entire reason to prefer 3.1.
    let backend = backend();
    let pro = backend.capabilities(MODEL_3_1);
    assert_eq!(pro.max_views, 8);
    assert!(!pro.supports(GenerateType::Sketch), "unavailable on 3.1");
    assert!(!pro.supports(GenerateType::LowPoly));

    let older = backend.capabilities(MODEL_3_0);
    assert_eq!(older.max_views, 4);
    assert!(older.supports(GenerateType::Sketch), "and 3.0 still has it");
}

#[test]
fn a_model_we_have_never_heard_of_gets_the_intersection_and_not_the_union() {
    // A project may name a model that has been retired, or one released next
    // month. Offering eight views to something that takes four is a paid call
    // refused after several megabytes have been uploaded, so the conservative
    // answer is the one both known models can honour.
    let caps = backend().capabilities("4.0-preview");
    assert_eq!(caps.max_views, 4, "the smaller of the two");
    assert!(caps.supports(GenerateType::Normal));
    assert!(!caps.supports(GenerateType::Sketch), "which 3.1 does not have");
    assert!(caps.requires_billing, "and this one is known without asking");
}

#[test]
fn the_obj_result_is_unzipped_into_a_mesh_and_the_files_it_references() {
    // The trap `docs/08-providers.md` names: the OBJ `Url` is a `.zip` of the
    // mesh, its `.mtl` and its textures. Written straight to `model.obj` those
    // bytes are a file every viewer refuses, and nobody finds out until after
    // the 24-hour URL has expired.
    let (format, mesh, extras) = assemble(archive::tests::obj_archive()).unwrap();
    assert_eq!(format, MeshFormat::Obj);
    assert_eq!(mesh.name, "model.obj", "and it keeps its name, because the .mtl names it");
    let names: Vec<&str> = extras.iter().map(|file| file.name.as_str()).collect();
    assert_eq!(names, ["model.mtl", "texture_0.png"]);
    assert!(!format.is_self_contained(), "so none of those may be dropped");
}

#[test]
fn a_bare_glb_is_taken_as_it_is_rather_than_looked_for_inside_an_archive() {
    // The other documented shape, and the one the international docs
    // contradict. Deciding from the magic rather than from the declared
    // `Type` is what makes both work without trusting either.
    let bytes = [&archive::GLB_MAGIC[..], &2u32.to_le_bytes()[..], &[0u8; 8][..]].concat();
    let (format, mesh, extras) = assemble(bytes.clone()).unwrap();
    assert_eq!(format, MeshFormat::Glb);
    assert_eq!(mesh.bytes, bytes);
    assert!(extras.is_empty(), "a glb carries its own textures");
    assert!(format.is_self_contained());
}

#[test]
fn the_mesh_inside_an_archive_is_picked_by_format_and_not_by_size() {
    // A texture map is routinely larger than the mesh that uses it, so
    // "the biggest file" is a heuristic that picks the albedo map. And an
    // archive with a `.glb` in it prefers that over the `.obj` beside it,
    // because a self-contained container cannot lose its materials.
    let bytes = archive::tests::zip(
        &[
            ("readme.txt", b"generated by hunyuan3d" as &[u8]),
            ("texture_0.png", &[0u8; 512]),
            ("model.obj", b"v 0 0 0\n"),
            ("model.glb", b"glTF small"),
        ],
        false,
    );
    let (format, mesh, extras) = assemble(bytes).unwrap();
    assert_eq!(format, MeshFormat::Glb);
    assert_eq!(mesh.name, "model.glb");
    assert_eq!(extras.len(), 3, "and nothing is thrown away");
}

#[test]
fn a_download_that_is_neither_shape_says_what_arrived_instead() {
    // Almost always an XML error document from object storage — an expired
    // signature, a bucket policy — and its first line is the whole diagnosis.
    // Reported as "not a mesh" with nothing else, it is a dead end.
    let error = assemble(b"<?xml version=\"1.0\"?><Error><Code>AccessDenied".to_vec()).unwrap_err();
    assert!(error.to_string().contains("AccessDenied"), "{error}");
    assert!(matches!(error, Error::NotAMesh { .. }));

    // And an archive with nothing recognisable in it names what was in it,
    // rather than claiming the download failed.
    let junk = archive::tests::zip(&[("readme.txt", b"nothing here" as &[u8])], false);
    let error = assemble(junk).unwrap_err();
    assert!(error.to_string().contains("readme.txt"), "{error}");
}

#[test]
fn a_preview_url_becomes_a_filename_without_its_signature() {
    // These are signed object-storage URLs and the signature is most of their
    // length. A filename built from the whole URL is unusable on every
    // filesystem there is.
    assert_eq!(preview_name("https://cos.example/jobs/a3f9.png?q-signature=deadbeef"), "a3f9.png");
    assert_eq!(preview_name("https://cos.example/jobs/a3f9"), "preview.png");
    assert_eq!(preview_name(""), "preview.png");
}

#[test]
fn a_backend_works_through_a_box_dyn_and_needs_no_network_to_build() {
    // `project.json` names the backend, so the generate path holds a
    // `Box<dyn MeshBackend>` — and the Inspector draws a backend dropdown on a
    // machine that has never had a Tencent key, so constructing one must not
    // touch the network.
    let boxed: Box<dyn MeshBackend> = Box::new(backend());
    assert_eq!(boxed.id(), "hunyuan3d");
    assert_eq!(boxed.label(), "Tencent Hunyuan3D");
    assert_eq!(boxed.default_model(), "3.1");
    assert!(boxed.capabilities(DEFAULT_MODEL).requires_billing);
}

#[test]
fn debug_output_never_prints_either_half_of_an_account_wide_credential() {
    // A Tencent `SecretKey` is a master credential rather than a scoped token,
    // which `docs/08-providers.md` calls materially more dangerous to hold
    // than an OpenAI-style key. A derived `Debug` on the struct that owns one
    // is the realistic way it reaches a log file.
    let printed = format!("{:?}", backend());
    assert!(!printed.contains("Gu5t9x"), "{printed}");
    assert!(!printed.contains("AKIDzzzz"), "{printed}");
    assert!(printed.contains("ApSingapore"), "the region is not a secret: {printed}");
}

#[test]
fn the_multi_view_path_is_the_one_the_default_model_takes() {
    // The lucky finding `docs/08-providers.md` describes: 3.1's headline
    // feature is multi-view input and the Turnaround preset is a multi-view
    // generator. If the default model ever stopped accepting eight views this
    // is the pairing that would break, silently, into a worse mesh.
    let turnaround = Turnaround::new(View::ALL.into_iter().map(png).collect()).unwrap();
    let request = MeshRequest::from_turnaround(DEFAULT_MODEL, turnaround);
    let caps = backend().capabilities(&request.model);
    assert_eq!(caps.max_views, 8);
    assert!(wire::submit_body(&request, &caps).is_ok());
}

#[test]
fn a_zero_length_wait_finishes_without_a_thread_and_a_real_one_wakes_up() {
    // The timer belongs to no runtime, which is what keeps `Cargo.toml`'s
    // claim that this crate names none true of the crate rather than only of
    // its documentation. Both halves are worth pinning: a zero that spawned a
    // thread would make the poll tests spawn hundreds, and a real sleep that
    // lost its wakeup is a generation that hangs until the process ends.
    block_on(sleep(Duration::ZERO));
    let started = Instant::now();
    block_on(sleep(Duration::from_millis(30)));
    assert!(started.elapsed() >= Duration::from_millis(25), "it did not actually wait");
}
