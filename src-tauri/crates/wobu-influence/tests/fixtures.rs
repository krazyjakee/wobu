//! The engine against worlds that exist as files rather than as Rust.
//!
//! Every other test in this crate builds its world with `Node::new` and pushes
//! links onto it. That proves the walk, the weighting and the two budgets, and it
//! is the right shape for those questions — but it cannot notice that the format
//! those nodes arrive in has drifted, because no file was ever involved. The
//! worlds under `fixtures/` are hand-written Markdown, the same thing a `git
//! pull` or an Obsidian user would hand us, and they are the only place in this
//! crate where a reader can look at a file and see what a world actually is.
//!
//! Two things follow from that, and they are the whole design of this file:
//!
//! - **The fixtures are read with the store's own reader.** `wobu-store` is a
//!   dev-dependency here and can never be anything else — see `Cargo.toml`. A
//!   frontmatter parser written inside this file would be a second reader of the
//!   format, free to drift from the real one and keep passing, which is the
//!   drift these fixtures exist to catch.
//! - **The fixtures are proved to be what the app writes**, by round-tripping
//!   every file through `to_markdown` and demanding byte equality
//!   (`every_fixture_file_is_exactly_what_the_app_would_write`). Without that
//!   they would document a format nothing produces, which is worse than
//!   documenting none.
//!
//! Snapshots are longhand `Vec`s, like the rest of the crate and not the
//! `insta`-style files #3 asked for. The thing a reviewer has to check here is
//! layer order and drop order, and an opaque generated file is exactly where a
//! wrong order goes unnoticed — which is the argument `stacks.rs` opens with, and
//! it did not get weaker by the world moving onto disk.

use std::path::{Path, PathBuf};

use wobu_core::{FragmentTarget, Id, Layer, NodeKind, Preset, default_preset, preset};
use wobu_influence::{
    Budget, Chars, CompiledImages, CompiledPrompt, DropReason, Fragment, RefBucket, ResolvedStack,
    Shot, Sliders, World, compile, compile_images, fragments, image_budget, resolve,
};

use DropReason::{Budget as Cut, Silenced};
use FragmentTarget::{MoodboardOnly, Negative, Prompt, StructureRef, StyleRef};

/* ── the loader ───────────────────────────────────────────────────────────── */

/// A project folder, read into memory without touching SQLite.
///
/// The index is a disposable cache of what is already in the Markdown
/// (`docs/02-data-model.md`), so a fixture never needs one — and must not open
/// one, because `Project::open` probes the folder for writability, stages temp
/// files in it and keys a SQLite database by the project's ULID. All three would
/// dirty a committed fixture, and the last would have every test in this file
/// racing for one database file under the default parallel runner. Reading the
/// Markdown directly is both simpler and the thing being tested.
struct Fixture {
    root: PathBuf,
    /// Owned, because [`World`] borrows the nodes the caller is already holding
    /// — that is what keeps the engine off the filesystem, so somebody has to be
    /// holding them.
    nodes: Vec<wobu_core::Node>,
}

impl Fixture {
    /// Read `fixtures/<name>.wobu`.
    fn open(name: &str) -> Fixture {
        Fixture::at(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name))
    }

    fn at(root: PathBuf) -> Fixture {
        assert!(root.join("project.json").is_file(), "{} is not a project folder", root.display());
        let nodes = markdown_files(&root.join("nodes"))
            .into_iter()
            .map(|path| {
                let text = std::fs::read_to_string(&path).unwrap();
                wobu_store::markdown::from_markdown(&text, &path)
                    .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()))
            })
            .collect();
        Fixture { root, nodes }
    }

    fn world(&self) -> World<'_> {
        World::new(self.nodes.iter())
    }

    /// A node's id, looked up by the name in its frontmatter.
    ///
    /// Tests name subjects rather than pasting ULIDs, because a test that says
    /// `01KG7T2EQ60000000000000000` cannot be read against the fixture without
    /// grepping for it, and the point of a fixture is being readable.
    fn id(&self, name: &str) -> Id {
        self.nodes
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("no node named {name} in {}", self.root.display()))
            .id
    }

    /// Every reference image actually in the folder, as the ids they derive to.
    fn assets_on_disk(&self) -> Vec<Id> {
        let originals = self.root.join("assets/originals");
        if !originals.is_dir() {
            return Vec::new();
        }
        let mut ids: Vec<Id> = files_under(&originals)
            .iter()
            .map(|path| {
                let hash = path.file_stem().unwrap().to_string_lossy();
                wobu_store::assets::asset_id(&hash)
                    .unwrap_or_else(|| panic!("{} is not named after a digest", path.display()))
            })
            .collect();
        ids.sort();
        ids
    }

    /// Every reference image the *frontmatter* claims, deduplicated.
    fn assets_claimed(&self) -> Vec<Id> {
        let mut ids: Vec<Id> =
            self.nodes.iter().flat_map(|n| n.asset_links.iter().map(|a| a.asset_id)).collect();
        ids.sort();
        ids.dedup();
        ids
    }
}

/// Every `.md` under `nodes/`, sorted.
///
/// Sorted for reproducibility rather than for correctness: resolution does not
/// depend on load order and `stacks.rs` pins that, but a loader that yielded
/// whatever `read_dir` felt like would make a failure here hard to reproduce.
fn markdown_files(nodes: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = files_under(nodes)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    paths.sort();
    paths
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// The reference images in `Saltmarch.wobu`, named by hand.
///
/// An asset's id is derived from its BLAKE3 hash rather than minted
/// (`docs/02-data-model.md`), so a fixture cannot pick readable ids for its
/// pictures the way it picks ULIDs for its nodes — and a drop report over five
/// references is five indistinguishable ULIDs without a table like this one.
/// Checked against the folder in both directions by
/// `every_reference_the_frontmatter_claims_is_a_file_in_the_folder`, so it cannot
/// drift from the bytes it names.
const REFERENCES: &[(&str, &str)] = &[
    ("house reed weave", "7VT43TZ6X7YF6S95HHQMVW1BGQ"),
    ("house wet plank", "26WQZW0CKWFT1QSVJA073F56NR"),
    ("warden reed-cloth", "70Y30XSB03PZ9HM82NGREQGBC6"),
    ("warden shell collar", "1DFFGQ4R4V5D9AZ1QTMY1V1MKF"),
    ("wren coat", "3M80JPTHFCCYFH9T2GYREHG6HN"),
    ("wren punting pose", "37HQ43ZCTR59S72E9PMDY68XPY"),
    ("wren moodboard", "0XJ3HE8Q29XAPXMZ9PSF2CK3SP"),
];

fn reference_id(name: &str) -> Id {
    let (_, ulid) = REFERENCES.iter().find(|(n, _)| *n == name).expect("a named reference");
    Id::from_string(ulid).expect("fixture reference ids are valid ULIDs")
}

fn reference_name(id: Id) -> &'static str {
    REFERENCES
        .iter()
        .find(|(_, ulid)| Id::from_string(ulid).unwrap() == id)
        .map(|(name, _)| *name)
        .expect("every reference in the fixture is named")
}

/* ── snapshot shapes, matching the rest of the crate ──────────────────────── */

/// The stack as the Inspector would list it, top to bottom.
fn stack_rows<'a>(stack: &ResolvedStack<'a>) -> Vec<(Layer, &'a str, f32)> {
    stack.sources().iter().map(|s| (s.layer, s.name(), s.weight)).collect()
}

/// One row of a layer card: which card, which section, what it is worth and
/// where it goes.
///
/// The fragment's *body* is deliberately not in here, which is the one place
/// this file's snapshots are narrower than `fragments.rs`'s. Every sentence in
/// these files is pinned character for character by
/// `the_two_prompts_are_the_files_joined_in_layer_order`, and which picture is
/// which by `the_style_bucket_overflows_before_any_other_does`, so carrying the
/// text through here as well would only make the table too wide to read — and a
/// snapshot nobody reads is the failure mode the whole longhand style exists to
/// avoid.
type Row<'a> = (Layer, &'a str, &'static str, f32, FragmentTarget);

fn fragment_rows<'a>(fragments: &[Fragment<'a>]) -> Vec<Row<'a>> {
    fragments
        .iter()
        .map(|f| (f.layer(), f.source_name(), f.section(), f.weight(), f.target()))
        .collect()
}

/// One line of a drop report: which card, which section, and why.
type Casualty<'a> = (Layer, &'a str, &'static str, DropReason);

fn report<'a>(compiled: &CompiledPrompt<'a>) -> Vec<Casualty<'a>> {
    compiled
        .dropped()
        .iter()
        .map(|d| (d.fragment.layer(), d.fragment.source_name(), d.fragment.section(), d.reason))
        .collect()
}

/// One reference in a bucket, named so two of one role on one card can be told
/// apart.
type Ref<'a> = (Layer, &'a str, &'static str);

fn kept<'a>(images: &'a CompiledImages<'a>, bucket: RefBucket) -> Vec<Ref<'a>> {
    images
        .bucket(bucket)
        .map(|b| {
            b.kept()
                .iter()
                .map(|f| (f.layer(), f.source_name(), reference_name(f.asset_id().unwrap())))
                .collect()
        })
        .unwrap_or_default()
}

fn lost<'a>(
    images: &'a CompiledImages<'a>,
    bucket: RefBucket,
) -> Vec<(Layer, &'a str, &'static str, DropReason)> {
    images
        .bucket(bucket)
        .map(|b| {
            b.dropped()
                .iter()
                .map(|d| {
                    let f = d.fragment;
                    (f.layer(), f.source_name(), reference_name(f.asset_id().unwrap()), d.reason)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Everything `Saltmarch.wobu` contributes for Wren under one preset —
/// unsorted, unfiltered, zero-weight and `moodboard_only` entries included,
/// which is the slice both `compile` and `compile_images` are specified to take.
fn saltmarch_fragments<'a>(world: &World<'a>, subject: Id, preset: &Preset) -> Vec<Fragment<'a>> {
    let stack = resolve(world, subject, Some(Shot::new("Character sheet · 3:4"))).unwrap();
    fragments(&stack, preset, &Sliders::neutral())
}

/* ── the format ───────────────────────────────────────────────────────────── */

#[test]
fn every_fixture_file_is_exactly_what_the_app_would_write() {
    // The test that makes every other test in this file mean something. A
    // fixture is only format documentation while the app would produce it byte
    // for byte; the moment it would not, these files document a format that does
    // not exist, and the engine is being proved against a world it will never be
    // handed. It is also the regression a hand-written fixture is uniquely able
    // to catch — opening one of these worlds and saving a single node must not
    // rewrite files nobody touched.
    for name in ["Saltmarch.wobu", "Ouroboros.wobu"] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name);
        let files = markdown_files(&root.join("nodes"));
        assert!(!files.is_empty(), "{name} has no nodes");
        for path in files {
            let original = std::fs::read_to_string(&path).unwrap();
            let node = wobu_store::markdown::from_markdown(&original, &path)
                .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));
            let rewritten = wobu_store::markdown::to_markdown(&node).unwrap();
            assert_eq!(rewritten, original, "{} is not what the app writes", path.display());
        }
    }
}

#[test]
fn every_reference_the_frontmatter_claims_is_a_file_in_the_folder() {
    // Asset ids are derived from the bytes and nothing on disk records one
    // (`docs/02-data-model.md`), so an id in frontmatter that names no file is
    // not a broken pointer the store would report — it is a reference that
    // resolves to nothing, silently, at the moment a request is being built.
    // Both directions, because a picture in the folder that no node claims is a
    // fixture that has quietly stopped exercising the image budget.
    let saltmarch = Fixture::open("Saltmarch.wobu");
    assert_eq!(saltmarch.assets_claimed(), saltmarch.assets_on_disk());

    // And the readable names this file leans on are the same set, so a report
    // full of ULIDs can be read as a report about pictures.
    let mut named: Vec<Id> = REFERENCES.iter().map(|(name, _)| reference_id(name)).collect();
    named.sort();
    assert_eq!(named, saltmarch.assets_on_disk());
}

/* ── the whole stack, off disk ────────────────────────────────────────────── */

#[test]
fn the_seven_layers_read_off_disk_in_the_documented_order() {
    // The same claim `stacks.rs` opens with, made against files instead of Rust:
    // the seven layers of docs/04, in order. What this can catch and the
    // code-built one cannot is the layer arriving from the *frontmatter* — a
    // `role:` spelling drifting, `parent:` ceasing to be read as nesting, or the
    // singletons stopping being found by `kind:`. Every one of those is a
    // silent reorder of the prompt.
    let saltmarch = Fixture::open("Saltmarch.wobu");
    let world = saltmarch.world();
    let subject = saltmarch.id("Wren Alder");
    let stack = resolve(&world, subject, Some(Shot::new("Character sheet · 3:4"))).unwrap();

    assert_eq!(stack_rows(&stack), vec![
        (Layer::Style, "Saltmarch House Style", 1.0),
        (Layer::World, "Saltmarch", 1.0),
        (Layer::Ancestry, "Fenwrought", 1.0),
        (Layer::Culture, "The Reed Wardens", 1.0),
        // Nested in the file by `parent:`, not linked — and the parent is
        // therefore further out, which is what puts the region first.
        (Layer::Place, "The Long Shallows", 1.0),
        (Layer::Place, "Stiltmoor", 1.0),
        (Layer::Subject, "Wren Alder", 1.0),
        (Layer::Shot, "Character sheet · 3:4", 1.0),
    ]);
}

#[test]
fn the_layer_cards_list_what_the_files_actually_say() {
    // Every fragment of a real seven-layer world, in the order the Inspector
    // lists them and the compiler joins them. The regression this guards that no
    // code-built world can: the `### ` headings in these files are the *labels*
    // from the kind registry, and they are matched back to section keys on read.
    // Rename a label in `wobu-core` and every file on every user's disk stops
    // contributing that section — no error, just a thinner prompt.
    let saltmarch = Fixture::open("Saltmarch.wobu");
    let world = saltmarch.world();
    let sheet = default_preset(NodeKind::Character);
    let extracted = saltmarch_fragments(&world, saltmarch.id("Wren Alder"), sheet);

    assert_eq!(fragment_rows(&extracted), vec![
        (Layer::Style, "Saltmarch House Style", "medium", 1.0, Prompt),
        (Layer::Style, "Saltmarch House Style", "lighting", 1.0, Prompt),
        // A list section is one fragment per item, not one per list, so a budget
        // can shed one swatch without taking the palette with it.
        (Layer::Style, "Saltmarch House Style", "palette", 1.0, Prompt),
        (Layer::Style, "Saltmarch House Style", "palette", 1.0, Prompt),
        (Layer::Style, "Saltmarch House Style", "never", 1.0, Negative),
        (Layer::Style, "Saltmarch House Style", "never", 1.0, Negative),
        // References follow the prose of the card they hang off, routed by role.
        (Layer::Style, "Saltmarch House Style", "material", 1.0, StyleRef),
        (Layer::Style, "Saltmarch House Style", "material", 1.0, StyleRef),
        (Layer::World, "Saltmarch", "era", 1.0, Prompt),
        (Layer::World, "Saltmarch", "tone", 1.0, Prompt),
        (Layer::World, "Saltmarch", "never", 1.0, Negative),
        (Layer::World, "Saltmarch", "never", 1.0, Negative),
        // A character sheet is read as a shape, so `silhouette` is 1.4 wherever
        // it appears and `anatomy` 1.2 — at any depth in the stack.
        (Layer::Ancestry, "Fenwrought", "silhouette", 1.4, Prompt),
        (Layer::Ancestry, "Fenwrought", "anatomy", 1.2, Prompt),
        (Layer::Ancestry, "Fenwrought", "never", 1.0, Negative),
        (Layer::Culture, "The Reed Wardens", "costume", 1.3, Prompt),
        (Layer::Culture, "The Reed Wardens", "ornament", 1.0, Prompt),
        (Layer::Culture, "The Reed Wardens", "never", 1.0, Negative),
        // A role whose name is also a section key picks up that section's
        // priority, which is why these two are 1.3 and the house style's
        // `material` references are 1.0 — `materials` is the section key.
        (Layer::Culture, "The Reed Wardens", "costume", 1.3, StyleRef),
        (Layer::Culture, "The Reed Wardens", "costume", 1.3, StyleRef),
        (Layer::Place, "The Long Shallows", "climate", 1.0, Prompt),
        // Flat light is the point of a character sheet, so a location's ambient
        // light is actively unhelpful here however strongly the world argues.
        (Layer::Place, "The Long Shallows", "light", 0.3, Prompt),
        (Layer::Place, "Stiltmoor", "architecture", 1.0, Prompt),
        (Layer::Place, "Stiltmoor", "wear", 1.0, Prompt),
        (Layer::Subject, "Wren Alder", "silhouette", 1.4, Prompt),
        (Layer::Subject, "Wren Alder", "costume", 1.3, Prompt),
        (Layer::Subject, "Wren Alder", "materials", 1.0, Prompt),
        (Layer::Subject, "Wren Alder", "signature", 1.0, Prompt),
        (Layer::Subject, "Wren Alder", "never", 1.0, Negative),
        (Layer::Subject, "Wren Alder", "costume", 1.3, StyleRef),
        (Layer::Subject, "Wren Alder", "pose", 1.0, StructureRef),
        (Layer::Subject, "Wren Alder", "mood", 1.0, MoodboardOnly),
        (Layer::Shot, "Character sheet · 3:4", "framing", 1.0, Prompt),
    ]);
}

#[test]
fn the_two_prompts_are_the_files_joined_in_layer_order() {
    // What actually gets sent, from a world anyone can open in Obsidian and
    // read. General context first, the subject's own sentences after it, the
    // framing last where a text encoder's recency bias does the most good.
    let saltmarch = Fixture::open("Saltmarch.wobu");
    let world = saltmarch.world();
    let sheet = default_preset(NodeKind::Character);
    let extracted = saltmarch_fragments(&world, saltmarch.id("Wren Alder"), sheet);
    let compiled = compile(&extracted, Budget::unlimited());

    assert_eq!(
        compiled.prompt(),
        format!(
            "Egg tempera on gessoed panel, built in thin glazes., \
             Flat sea-light from a white sky, doubled by the water under it., \
             #8b8f7a, #4a554e, \
             Reed age, two generations after the water came inland., \
             Patient and damp. Nobody is winning., \
             Low and wide, the weight carried forward over the toes., \
             Webbed hands, a second eyelid, no external ears., \
             Banded reed-cloth, cut short at the knee for wading., \
             Shell discs sewn in a row along the collar., \
             Standing water to the horizon, and no dry season left., \
             Overcast, and doubled by the water underneath it., \
             Plank walkways on driven piles, roped rather than nailed., \
             Everything below waist height is green and slick., \
             Stooped even for a Fenwrought, \
             Warden reed-cloth cut short at the shin., \
             Waxed cloth gone stiff with salt, \
             A punting pole never out of reach, {}",
            sheet.framing
        )
    );
    // Every layer's `never` list, in the same order and in its own pool. Six of
    // the seven come from layers above the subject, which is what "populate
    // `never`" upstream is worth: the subject's own file contributes one line.
    assert_eq!(
        compiled.negative(),
        "Direct sunlight, Photographic detail, Stone cities, Horses, \
         Upright human proportions, Metal armour, Dry clothing"
    );
    assert_eq!(report(&compiled), vec![]);
    assert_eq!(compiled.overflow(), None);
}

/* ── the cases #3 named ───────────────────────────────────────────────────── */

#[test]
fn a_text_budget_too_small_says_which_sentences_it_cut() {
    // Text budget overflow, over a world with enough written into it to have a
    // real preference. This is the shape of the problem the budget exists for
    // and the one a code-built fixture keeps understating: seven layers of
    // perfectly reasonable prose is already far more than a tight backend takes,
    // and nothing about any single file looks wrong.
    //
    // The lightest go first, so what survives is the preset's own priorities
    // made visible — silhouette at 1.4, costume at 1.3, anatomy at 1.2 — and
    // everything the sheet has no opinion about is gone. Including, worth saying
    // out loud, the shot's own framing text: it is an ordinary 1.0 fragment and
    // it competes like one. The report is what the Inspector puts on each card,
    // and a user who cannot see it cannot learn to write leaner notes upstream,
    // which is the feedback loop the whole engine is for.
    let saltmarch = Fixture::open("Saltmarch.wobu");
    let world = saltmarch.world();
    let sheet = default_preset(NodeKind::Character);
    let extracted = saltmarch_fragments(&world, saltmarch.id("Wren Alder"), sheet);

    let budget = Budget { prompt: Chars::new(260), negative: Chars::UNLIMITED };
    let compiled = compile(&extracted, budget);

    assert_eq!(
        compiled.prompt(),
        "Low and wide, the weight carried forward over the toes., \
         Webbed hands, a second eyelid, no external ears., \
         Banded reed-cloth, cut short at the knee for wading., \
         Stooped even for a Fenwrought, \
         Warden reed-cloth cut short at the shin."
    );
    assert_eq!(report(&compiled), vec![
        (Layer::Style, "Saltmarch House Style", "medium", Cut),
        (Layer::Style, "Saltmarch House Style", "lighting", Cut),
        (Layer::Style, "Saltmarch House Style", "palette", Cut),
        (Layer::Style, "Saltmarch House Style", "palette", Cut),
        (Layer::World, "Saltmarch", "era", Cut),
        (Layer::World, "Saltmarch", "tone", Cut),
        (Layer::Culture, "The Reed Wardens", "ornament", Cut),
        (Layer::Place, "The Long Shallows", "climate", Cut),
        (Layer::Place, "The Long Shallows", "light", Cut),
        (Layer::Place, "Stiltmoor", "architecture", Cut),
        (Layer::Place, "Stiltmoor", "wear", Cut),
        (Layer::Subject, "Wren Alder", "materials", Cut),
        (Layer::Subject, "Wren Alder", "signature", Cut),
        (Layer::Shot, "Character sheet · 3:4", "framing", Cut),
    ]);
    // The report is in reading order, not drop order, so it walks alongside the
    // layer cards — which is why the 0.3 ambient light that went first is listed
    // in the middle.
    assert_eq!(compiled.overflow(), None);
}

#[test]
fn two_presets_over_one_world_keep_different_sentences() {
    // "A material study boosts `materials` and drops `silhouette`; a turnaround
    // does the reverse" (docs/04). `wobu-core` pins that as a property of the
    // registry; this is what it means for a world someone wrote. Same files,
    // same subject, same budget — and the one sentence that survives is a
    // different one, which is the entire promise of the preset dropdown.
    let saltmarch = Fixture::open("Saltmarch.wobu");
    let world = saltmarch.world();
    let subject = saltmarch.id("Wren Alder");
    // Room for exactly one fragment. The floor of one is what stops an empty
    // positive prompt, so this is the cleanest possible reading of "which
    // sentence does this preset think matters most".
    let budget = Budget { prompt: Chars::new(32), negative: Chars::UNLIMITED };

    let study =
        compile(&saltmarch_fragments(&world, subject, preset("material_study").unwrap()), budget);
    assert_eq!(study.prompt(), "Waxed cloth gone stiff with salt");

    let turnaround =
        compile(&saltmarch_fragments(&world, subject, preset("turnaround").unwrap()), budget);
    assert_eq!(turnaround.prompt(), "Stooped even for a Fenwrought");
}

#[test]
fn the_style_bucket_overflows_before_any_other_does() {
    // Per-role image budget overflow, from a folder. Five references land in the
    // style bucket — two from the house style, two from the culture, one from
    // the subject — and `gemini-3-pro-image` takes three. A character sheet
    // leans on costume, so the house style's two materials are the lightest and
    // they are what goes, attributed to the card that lost them.
    let saltmarch = Fixture::open("Saltmarch.wobu");
    let world = saltmarch.world();
    let extracted = saltmarch_fragments(
        &world,
        saltmarch.id("Wren Alder"),
        default_preset(NodeKind::Character),
    );
    let images = compile_images(&extracted, image_budget("gemini-3-pro-image").unwrap());

    assert_eq!(kept(&images, RefBucket::StyleRefs), vec![
        (Layer::Culture, "The Reed Wardens", "warden reed-cloth"),
        (Layer::Culture, "The Reed Wardens", "warden shell collar"),
        (Layer::Subject, "Wren Alder", "wren coat"),
    ]);
    assert_eq!(lost(&images, RefBucket::StyleRefs), vec![
        (Layer::Style, "Saltmarch House Style", "house reed weave", Cut),
        (Layer::Style, "Saltmarch House Style", "house wet plank", Cut),
    ]);
    // The pose is a character reference and never competed for a style slot,
    // and the moodboard reference occupies nothing at all.
    assert_eq!(kept(&images, RefBucket::Characters), vec![(
        Layer::Subject,
        "Wren Alder",
        "wren punting pose"
    )]);
    assert_eq!(lost(&images, RefBucket::Characters), vec![]);
    assert_eq!(kept(&images, RefBucket::Objects), vec![]);
}

#[test]
fn a_ring_of_links_on_disk_contributes_each_court_exactly_once() {
    // Two cultures that each claim the other, written into two files. `stacks.rs`
    // proves the walk terminates; what only a compile can show is the point of
    // first-visit-wins — each court contributes its costume *once*. A ring that
    // resolved to two cards for one node would double that node's weight in the
    // prompt with nothing on screen to explain it.
    let ouroboros = Fixture::open("Ouroboros.wobu");
    let world = ouroboros.world();
    let stack = resolve(&world, ouroboros.id("Moss"), None).unwrap();

    assert_eq!(stack_rows(&stack), vec![
        (Layer::Culture, "The Briar Court", 1.0),
        (Layer::Culture, "The Hollow Court", 1.0),
        (Layer::Place, "The Sunken Mill", 0.0),
        (Layer::Subject, "Moss", 1.0),
    ]);

    let extracted = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());
    let compiled = compile(&extracted, Budget::unlimited());
    assert_eq!(
        compiled.prompt(),
        "Waxed thorn-thread stitched over the shoulders., \
         Bleached linen worn loose over bare feet., \
         Small, and always half turned away."
    );
}

#[test]
fn a_link_weighted_to_zero_on_disk_keeps_its_card_and_silences_its_fragments() {
    // `weight: 0.0` in frontmatter is not `enabled: false`, and the difference is
    // the whole reason the drop report is an enum. A disabled link is not walked
    // at all; a link weighted to nothing still puts its card in the Inspector,
    // still lists what that card *would* have said, and reports every one of
    // those fragments as silenced rather than cut. A user who turned the mill
    // down needs "put the slider back up" as an available answer, and a card
    // that had vanished would read as the notes having been lost.
    let ouroboros = Fixture::open("Ouroboros.wobu");
    let world = ouroboros.world();
    let stack = resolve(&world, ouroboros.id("Moss"), None).unwrap();
    let extracted = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());

    // The card is present and its rows are listed, at zero.
    let mill: Vec<_> = fragment_rows(&extracted)
        .into_iter()
        .filter(|(layer, ..)| *layer == Layer::Place)
        .collect();
    assert_eq!(mill, vec![
        (Layer::Place, "The Sunken Mill", "architecture", 0.0, Prompt),
        (Layer::Place, "The Sunken Mill", "never", 0.0, Negative),
    ]);

    // And nothing of the mill is sent, under a budget with room for everything —
    // so this cannot be mistaken for the budget's doing.
    let compiled = compile(&extracted, Budget::unlimited());
    assert!(!compiled.prompt().contains("millhouse"), "{}", compiled.prompt());
    assert!(!compiled.negative().contains("Dry stonework"), "{}", compiled.negative());
    assert_eq!(report(&compiled), vec![
        (Layer::Place, "The Sunken Mill", "architecture", Silenced),
        (Layer::Place, "The Sunken Mill", "never", Silenced),
    ]);
}

/* ── the worked example ───────────────────────────────────────────────────── */

#[test]
fn the_hand_written_example_project_resolves_the_way_stacks_rs_claims_it_does() {
    // `stacks.rs` says its main world "mirrors `examples/Ashfall.wobu`", and
    // until something read that folder it was a comment rather than a fact. This
    // is the one test that can catch the two drifting apart — somebody editing
    // the example world, or the code-built one, and leaving the other behind.
    // It is also the only check that the project shipped as the worked example
    // in the guide still compiles to the stack the guide describes.
    let ashfall =
        Fixture::at(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/Ashfall.wobu"));
    let world = ashfall.world();
    let stack =
        resolve(&world, ashfall.id("Kael Vantris"), Some(Shot::new("Character sheet · 3:4")))
            .unwrap();

    assert_eq!(stack_rows(&stack), vec![
        (Layer::Style, "Ashfall House Style", 1.0),
        (Layer::World, "Ashfall", 1.0),
        (Layer::Ancestry, "Vashk", 1.0),
        (Layer::Culture, "Ember Guild", 1.0),
        (Layer::Place, "The Ember Coast", 1.0),
        (Layer::Place, "Cinder Bay", 1.0),
        (Layer::Subject, "Kael Vantris", 1.0),
        (Layer::Shot, "Character sheet · 3:4", 1.0),
    ]);
}
