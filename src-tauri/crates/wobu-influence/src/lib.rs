//! Influence stack resolution.
//!
//! Given a subject node, the engine walks outward through the world and collects
//! the sources that will describe it, in the fixed order from
//! `docs/04-influence-engine.md`:
//!
//! ```text
//! 1 Style · 2 World · 3 Ancestry · 4 Culture · 5 Place · 6 Subject · 7 Shot
//! ```
//!
//! That order is the product, not an implementation detail. The compiler joins
//! sources in it so that general context lands first and the subject last, where
//! the recency bias of a text encoder does the most good; the Inspector reads
//! top to bottom in the same order; and every fragment keeps its layer through
//! to the screen, which is what lets the compiled prompt be tinted by origin.
//! Getting the order wrong produces art that is subtly off and an attribution
//! trail that lies about why.
//!
//! The crate is pure: a borrowed view of already-loaded nodes in, a resolved
//! stack out, no IO of any kind. `prompt_compile` runs on every Inspector
//! interaction and must stay sub-millisecond (`docs/05-architecture.md`), which
//! is why the input is [`World`] and not a project handle — see there.
//!
//! ```
//! use wobu_core::{Layer, Node, NodeKind};
//! use wobu_influence::{Shot, World, resolve};
//!
//! let style = Node::new(NodeKind::StyleGuide, "Ashfall House Style")?;
//! let kael = Node::new(NodeKind::Character, "Kael Vantris")?;
//!
//! let world = World::new([&style, &kael]);
//! let stack = resolve(&world, kael.id, Some(Shot::new("Character sheet"))).unwrap();
//!
//! let layers: Vec<_> = stack.sources().iter().map(|s| (s.layer, s.name())).collect();
//! assert_eq!(layers, [
//!     (Layer::Style, "Ashfall House Style"),
//!     (Layer::Subject, "Kael Vantris"),
//!     (Layer::Shot, "Character sheet"),
//! ]);
//! # Ok::<(), wobu_core::Error>(())
//! ```
//!
//! A source does not contribute a paragraph, it contributes [`fragments`]: one
//! per description section, one per item of a list section, one per reference
//! image, each weighted by `link.weight × section_priority × user_slider` and
//! routed to a prompt, a negative prompt or an image adapter.
//!
//! ```
//! use wobu_core::{
//!     Description, FragmentTarget, Layer, Node, NodeKind, SectionValue, default_preset,
//! };
//! use wobu_influence::{Sliders, World, fragments, resolve};
//!
//! let mut kael = Node::new(NodeKind::Character, "Kael Vantris")?;
//! kael.description = Some(Description::from_sections([
//!     ("silhouette".to_string(), SectionValue::Text("Tall, narrow, hooded".into())),
//!     ("never".to_string(), SectionValue::List(vec!["modern firearms".into()])),
//! ]));
//!
//! let world = World::new([&kael]);
//! let stack = resolve(&world, kael.id, None).unwrap();
//! let compiled = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());
//!
//! let rows: Vec<_> = compiled
//!     .iter()
//!     .map(|f| (f.layer(), f.section(), f.text().unwrap(), f.weight(), f.target()))
//!     .collect();
//! assert_eq!(rows, [
//!     // A character sheet is read as a shape, so it weights `silhouette` above 1.0.
//!     (Layer::Subject, "silhouette", "Tall, narrow, hooded", 1.4, FragmentTarget::Prompt),
//!     (Layer::Subject, "never", "modern firearms", 1.0, FragmentTarget::Negative),
//! ]);
//! # Ok::<(), wobu_core::Error>(())
//! ```
//!
//! [`compile`] fits that slice into a [`Budget`] and emits the two prompts. It
//! drops the lightest fragments first and hands back an account of every one of
//! them, because "the Inspector reports what was dropped rather than truncating
//! silently" (`docs/04-influence-engine.md`) — a user who cannot see what was cut
//! cannot learn to write better upstream notes, which is the feedback loop the
//! whole engine exists for.
//!
//! ```
//! use wobu_core::{Description, Node, NodeKind, SectionValue, default_preset};
//! use wobu_influence::{
//!     Budget, Chars, DropReason, Sliders, World, compile, fragments, resolve,
//! };
//!
//! let mut kael = Node::new(NodeKind::Character, "Kael Vantris")?;
//! kael.description = Some(Description::from_sections([
//!     ("silhouette".to_string(), SectionValue::Text("Tall, narrow, hooded".into())),
//!     ("costume".to_string(), SectionValue::Text("Ash-grey longcoat".into())),
//!     ("never".to_string(), SectionValue::List(vec!["modern firearms".into()])),
//! ]));
//!
//! let world = World::new([&kael]);
//! let stack = resolve(&world, kael.id, None).unwrap();
//! let extracted = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());
//!
//! // Room for one of the two prompt fragments. A character sheet is read as a
//! // shape, so `silhouette` outweighs `costume` and the longcoat is what goes.
//! let budget = Budget { prompt: Chars::new(24), negative: Chars::UNLIMITED };
//! let compiled = compile(&extracted, budget);
//!
//! assert_eq!(compiled.prompt(), "Tall, narrow, hooded");
//! assert_eq!(compiled.negative(), "modern firearms");
//! let cut: Vec<_> =
//!     compiled.dropped().iter().map(|d| (d.fragment.section(), d.reason)).collect();
//! assert_eq!(cut, [("costume", DropReason::Budget)]);
//! # Ok::<(), wobu_core::Error>(())
//! ```
//!
//! The per-role image budget (#44) takes the same slice and prices what this one
//! deliberately does not; the `influence_resolve` / `prompt_compile` commands
//! (#46) group it back into `SnapshotLayer`s, which is why a [`Fragment`] keeps
//! its own layer and node rather than being handed out already grouped.

mod budget;
mod compile;
mod extract;
mod fragment;
mod resolve;
mod stack;
mod world;

pub use budget::{Budget, Chars, CompiledPrompt, DropReason, Dropped};
pub use compile::compile;
pub use extract::fragments;
pub use fragment::{Fragment, FragmentBody, Sliders, section_target};
pub use resolve::resolve;
pub use stack::{Origin, Reached, ResolvedSource, ResolvedStack, Shot};
pub use world::World;
