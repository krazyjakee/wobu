//! Image backends: asking something to draw a picture, and knowing in advance
//! what it will refuse to do.
//!
//! [`ImageBackend`] is the boundary every backend sits behind, written to the
//! intersection of what a local ComfyUI and Google's image models both document
//! rather than to either one. The reasoning for each place they differ, and
//! which of the two forced it, is in `backend.rs`. The trait names no HTTP
//! client and no runtime: `reqwest` is reached only from the adapters
//! ([#51](https://github.com/krazyjakee/wobu/issues/51),
//! [#52](https://github.com/krazyjakee/wobu/issues/52)), exactly as `wobu-llm`
//! kept it out of `provider.rs`.
//!
//! See `docs/08-providers.md`.
//!
//! ## Two traits, because a mesh is not a picture
//!
//! [`MeshBackend`] sits beside [`ImageBackend`] rather than inside it.
//! `docs/08-providers.md` lists Text, Image and Mesh as three separate
//! capabilities, and the second one here is Tencent Hunyuan3D
//! ([#64](https://github.com/krazyjakee/wobu/issues/64)): a request with no
//! aspect ratio, no resolution, no seed and no negative prompt, whose result is a
//! set of files that reference each other by name rather than one blob with
//! dimensions. `mesh.rs` argues it field by field. What the two share is the part
//! that is about *calling a provider* rather than about a medium — [`Error`],
//! [`Cancel`] and [`ProgressSink`] — because two error types would mean two
//! copies of the UI's code table and two ways for `wobu-jobs` to read a failure.
//!
//! ## The half of this crate that is not the trait
//!
//! A backend that quietly does less than it was asked is worse than one that
//! refuses, because the user cannot tell the difference between art that came
//! out wrong and art that came out of a request missing half its input. So
//! [`Capabilities`] is a declaration of what a backend takes, and [`negotiate`]
//! is a total function from *what the influence stack wants* and *what the
//! backend offers* to a request plus an account of everything that had to give.
//! "Silently drop it" is not one of the answers, anywhere.
//!
//! Three consequences the user can actually see, which are the point of doing
//! this at all:
//!
//! **A backend with no structure adapter shows structure references as visibly
//! downgraded to mood-board-only.** The same stack, negotiated against two
//! backends, produces two different requests and says so.
//!
//! ```
//! use wobu_core::{AssetRef, AssetRole, Node, NodeKind, default_preset, new_id};
//! use wobu_imagine::{
//!     AspectRatio, Capabilities, ReferenceMechanisms, Resolution, negotiate,
//! };
//! use wobu_influence::{ImageBudget, Sliders, World, fragments, image_budget, resolve};
//!
//! let mut kael = Node::new(NodeKind::Character, "Kael Vantris")?;
//! kael.asset_links.push(AssetRef::new(new_id(), AssetRole::Pose));
//! kael.asset_links.push(AssetRef::new(new_id(), AssetRole::Costume));
//!
//! let world = World::new([&kael]);
//! let stack = resolve(&world, kael.id, None).unwrap();
//! let extracted = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());
//!
//! let comfy = Capabilities {
//!     max_resolution: Resolution::new(2048, 2048),
//!     aspect_ratios: vec![],
//!     image_refs: ImageBudget::unlimited(),
//!     reference_mechanisms: ReferenceMechanisms::unlimited(),
//!     loras: true,
//!     negative_prompt: true,
//!     requires_billing: false,
//!     streaming_preview: true,
//! };
//! let gemini = Capabilities {
//!     max_resolution: Resolution::new(4096, 4096),
//!     aspect_ratios: AspectRatio::ALL.to_vec(),
//!     image_refs: image_budget("gemini-3-pro-image").unwrap(),
//!     reference_mechanisms: ReferenceMechanisms::image_prompt(),
//!     loras: false,
//!     negative_prompt: false,
//!     requires_billing: true,
//!     streaming_preview: false,
//! };
//!
//! let aspect = AspectRatio::parse(default_preset(NodeKind::Character).aspect).unwrap();
//!
//! // Locally, both references are sent and there is nothing to report.
//! let local = negotiate(&extracted, aspect, &comfy);
//! assert!(local.is_exact());
//! assert_eq!(local.images().kept().count(), 2);
//!
//! // Remotely, the pose reference cannot be structure, so it is withheld and
//! // the card it came from says why — rather than the picture simply not
//! // arriving.
//! let remote = negotiate(&extracted, aspect, &gemini);
//! let told: Vec<_> =
//!     remote.downgrades().iter().map(|d| (d.fragment.section(), d.reason.label())).collect();
//! assert_eq!(told, [(
//!     "pose",
//!     "mood-board only — this backend cannot use it as structure",
//! )]);
//! assert_eq!(remote.images().kept().count(), 1);
//! # Ok::<(), wobu_core::Error>(())
//! ```
//!
//! **Aspect ratios the backend does not support do not appear in the dropdown**,
//! and a preset that asks for one gets the nearest shape rather than a dead
//! Generate button — reported, because an environment matte that silently comes
//! back square is a wrong picture nothing on screen explains.
//!
//! ```
//! use wobu_imagine::{
//!     AspectRatio, Capabilities, ReferenceMechanisms, Resolution, negotiate,
//! };
//! use wobu_influence::ImageBudget;
//!
//! let backend = Capabilities {
//!     max_resolution: Resolution::new(4096, 4096),
//!     aspect_ratios: vec![
//!         AspectRatio::parse("1:1").unwrap(),
//!         AspectRatio::parse("16:9").unwrap(),
//!     ],
//!     image_refs: ImageBudget::unlimited(),
//!     reference_mechanisms: ReferenceMechanisms::image_prompt(),
//!     loras: false,
//!     negative_prompt: false,
//!     requires_billing: true,
//!     streaming_preview: false,
//! };
//!
//! let offered: Vec<String> = AspectRatio::ALL
//!     .iter()
//!     .filter(|a| backend.supports_aspect(**a))
//!     .map(|a| a.to_string())
//!     .collect();
//! assert_eq!(offered, ["1:1", "16:9"]);
//!
//! let matte = negotiate(&[], AspectRatio::parse("21:9").unwrap(), &backend);
//! assert_eq!(matte.aspect().to_string(), "16:9");
//! assert_eq!(matte.requested_aspect().map(|a| a.to_string()), Some("21:9".to_string()));
//! assert_eq!(matte.resolution(), Resolution::new(4096, 2304));
//! ```
//!
//! **Provider reference caps drive the counting budget**, so the Inspector can say
//! `3/3 style refs`. The caps themselves are not restated here: they are
//! `wobu_influence::ImageBudget`, read out of the registry #44 put behind
//! `image_budget`, and [`Capabilities::image_refs`] carries that value
//! unmodified. That is the one field where this crate's shape differs from the
//! sketch in [#50](https://github.com/krazyjakee/wobu/issues/50), which had a
//! map keyed by our own `AssetRole` — keying it that way would push the
//! role-to-bucket judgement into every adapter and let two of them disagree
//! about which pool a `pose` reference competes in. Routing is the separate
//! [`ReferenceMechanisms`] axis: an adapter can expose one structure input and
//! no image-prompt input without changing any provider bucket.

mod aspect;
mod backend;
mod capability;
pub mod comfy;
mod dimensions;
mod error;
pub mod gemini;
mod mesh;
mod negotiate;
pub mod tencent;

pub use aspect::{AspectRatio, Resolution};
pub use backend::{
    Discard, GeneratedImage, ImageBackend, ImageOutcome, ImageRequest, ImageUsage, ProgressSink,
    Reference, Watermark,
};
pub use capability::{Capabilities, ReferenceMechanism, ReferenceMechanisms};
pub use comfy::ComfyBackend;
pub use error::{Error, Result};
pub use gemini::GeminiBackend;
pub use mesh::{
    DEFAULT_FACE_COUNT, FACE_COUNT, GenerateType, GeneratedMesh, MeshBackend, MeshCapabilities,
    MeshFile, MeshFormat, MeshInput, MeshOutcome, MeshRequest, MeshUsage, MeshView, View,
};
pub use negotiate::{Downgrade, Downgraded, Negotiated, negotiate};
pub use tencent::HunyuanBackend;

/// The cancellation token, re-exported from `wobu-llm` rather than defined
/// again here.
///
/// The job queue hands `wobu_llm::Cancel` to every task it runs, and `wobu-jobs`
/// re-exports the same type for the same reason its own note gives: a further
/// copy "would buy nothing and cost every task a bridge: a spawned mirror whose
/// only job is to set one token when another is set, one per job, with its own
/// way of leaking". A generate task is exactly that task, so this is either the
/// same token or it is a bridge, and the bridge is the thing that leaks.
///
/// It is the only thing this crate takes from `wobu-llm`, and that edge cannot
/// become a cycle: a text provider asks a model for words and has no reason to
/// know this crate exists. The edge that *would* be a cycle is the one to
/// `wobu-jobs` — which will want to read this crate's [`Error`] the way it
/// already reads `wobu-llm`'s — and it is why [`ProgressSink`] names none of the
/// queue's types.
pub use wobu_llm::Cancel;
