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
//! Fragment extraction and weighting (#42), the text and image budgets (#43) and
//! the `influence_resolve` / `prompt_compile` commands (#46) build on
//! [`ResolvedSource`], which carries everything a `SnapshotLayer` needs except
//! its fragments.

mod resolve;
mod stack;
mod world;

pub use resolve::resolve;
pub use stack::{Origin, Reached, ResolvedSource, ResolvedStack, Shot};
pub use world::World;
