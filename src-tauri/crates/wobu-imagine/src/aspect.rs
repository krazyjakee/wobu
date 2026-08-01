//! The aspect-ratio vocabulary, and the pixel dimensions a backend's ceiling
//! leaves room for.
//!
//! `wobu-core`'s presets carry `aspect` as a `&'static str` and say why: "#50
//! owns the aspect vocabulary, and it is the intersection of what each backend
//! actually accepts, which no core enum could track" (`preset.rs`). This is that
//! vocabulary. It stays out of `wobu-core` because the set is a fact about
//! providers, not about worlds — a `21:9` that every current backend takes could
//! be gone next year, and a core enum would make removing it a data-model
//! change.

use std::fmt;

use serde::{Serialize, Serializer};

/// A shape, as the token both vendors take.
///
/// **Not reduced to lowest terms**, and that is the whole subtlety of this type.
/// `21:9` is in Google's documented list and in `environment_matte`; `7:3`,
/// which is the same shape, is in neither, and a request naming it is refused.
/// Reducing in the constructor would be quietly rewriting a value we can send
/// into one we cannot — so equality here is equality of the *token*, and
/// sameness of shape is [`distance`](Self::distance) being zero.
///
/// The property reducing would have bought — that the dropdown never offers one
/// shape twice — is checked directly instead, by
/// `no_two_entries_in_the_vocabulary_are_the_same_shape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AspectRatio {
    width: u16,
    height: u16,
}

/// For [`AspectRatio::ALL`], which is written out in a const and so cannot go
/// through a constructor that returns `Option`. Not public: everything arriving
/// from outside goes through [`AspectRatio::new`], which rejects a zero side.
const fn ratio(width: u16, height: u16) -> AspectRatio {
    AspectRatio { width, height }
}

impl AspectRatio {
    /// The vocabulary presets are written in and the Generate dropdown is drawn
    /// from: the intersection both Google image docs agree on
    /// (`docs/08-providers.md`).
    ///
    /// **Not** a list of every ratio expressible — a local ComfyUI will take any
    /// two numbers, and [`AspectRatio::new`] will build them. This is the set we
    /// are willing to *offer*, which is a smaller and more useful thing: a
    /// backend declares which of these it takes, and one it does not declare
    /// does not appear in the dropdown.
    pub const ALL: [AspectRatio; 10] = [
        ratio(1, 1),
        ratio(3, 2),
        ratio(2, 3),
        ratio(3, 4),
        ratio(4, 3),
        ratio(4, 5),
        ratio(5, 4),
        ratio(9, 16),
        ratio(16, 9),
        ratio(21, 9),
    ];

    /// `None` for a ratio with a zero side, which is not a shape.
    ///
    /// Keeps both numbers exactly as given — see the note on the type.
    pub fn new(width: u16, height: u16) -> Option<AspectRatio> {
        (width != 0 && height != 0).then_some(AspectRatio { width, height })
    }

    /// From `"3:4"` — the form a preset carries and `project.json` stores.
    ///
    /// `None` rather than an error type because both callers already have one:
    /// a preset's value is checked by test at build time, and a hand-edited
    /// `project.json` is the store's problem. Trailing spaces are not accepted;
    /// this parses a vocabulary, not user prose.
    pub fn parse(text: &str) -> Option<AspectRatio> {
        let (width, height) = text.split_once(':')?;
        AspectRatio::new(width.parse().ok()?, height.parse().ok()?)
    }

    pub fn width(self) -> u16 {
        self.width
    }

    pub fn height(self) -> u16 {
        self.height
    }

    /// Width over height, for a backend that wants a float rather than a token.
    ///
    /// Deliberately not what [`distance`](Self::distance) is built on: comparing
    /// two of these is the thing that breaks ties on floating-point noise. What
    /// goes on the wire is [`fmt::Display`], because both vendors take the
    /// string.
    pub fn ratio(self) -> f32 {
        f32::from(self.width) / f32::from(self.height)
    }

    /// How different two shapes are, symmetrically. Zero for the same shape,
    /// whatever the two are spelled as.
    ///
    /// The ratio *of the ratios*, folded so it is never below one, and not the
    /// difference of the ratios. Two reasons, and both show up in the dropdown:
    ///
    /// - **The difference is not symmetric about square.** `16:9` is 1.78 and
    ///   `9:16` is 0.5625, so arithmetic distance makes every landscape ratio
    ///   look further from square than the portrait ratio that mirrors it. A
    ///   substitution picked that way turns a `3:4` character sheet square on a
    ///   backend that offers `4:3`, while leaving `4:3` alone.
    /// - **A logarithm would be symmetric but not *exactly* symmetric.**
    ///   `ln(4/3)` and `-ln(3/4)` differ in the last bit, which is enough to
    ///   decide a tie — so the same request would pick a landscape on one build
    ///   and a portrait on the next, and an `influence_snapshot` would stop
    ///   reproducing. Folding to `max(r, 1/r)` divides the same two integers in
    ///   the same order either way round, so a tie stays a tie.
    pub fn distance(self, other: AspectRatio) -> f32 {
        let ratio = f64::from(u32::from(self.width) * u32::from(other.height))
            / f64::from(u32::from(self.height) * u32::from(other.width));
        (if ratio >= 1.0 { ratio } else { ratio.recip() }) as f32 - 1.0
    }

    /// The largest image of this shape that fits inside `ceiling`.
    ///
    /// This is what makes
    /// [`Capabilities::max_resolution`](crate::Capabilities::max_resolution) do
    /// work rather than be a number in a tooltip: a `21:9` matte on a backend
    /// whose ceiling is square is 2048×877, not 2048×2048 with the top and
    /// bottom invented. Gemini rounds the answer to its nearest documented size
    /// class and ComfyUI uses it directly — no alignment is applied here,
    /// because a multiple of eight is a fact about latent diffusion and not
    /// about ratios, and folding it in would be this module growing a ComfyUI
    /// shape.
    ///
    /// Never returns a zero side: an extreme ratio against a tiny ceiling
    /// truncates towards nothing, and a request for a zero-pixel image is a
    /// failure with no useful message.
    pub fn fit(self, ceiling: Resolution) -> Resolution {
        let (width, height) = (u64::from(self.width), u64::from(self.height));
        let fitted = u64::from(ceiling.width).min(u64::from(ceiling.height) * width / height).max(1);
        Resolution {
            width: fitted as u32,
            height: (fitted * height / width).max(1) as u32,
        }
    }
}

impl fmt::Display for AspectRatio {
    /// `3:4` — the form both vendors accept, the form a preset is written in,
    /// and the form this parses back from.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.width, self.height)
    }
}

impl Serialize for AspectRatio {
    /// A string and not `{ width, height }`, so the dropdown's values are the
    /// same tokens as a preset's `aspect` and the frontend never has to
    /// reassemble one.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Pixel dimensions.
///
/// A named struct rather than the `(u32, u32)` the issue sketches, because the
/// pair is passed through three crates before it reaches a request and a
/// transposed tuple is the kind of bug that produces a plausible-looking
/// portrait where a landscape was asked for, with nothing to fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Resolution {
    pub const fn new(width: u32, height: u32) -> Resolution {
        Resolution { width, height }
    }

    /// `u64`, because 4K square is 16.7 million and a backend that grows a
    /// bigger ceiling should not silently overflow the number #55 prices with.
    pub fn pixels(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// What a paid backend charges by, near enough to estimate with. Gemini
    /// prices per image and per size class rather than per pixel, so this is
    /// the input to a local cost model and never a figure read back from a
    /// provider (`docs/08-providers.md`).
    pub fn megapixels(self) -> f32 {
        self.pixels() as f32 / 1_000_000.0
    }

    pub fn fits_in(self, ceiling: Resolution) -> bool {
        self.width <= ceiling.width && self.height <= ceiling.height
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}×{}", self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ratio_is_the_token_the_vendors_take_and_never_its_lowest_terms() {
        // The one that caught a wrong design. `21:9` reduces to `7:3`, and `7:3`
        // is in neither vendor's documented list and in no preset — so a
        // constructor that reduced would be silently turning a value we can send
        // into one that is refused, which is the exact class of failure this
        // crate exists to prevent. Equality is equality of the token; sameness
        // of shape is `distance` being zero, and the two are different questions.
        assert_eq!(AspectRatio::new(21, 9).unwrap().to_string(), "21:9");
        assert_ne!(AspectRatio::new(21, 9), AspectRatio::new(7, 3));
        assert_eq!(AspectRatio::new(21, 9).unwrap().distance(ratio(7, 3)), 0.0);
        assert_eq!(AspectRatio::new(0, 9), None, "a zero side is not a shape");
        assert_eq!(AspectRatio::new(9, 0), None);
    }

    #[test]
    fn a_ratio_parses_back_from_exactly_what_it_prints() {
        // `Generation.params` and `project.json` store the printed form, and an
        // adapter puts it on the wire. A round trip that lost a value would
        // reopen an old generation at the wrong shape.
        for aspect in AspectRatio::ALL {
            assert_eq!(AspectRatio::parse(&aspect.to_string()), Some(aspect), "{aspect}");
        }
        assert_eq!(AspectRatio::parse("21:9"), Some(ratio(21, 9)));
        assert_eq!(AspectRatio::parse("21x9"), None);
        assert_eq!(AspectRatio::parse("21:"), None);
        assert_eq!(AspectRatio::parse(""), None);
        assert_eq!(AspectRatio::parse(" 1:1 "), None, "this is a vocabulary, not prose");
    }

    #[test]
    fn every_aspect_the_preset_registry_asks_for_is_in_this_vocabulary() {
        // The cross-check `wobu-core` cannot make. `preset.rs` keeps its own
        // copy of the supported list because it may not depend on this crate,
        // and says so; this is the direction that can be checked, and without it
        // a ratio removed here leaves a preset asking for something no backend
        // offers, which fails late or — worse, per `docs/08-providers.md` —
        // is silently ignored and returns a square.
        for preset in wobu_core::preset_registry() {
            let aspect = AspectRatio::parse(preset.aspect)
                .unwrap_or_else(|| panic!("{} asks for `{}`", preset.id, preset.aspect));
            assert!(
                AspectRatio::ALL.contains(&aspect),
                "{} asks for {aspect}, which no backend is offered",
                preset.id,
            );
        }
    }

    #[test]
    fn no_two_entries_in_the_vocabulary_are_the_same_shape() {
        // What reducing would have bought, checked directly. Two tokens for one
        // shape in `ALL` would put the same picture in the dropdown twice, and
        // a backend that declared one of them would be told it does not support
        // the other.
        for (i, a) in AspectRatio::ALL.iter().enumerate() {
            for b in &AspectRatio::ALL[i + 1..] {
                assert_ne!(a.distance(*b), 0.0, "{a} and {b} are the same shape");
            }
        }
        // `ALL` is written out through a const fn that cannot check anything, so
        // this is also the only place a zero side could be caught.
        for aspect in AspectRatio::ALL {
            assert_eq!(AspectRatio::new(aspect.width, aspect.height), Some(aspect), "{aspect}");
        }
    }

    #[test]
    fn distance_is_exactly_symmetric_about_square() {
        // Two failures in one test. Under arithmetic distance `16:9` is 0.78
        // from square and `9:16` is 0.44, so a backend offering only `1:1` and
        // `9:16` would be handed a portrait for a landscape shot. Under a
        // logarithm the two are equal to within a last bit that is not zero, so
        // the tie between `4:3` and `3:4` breaks on floating-point noise and the
        // same request reproduces differently on a different build.
        let square = ratio(1, 1);
        assert_eq!(square.distance(ratio(16, 9)), square.distance(ratio(9, 16)));
        assert_eq!(square.distance(ratio(4, 3)), square.distance(ratio(3, 4)));
        assert_eq!(square.distance(square), 0.0);
        assert_eq!(ratio(16, 9).distance(square), square.distance(ratio(16, 9)));
        assert!(ratio(16, 9).distance(ratio(21, 9)) < ratio(16, 9).distance(square));
    }

    #[test]
    fn fitting_a_shape_into_a_ceiling_never_exceeds_it_and_never_reaches_zero() {
        // A width over the ceiling is a provider error; a zero side is a request
        // for no image at all, which fails with a message about nothing.
        let ceilings =
            [Resolution::new(2048, 2048), Resolution::new(1024, 768), Resolution::new(8, 8)];
        for ceiling in ceilings {
            for aspect in AspectRatio::ALL {
                let fitted = aspect.fit(ceiling);
                assert!(fitted.fits_in(ceiling), "{aspect} in {ceiling} gave {fitted}");
                assert!(fitted.width > 0 && fitted.height > 0, "{aspect} in {ceiling}");
            }
        }

        // And it fits by growing to the ceiling rather than by shrinking away
        // from it: a 4K backend asked for a square must return 4K, not 1×1.
        assert_eq!(ratio(1, 1).fit(Resolution::new(2048, 2048)), Resolution::new(2048, 2048));
        assert_eq!(ratio(21, 9).fit(Resolution::new(2048, 2048)), Resolution::new(2048, 877));
        assert_eq!(ratio(9, 16).fit(Resolution::new(2048, 2048)), Resolution::new(1152, 2048));
    }

    #[test]
    fn an_aspect_serialises_as_the_token_the_preset_registry_writes() {
        // #47 renders the dropdown from these and #46 puts them over the bridge.
        // A `{ width, height }` object would make the frontend reassemble a
        // string that already exists in `Preset::aspect`, and the two would
        // drift on the first ratio somebody spells differently.
        assert_eq!(serde_json::to_string(&ratio(3, 4)).unwrap(), r#""3:4""#);
        assert_eq!(serde_json::to_string(&ratio(21, 9)).unwrap(), r#""21:9""#);
        assert_eq!(
            serde_json::to_string(&AspectRatio::ALL).unwrap(),
            r#"["1:1","3:2","2:3","3:4","4:3","4:5","5:4","9:16","16:9","21:9"]"#,
        );
    }

    #[test]
    fn a_resolution_reports_its_own_size_without_overflowing() {
        // #55 prices batches off this. A `u32` product overflows above 4K
        // square, and a spend estimate that wraps is wrong in the direction
        // nobody notices until the bill.
        assert_eq!(Resolution::new(4096, 4096).pixels(), 16_777_216);
        assert_eq!(Resolution::new(u32::MAX, u32::MAX).pixels(), 18_446_744_065_119_617_025);
        assert!((Resolution::new(1000, 1000).megapixels() - 1.0).abs() < 1e-6);
        assert!(Resolution::new(1024, 768).fits_in(Resolution::new(1024, 1024)));
        assert!(!Resolution::new(1024, 1080).fits_in(Resolution::new(1024, 1024)));
    }
}
