//! The mesh-backend trait: a third capability, not a fourth image backend.
//!
//! ## Why this is not [`ImageBackend`](crate::ImageBackend)
//!
//! `docs/08-providers.md` lists Text, Image and Mesh as three capabilities, and
//! `backend.rs` opens by saying a trait written against a single vendor is that
//! vendor's request struct wearing a trait. Bending [`ImageBackend`] around
//! Hunyuan3D would be the same mistake pointed the other way: not one vendor
//! deforming a trait, but one *medium* deforming a trait written for another.
//!
//! Field by field, against `ImageRequest` and `GeneratedImage`:
//!
//! - **`aspect` and `resolution` are meaningless.** A mesh has no shape on a
//!   page. They are not optional on `ImageRequest` either — `ImageRequest::new`
//!   takes a `Negotiated` precisely so the two cannot be set separately — so a
//!   mesh request would have to invent a ratio and a pixel size, and every
//!   surface that draws an aspect dropdown off `Capabilities::aspect_ratios`
//!   would draw one for the 3D backend.
//! - **`seed` does not exist.** `SubmitHunyuanTo3DProJob` takes no seed, so
//!   `Generation.seed` for a mesh would record a number nothing used.
//! - **`negative` does not exist**, and on 3.1 neither does the positive prompt
//!   when images are supplied — `docs/08-providers.md` is explicit that 3.1 has
//!   no text+image conditioning path at all, so text-to-mesh and image-to-mesh
//!   are mutually exclusive. That is [`MeshInput`], and it is an enum for that
//!   reason.
//! - **References are not a budget, they are a slate of named views.**
//!   `Reference` carries a `RefBucket` and a weight because an image backend
//!   competes pictures for a limited number of slots. Hunyuan3D takes exactly one
//!   image per [`View`] and refuses duplicates; there is nothing to meter and
//!   nothing to evict.
//! - **The result is a set of files, not one blob with dimensions.**
//!   `GeneratedImage` carries `width`, `height` and a `Watermark`; a mesh arrives
//!   as a `.zip` of a mesh plus a `.mtl` plus texture maps, and the useful facts
//!   about it are which file is the mesh and what the rest are called.
//! - **Cost is counted in jobs, not images.** `ImageUsage::billed_images` is
//!   what a spend ceiling reads for a backend that charges per picture. Hunyuan3D
//!   charges per job, and the international `Query` response omits the
//!   `ResultCreditConsumed` field the mainland one returns, so there is no
//!   per-image figure to report and no credit figure to read back either.
//!
//! `Capabilities` is the clearest case of all: every one of its eight fields —
//! `max_resolution`, `aspect_ratios`, `image_refs`, `controlnet`, `loras`,
//! `negative_prompt`, `streaming_preview` — is a question about a picture, and
//! [`negotiate`](crate::negotiate) is a total function over it. A mesh backend
//! implementing `ImageBackend` would answer seven of them with a value chosen to
//! be ignored, and `negotiate` would reshape a request that has no shape.
//!
//! ## What *is* shared, and why that is the right line
//!
//! [`Error`], [`Cancel`] and [`ProgressSink`] are the crate's, not the image
//! trait's. They describe how a *provider call* fails, how it is stopped and how
//! it reports itself, and none of those three names a picture — `ProgressSink`
//! takes primitives precisely so it commits to no vendor's shape. Duplicating
//! them for meshes would mean the generate path had two error types, `wobu-jobs`
//! had two tables to map onto `Billed`, and `error.rs`'s hand-copy of the UI's
//! code table had a second copy to drift from.
//!
//! ## How this composes with the job queue
//!
//! Exactly as `backend.rs` describes, and one point harder. `docs/08-providers.md`
//! records a limit of **three concurrent Pro jobs**, and that cap is
//! [`wobu_jobs::Queue`]'s — `queue.rs` already defaults `concurrency` to 3 and
//! names Hunyuan3D as the reason. Nothing here counts in-flight jobs, holds a
//! semaphore or sleeps to stay under a rate: a second admission controller inside
//! an adapter would be invisible to the queue's own accounting, would not survive
//! `Queue::set_concurrency`, and would be counting a limit that is per *account*
//! while the adapter is per *backend instance*. What this file owns is one job at
//! a time, honestly reported and stoppable.
//!
//! [`ImageBackend`]: crate::ImageBackend
//! [`wobu_jobs::Queue`]: https://docs.rs/wobu-jobs

use std::fmt;
use std::ops::RangeInclusive;

use async_trait::async_trait;
use wobu_llm::Cancel;

use crate::backend::ProgressSink;
use crate::error::{Error, Result};

/// One of the eight camera positions Hunyuan3D 3.1 reconstructs from.
///
/// An enum rather than a string, and spelled exactly as the provider spells it,
/// because `wobu_core::preset`'s Turnaround preset emits these same eight names
/// so that "the mesh adapter can pass them straight through with no intermediate
/// mapping". Two spellings of `left_front` in two crates is a view silently
/// dropped by the provider, which comes back as a worse mesh and no error.
///
/// The order is the order the preset lists them in, which is also the order they
/// are sent — front first, because a single-image request is the front view and
/// nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum View {
    Front,
    Left,
    Right,
    Back,
    Top,
    Bottom,
    LeftFront,
    RightFront,
}

impl View {
    /// All eight, in the order the Turnaround preset emits them.
    pub const ALL: [View; 8] = [
        View::Front,
        View::Left,
        View::Right,
        View::Back,
        View::Top,
        View::Bottom,
        View::LeftFront,
        View::RightFront,
    ];

    /// The wire spelling, which is also the preset's.
    pub fn as_str(self) -> &'static str {
        match self {
            View::Front => "front",
            View::Left => "left",
            View::Right => "right",
            View::Back => "back",
            View::Top => "top",
            View::Bottom => "bottom",
            View::LeftFront => "left_front",
            View::RightFront => "right_front",
        }
    }

    /// Read a preset's view name. `None` for anything else, because a preset
    /// that grows a ninth view must not silently send it as a front image.
    pub fn parse(name: &str) -> Option<View> {
        View::ALL.into_iter().find(|view| view.as_str() == name)
    }
}

impl fmt::Display for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One rendered view, already resolved to bytes.
///
/// Bytes rather than a path for the reason `Reference` gives: this crate does no
/// IO, and the provider takes base64 in the request body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshView {
    pub view: View,
    pub bytes: Vec<u8>,
    /// `image/png` or `image/jpeg`. Carried rather than sniffed because
    /// multi-view input accepts **only** those two, and the sensible place to say
    /// so is where the picture is picked rather than at the point of failure.
    pub mime: String,
}

impl MeshView {
    pub fn new(view: View, bytes: Vec<u8>, mime: impl Into<String>) -> MeshView {
        MeshView { view, bytes, mime: mime.into() }
    }
}

/// What the mesh is reconstructed from — and never both.
///
/// An enum rather than two optional fields, because `docs/08-providers.md` is
/// explicit that on 3.1 "text-to-3D and image-to-3D are mutually exclusive": the
/// `Sketch` generate mode was the only one that took a prompt and an image
/// together, and it is unavailable on 3.1. Two fields would make an
/// unsatisfiable request expressible, and the provider's answer to one is an
/// `InvalidParameterValue` after the request has been signed and sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeshInput {
    /// Text-to-mesh. No views, no references, and no influence stack behind it —
    /// by the time a turnaround exists the influence has been baked into the
    /// pictures.
    Prompt(String),
    /// Image-to-mesh. One entry is single-image; more is multi-view, which is
    /// 3.1's headline feature and the one our Turnaround preset exists to feed.
    Views(Vec<MeshView>),
}

/// The reconstruction mode.
///
/// All four the provider documents, and not only the two we can send: which are
/// available depends on the model, and [`MeshCapabilities::generate_types`] is
/// where that is decided. An enum with two variants would put the model-dependent
/// half of the answer in the type system, where it cannot vary by model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerateType {
    /// Geometry and texture. The default, and what a concept mesh wants.
    Normal,
    /// Untextured geometry.
    Geometry,
    /// 3.0 only.
    LowPoly,
    /// 3.0 only, and the only mode that ever accepted a prompt and an image
    /// together.
    Sketch,
}

impl GenerateType {
    pub fn as_str(self) -> &'static str {
        match self {
            GenerateType::Normal => "Normal",
            GenerateType::Geometry => "Geometry",
            GenerateType::LowPoly => "LowPoly",
            GenerateType::Sketch => "Sketch",
        }
    }
}

impl fmt::Display for GenerateType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The provider's default triangle budget.
pub const DEFAULT_FACE_COUNT: u32 = 500_000;

/// The range the provider accepts, from `docs/08-providers.md`.
///
/// A constant rather than a number in a validation branch, because the UI slider
/// and the check that refuses an out-of-range request have to be the same two
/// numbers — a slider that offers 2000 faces is a paid call that fails.
pub const FACE_COUNT: RangeInclusive<u32> = 3_000..=1_500_000;

/// One mesh generation.
///
/// No `Negotiated` and no aspect, per the module note. `model` is opaque to this
/// crate exactly as `ImageRequest::model` is — for Hunyuan3D it is the string
/// `"3.1"`, which is a *parameter* on the submit action and not an endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshRequest {
    pub model: String,
    pub input: MeshInput,
    /// Triangles. Within [`FACE_COUNT`] or the adapter refuses the request
    /// rather than clamping it — the trait's "send exactly what the request says,
    /// or fail" applies here for the same reason it does to images.
    pub face_count: u32,
    /// Physically-based material output. Default false at the provider, and kept
    /// false here so that a request built and never told otherwise costs what the
    /// provider's default costs.
    pub enable_pbr: bool,
    pub generate_type: GenerateType,
}

impl MeshRequest {
    /// Image-to-mesh, which is the path the Turnaround preset feeds.
    pub fn from_views(model: impl Into<String>, views: Vec<MeshView>) -> MeshRequest {
        MeshRequest::new(model, MeshInput::Views(views))
    }

    /// Text-to-mesh. Mutually exclusive with views by construction.
    pub fn from_prompt(model: impl Into<String>, prompt: impl Into<String>) -> MeshRequest {
        MeshRequest::new(model, MeshInput::Prompt(prompt.into()))
    }

    fn new(model: impl Into<String>, input: MeshInput) -> MeshRequest {
        MeshRequest {
            model: model.into(),
            input,
            face_count: DEFAULT_FACE_COUNT,
            enable_pbr: false,
            generate_type: GenerateType::Normal,
        }
    }

    pub fn with_face_count(mut self, faces: u32) -> MeshRequest {
        self.face_count = faces;
        self
    }

    pub fn with_pbr(mut self, enabled: bool) -> MeshRequest {
        self.enable_pbr = enabled;
        self
    }

    pub fn with_generate_type(mut self, generate_type: GenerateType) -> MeshRequest {
        self.generate_type = generate_type;
        self
    }

    /// The views this request carries, empty for a text-to-mesh one.
    pub fn views(&self) -> &[MeshView] {
        match &self.input {
            MeshInput::Views(views) => views,
            MeshInput::Prompt(_) => &[],
        }
    }
}

/// What a mesh backend can do with a given model.
///
/// The mesh analogue of [`Capabilities`](crate::Capabilities), and short for the
/// same reason that one is long: these are the questions whose wrong answer
/// costs a paid call. `max_views` and `generate_types` in particular are where
/// "`LowPoly` and `Sketch` are unavailable on 3.1" lives, and where "3.0 accepts
/// front + 3 views, 3.1 accepts front + 7" lives.
///
/// Total over unknown models, exactly as `ImageBackend::capabilities` is: a
/// project may name a model that has been retired, and the answer must be the
/// most conservative registered one rather than the most permissive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshCapabilities {
    /// Including the front view. One means single-image only.
    pub max_views: usize,
    pub face_count: RangeInclusive<u32>,
    pub pbr: bool,
    /// In preference order, most useful first. The UI draws a picker from this
    /// and the adapter refuses anything not in it.
    pub generate_types: Vec<GenerateType>,
    /// Whether this model reconstructs from a prompt with no image at all.
    pub text_to_mesh: bool,
    pub requires_billing: bool,
}

impl MeshCapabilities {
    pub fn supports(&self, generate_type: GenerateType) -> bool {
        self.generate_types.contains(&generate_type)
    }
}

/// One file out of a generation, in memory.
///
/// `name` is the name inside the archive — `model.obj`, `model.mtl`,
/// `texture_0.png` — because an OBJ references its `.mtl` by name and the `.mtl`
/// references its textures by name, so renaming any of them on the way to disk
/// produces a mesh that loads with no materials and no error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

impl MeshFile {
    pub fn new(name: impl Into<String>, bytes: Vec<u8>) -> MeshFile {
        MeshFile { name: name.into(), bytes }
    }

    /// Lowercased, without the dot, empty for a name with no extension.
    pub fn extension(&self) -> String {
        self.name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default()
    }
}

/// What kind of mesh came back.
///
/// **Open, with an `Other` arm**, because `docs/08-providers.md` says the
/// international docs "list `Type` values that contradict GLB being returned" and
/// asks for `Type` to be switched on defensively rather than treated as an enum.
/// A closed enum here would mean a format we have never seen is a failure rather
/// than a file, and the file is very likely fine — the store writes bytes and a
/// viewer opens them.
///
/// Derived from the *filename* rather than from the provider's `Type` string, for
/// the same reason `GeneratedImage` reads its dimensions off the image: the
/// declared type and the delivered bytes are two different claims and only one of
/// them is a fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeshFormat {
    Glb,
    Gltf,
    Obj,
    Fbx,
    /// The extension, lowercased and without the dot.
    Other(String),
}

impl MeshFormat {
    /// What the file is, judged by its name.
    pub fn from_filename(name: &str) -> MeshFormat {
        let ext =
            name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "glb" => MeshFormat::Glb,
            "gltf" => MeshFormat::Gltf,
            "obj" => MeshFormat::Obj,
            "fbx" => MeshFormat::Fbx,
            other => MeshFormat::Other(other.to_owned()),
        }
    }

    /// The extension a store should write it under.
    ///
    /// `wobu_core::asset::mesh_path` currently hardcodes `.glb`; a mesh that is
    /// not one has to be written under its own extension or it is a file whose
    /// name lies about its contents.
    pub fn extension(&self) -> &str {
        match self {
            MeshFormat::Glb => "glb",
            MeshFormat::Gltf => "gltf",
            MeshFormat::Obj => "obj",
            MeshFormat::Fbx => "fbx",
            MeshFormat::Other(ext) => ext,
        }
    }

    /// Whether this file is the whole mesh or only part of one.
    ///
    /// A `.glb` carries its geometry, materials and textures in one container; an
    /// `.obj` is three or more files that reference each other by name. The
    /// difference decides whether [`GeneratedMesh::extras`] may be dropped, and
    /// dropping them silently is a grey mesh.
    pub fn is_self_contained(&self) -> bool {
        matches!(self, MeshFormat::Glb | MeshFormat::Fbx)
    }
}

impl fmt::Display for MeshFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

/// One finished mesh, as bytes, with everything it references.
///
/// **No URL, anywhere, deliberately.** `docs/08-providers.md`: result URLs are
/// valid for 24 hours, so a stored one is a link that works all through testing
/// and is dead by morning. There is no field here to put one in, which is the
/// only version of that rule that cannot be forgotten — the adapter downloads on
/// `DONE` and hands over bytes, and where they land is the store's decision
/// exactly as it is for `GeneratedImage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedMesh {
    pub format: MeshFormat,
    /// The mesh itself.
    pub mesh: MeshFile,
    /// The `.mtl` and the texture maps, for a format that keeps them outside the
    /// mesh. Empty for a `.glb`.
    pub extras: Vec<MeshFile>,
    /// The provider's own render of the result, when it sent one.
    ///
    /// Downloaded for the same reason the mesh is: `PreviewImageUrl` expires on
    /// the same 24-hour clock, so it is bytes or it is nothing.
    pub preview: Option<MeshFile>,
}

impl GeneratedMesh {
    /// Every file that has to reach disk together, mesh first.
    ///
    /// The preview is not in it: it is a picture *of* the asset rather than part
    /// of it, and it belongs wherever thumbnails go.
    pub fn files(&self) -> impl Iterator<Item = &MeshFile> {
        std::iter::once(&self.mesh).chain(self.extras.iter())
    }

    pub fn total_bytes(&self) -> usize {
        self.files().map(|file| file.bytes.len()).sum()
    }
}

/// What a mesh generation is known to have cost.
///
/// **Jobs, not images and not credits.** The provider bills per submitted job,
/// and `docs/08-providers.md` records that the international `Query` response
/// omits the `ResultCreditConsumed` field the mainland one returns — "we cannot
/// read spend back from the API". So this counts the one thing we can state as a
/// fact, and a cost estimate in money is a local model of published prices that
/// belongs with the spend ceiling rather than in an adapter that would have to
/// guess.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MeshUsage {
    /// Jobs the provider accepted and will charge for.
    ///
    /// Counted from the moment a `JobId` comes back, not from `DONE`: a job
    /// cancelled while it was generating has still been paid for, and a ceiling
    /// that only counted finished jobs would undercount exactly the runs the user
    /// abandoned because they were slow.
    pub billed_jobs: u32,
}

impl MeshUsage {
    pub fn free() -> MeshUsage {
        MeshUsage::default()
    }

    pub fn billed(jobs: u32) -> MeshUsage {
        MeshUsage { billed_jobs: jobs }
    }

    pub fn is_billed(self) -> bool {
        self.billed_jobs > 0
    }
}

/// The result of one mesh generation: what it cost, and what came back.
///
/// Not a `Result`, for `ImageOutcome`'s reason and one more of its own: the
/// window between "the provider accepted the job" and "we have the files" is
/// minutes long here rather than seconds, so the great majority of the ways this
/// can fail happen *after* the money was spent. A `?` that carried the error out
/// and left the usage behind would undercount nearly every failure this adapter
/// has.
#[derive(Debug)]
pub struct MeshOutcome {
    pub usage: MeshUsage,
    pub result: Result<GeneratedMesh>,
}

impl MeshOutcome {
    pub fn new(usage: MeshUsage, result: Result<GeneratedMesh>) -> MeshOutcome {
        MeshOutcome { usage, result }
    }

    /// A failure before the provider could have charged anything: no credential,
    /// a refused connection, a cancellation that beat the submit out of the door,
    /// a request we would not send.
    pub fn unbilled(error: Error) -> MeshOutcome {
        MeshOutcome { usage: MeshUsage::free(), result: Err(error) }
    }

    pub fn is_ok(&self) -> bool {
        self.result.is_ok()
    }
}

/// A mesh backend, selected per project and held behind a `dyn`.
///
/// `#[async_trait]` for `ImageBackend`'s reason: the backend is chosen at runtime
/// from `project.json`, so this is stored as `Box<dyn MeshBackend>` and a native
/// async trait method gives no `Send` future to box. Nothing here names a runtime
/// or an HTTP client.
#[async_trait]
pub trait MeshBackend: Send + Sync {
    /// Stable id: the `backend` in `project.json`, the `backend` field of every
    /// `Generation`, and the keychain entry.
    fn id(&self) -> &'static str;

    fn label(&self) -> &'static str;

    fn default_model(&self) -> &'static str;

    /// Total over unknown model ids, per [`MeshCapabilities`].
    fn capabilities(&self, model: &str) -> MeshCapabilities;

    /// Make one mesh.
    ///
    /// Contract for implementors:
    ///
    /// 1. **Send exactly what the request says, or fail.** A face count outside
    ///    [`MeshCapabilities::face_count`], or a [`GenerateType`] the model does
    ///    not have, is [`Error::Unsupported`] — never a quiet substitution, which
    ///    would bill for a mesh the user did not ask for.
    /// 2. **Honour `cancel` by stopping.** Racing the next wait against
    ///    `Cancel::cancelled` rather than polling a flag between requests: a poll
    ///    interval is seconds long and a mesh takes minutes, so a loop that
    ///    checked the flag only between requests would leave a stopped job running
    ///    for the rest of its interval.
    /// 3. **Report cost honestly**, from the moment a job id exists.
    /// 4. **Download before returning.** Result URLs expire in 24 hours, so a
    ///    [`GeneratedMesh`] must be bytes. There is no field for a URL, which is
    ///    what makes this checkable rather than remembered.
    async fn generate(
        &self,
        request: &MeshRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> MeshOutcome;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_names_are_the_ones_the_turnaround_preset_emits() {
        // `wobu_core::preset` spells these so "the mesh adapter can pass them
        // straight through with no intermediate mapping". A second spelling here
        // is a view the provider silently ignores: the reconstruction comes back
        // worse and nothing reports an error.
        let turnaround = wobu_core::preset("turnaround").expect("the Turnaround preset exists");
        let ours: Vec<&str> = View::ALL.iter().map(|view| view.as_str()).collect();
        assert_eq!(ours, turnaround.views, "same names, same order");
        assert_eq!(ours.len(), 8);
    }

    #[test]
    fn a_view_name_from_outside_the_eight_is_refused_rather_than_treated_as_front() {
        // The preset could grow a ninth view. Defaulting an unknown name to
        // anything would send two images for one position, which the provider
        // rejects as a duplicate after the request has been signed and sent.
        assert_eq!(View::parse("left_front"), Some(View::LeftFront));
        assert_eq!(View::parse("three_quarter"), None);
        assert_eq!(View::parse("Front"), None, "the wire spelling is lowercase");
    }

    #[test]
    fn text_to_mesh_and_image_to_mesh_cannot_both_be_asked_for() {
        // `docs/08-providers.md`: `Sketch` was the only mode taking a prompt and
        // an image together and it is unavailable on 3.1, so "3.1 has no
        // text+image conditioning path at all". Two optional fields would make
        // that request expressible and the provider's answer is an
        // `InvalidParameterValue` on a signed, sent call.
        let views =
            MeshRequest::from_views("3.1", vec![MeshView::new(View::Front, vec![1], "image/png")]);
        assert_eq!(views.views().len(), 1);

        let prompt = MeshRequest::from_prompt("3.1", "a wrought iron lantern");
        assert!(prompt.views().is_empty(), "and there is no field to put a view in");
        assert_eq!(prompt.input, MeshInput::Prompt("a wrought iron lantern".into()));
    }

    #[test]
    fn a_request_built_and_left_alone_costs_what_the_providers_defaults_cost() {
        // Every one of these is a provider default. A builder that started from
        // `EnablePBR: true` or a face count of its own choosing would charge more
        // than the user asked for, on a request nobody edited.
        let request = MeshRequest::from_prompt("3.1", "p");
        assert_eq!(request.face_count, DEFAULT_FACE_COUNT);
        assert_eq!(request.face_count, 500_000);
        assert!(!request.enable_pbr);
        assert_eq!(request.generate_type, GenerateType::Normal);
    }

    #[test]
    fn the_face_count_range_is_one_pair_of_numbers_and_not_two() {
        // The slider the UI draws and the check that refuses the request have to
        // be the same bounds. A slider offering 2000 faces is a paid call that
        // fails, and one capped at 100000 is a capability the user paid for and
        // cannot reach.
        assert_eq!(*FACE_COUNT.start(), 3_000);
        assert_eq!(*FACE_COUNT.end(), 1_500_000);
        assert!(FACE_COUNT.contains(&DEFAULT_FACE_COUNT));
        assert!(!FACE_COUNT.contains(&2_999));
        assert!(!FACE_COUNT.contains(&1_500_001));
    }

    #[test]
    fn a_mesh_format_we_have_never_seen_is_a_file_rather_than_a_failure() {
        // `docs/08-providers.md` asks for `Type` to be treated as an open string
        // because the international docs contradict GLB being returned. A closed
        // enum would turn a perfectly good `.ply` into a failed generation after
        // the job was paid for.
        assert_eq!(MeshFormat::from_filename("model.glb"), MeshFormat::Glb);
        assert_eq!(MeshFormat::from_filename("MODEL.OBJ"), MeshFormat::Obj);
        assert_eq!(MeshFormat::from_filename("mesh.ply"), MeshFormat::Other("ply".into()));
        assert_eq!(MeshFormat::from_filename("mesh.ply").extension(), "ply");
        assert_eq!(MeshFormat::from_filename("noextension"), MeshFormat::Other(String::new()));
    }

    #[test]
    fn an_obj_is_not_self_contained_and_a_glb_is() {
        // The `.mtl` and the textures are separate files an `.obj` names by
        // string. Dropping them writes a mesh that loads grey, with nothing on
        // screen to say a file is missing.
        assert!(!MeshFormat::Obj.is_self_contained());
        assert!(MeshFormat::Glb.is_self_contained());
        assert!(!MeshFormat::Other("ply".into()).is_self_contained(), "unknown means keep it all");
    }

    #[test]
    fn a_generated_mesh_carries_bytes_and_has_nowhere_to_put_a_url() {
        // The 24-hour result-URL expiry, enforced structurally. A field for a URL
        // is a field somebody serialises into `project.json`, and the link works
        // for the whole of an afternoon's testing.
        let mesh = GeneratedMesh {
            format: MeshFormat::Obj,
            mesh: MeshFile::new("model.obj", b"v 0 0 0\n".to_vec()),
            extras: vec![
                MeshFile::new("model.mtl", b"newmtl m\n".to_vec()),
                MeshFile::new("texture_0.png", vec![0x89, b'P', b'N', b'G']),
            ],
            preview: Some(MeshFile::new("preview.png", vec![0x89])),
        };
        let names: Vec<&str> = mesh.files().map(|file| file.name.as_str()).collect();
        assert_eq!(names, ["model.obj", "model.mtl", "texture_0.png"], "mesh first");
        assert_eq!(mesh.total_bytes(), 8 + 9 + 4);

        // The preview is a picture of the asset, not part of it, so it is not in
        // the set that has to land together.
        assert!(!names.contains(&"preview.png"));

        let printed = format!("{mesh:?}");
        assert!(!printed.contains("http"), "there is no URL in here to print: {printed}");
    }

    #[test]
    fn a_file_reports_its_own_extension_case_insensitively() {
        // Archive entries come back in whatever case the provider wrote them, and
        // a `.PNG` treated as an unknown extension is a texture written under the
        // wrong name and an `.mtl` that cannot find it.
        assert_eq!(MeshFile::new("Texture_0.PNG", vec![]).extension(), "png");
        assert_eq!(MeshFile::new("LICENSE", vec![]).extension(), "");
    }

    #[test]
    fn usage_counts_jobs_because_there_is_no_credit_figure_to_read() {
        // `docs/08-providers.md`: the international `Query` response omits
        // `ResultCreditConsumed`, so a per-call cost is a local model of published
        // prices. Counting jobs is the one thing we can state as fact, and the
        // spend meter reads it over the bridge in camelCase like everything else.
        let json = serde_json::to_value(MeshUsage::billed(1)).unwrap();
        assert_eq!(json["billedJobs"], 1);
        assert!(MeshUsage::billed(1).is_billed());
        assert!(!MeshUsage::free().is_billed());
        assert_eq!(
            serde_json::from_value::<MeshUsage>(serde_json::json!({})).unwrap(),
            MeshUsage::free(),
        );
    }

    #[test]
    fn a_failure_after_the_job_was_accepted_still_reports_what_it_cost() {
        // The case this non-`Result` shape exists for, and it is the common case
        // here rather than the rare one: a mesh job is minutes long, so nearly
        // every way it fails happens after the money was spent.
        let outcome = MeshOutcome::new(MeshUsage::billed(1), Err(Error::NoMesh));
        assert!(outcome.usage.is_billed());
        assert!(!outcome.is_ok());

        // And a request we refused to send costs nothing.
        let refused = MeshOutcome::unbilled(Error::Unsupported { detail: "3000 faces".into() });
        assert_eq!(refused.usage, MeshUsage::free());
    }

    #[test]
    fn capabilities_answer_per_model_because_the_two_models_differ_in_both_directions() {
        // 3.1 takes eight views and loses two generate modes; 3.0 takes four and
        // keeps all four modes. One answer for the whole backend would have to be
        // the worst of the two, which throws away 3.1's headline feature.
        let pro = MeshCapabilities {
            max_views: 8,
            face_count: FACE_COUNT,
            pbr: true,
            generate_types: vec![GenerateType::Normal, GenerateType::Geometry],
            text_to_mesh: true,
            requires_billing: true,
        };
        assert!(pro.supports(GenerateType::Normal));
        assert!(!pro.supports(GenerateType::Sketch), "unavailable on 3.1");
        assert!(!pro.supports(GenerateType::LowPoly));
    }
}
