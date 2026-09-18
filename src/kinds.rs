//! The bestiary.
//!
//! Every thing that can be summoned is one `Spec`.  A skob is a soft ring of
//! points; a box is the same machinery wound tight; sand is not a body at all
//! but a single grain that obeys the sandbox rules instead.

/// How a thing is put together and therefore how it moves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// A closed ring of skin around a middle point.
    Body,
    /// An open chain: links and nothing else, no inside.
    Chain,
    /// One grain, one braille dot big.
    Grain,
    /// One grain, one whole character cell big.
    BigGrain,
}

impl Mode {
    pub fn is_grain(self) -> bool {
        matches!(self, Mode::Grain | Mode::BigGrain)
    }
    /// A grain's footprint, in field pixels (2 across and 4 down per cell).
    pub fn grain_size(self) -> (i32, i32) {
        match self {
            Mode::BigGrain => (2, 4),
            _ => (1, 1),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Spec {
    pub name: &'static str,
    /// Points around the ring (bodies) or links in the chain.
    pub points: usize,
    /// Where point zero sits, in radians.  A box wants its corners at 45°.
    pub phase: f64,
    /// Multiplies the global stiffness.
    pub stiffness: f64,
    /// A point may not come closer to the middle than `shell_min` * radius,
    /// nor stray further than `shell_max` * radius.  Tight limits mean rigid.
    pub shell_min: f64,
    pub shell_max: f64,
    /// How far a point may slide round the ring from its own wedge.
    pub twist: f64,
    pub mode: Mode,
    /// A fixed radius, or None to take whatever --size says.
    pub radius: Option<f64>,
    /// Living things get eyes.  Objects do not.
    pub eyes: bool,
    /// A glint in the upper left, for things that are meant to look polished.
    pub shine: bool,
    /// Drawn as this character rather than in braille.
    pub glyph: Option<char>,
    /// The colours this kind comes in; empty means "whatever the flags say".
    pub palette: &'static [u8],
    /// What gravity does to it.  1.0 is a thing that falls; a negative number
    /// is a thing that does not.
    pub buoyancy: f64,
    /// Scales whatever size it would otherwise have been.
    pub scale: f64,
    /// A liquid finds its own level: it flows sideways when it cannot fall,
    /// nothing stands on it, and things float in it.
    pub liquid: bool,
    /// Heavier grains sink through lighter ones.
    pub density: u8,
    /// Drawn through this squash: a little narrower and taller than the circle
    /// its physics actually is.  Only a small balloon bothers.
    pub squash: (f64, f64),
    /// How much water holds it up.  1.0 floats; a box is too dense to bother
    /// and goes to the bottom.
    pub water_lift: f64,
}

/// A small balloon is drawn as an ellipse -- barely one, just enough to read as
/// a balloon rather than a ball -- while its physics stays a plain circle.  The
/// big ones are perfectly round.
pub const BALLOON_SQUASH: (f64, f64) = (0.93, 1.1);

const SAND_COLOURS: &[u8] = &[179, 180, 186, 187, 222, 223, 221, 215, 214, 143];
/// Skobs are not all one colour.  The base colour (-c) overrides this, and -r
/// is about the one you start with, not the ones you summon afterwards.
const SKOB_COLOURS: &[u8] = &[84, 80, 79, 114, 116, 117, 147, 152, 175, 176, 180, 210, 215, 222];
const ROPE_COLOURS: &[u8] = &[180, 179, 137, 138, 144, 187, 101, 143];

const WATER_COLOURS: &[u8] = &[27, 33, 39, 45, 51, 32, 38, 44, 75, 81, 117, 123, 31, 37];

pub const BESTIARY: &[Spec] = &[
    Spec {
        name: "skob",
        points: 14,
        phase: 0.0,
        stiffness: 1.0,
        shell_min: 0.38,
        shell_max: 1.65,
        twist: 0.85,
        mode: Mode::Body,
        radius: None,
        eyes: true,
        shine: false,
        glyph: None,
        palette: SKOB_COLOURS,
        buoyancy: 1.0,
        scale: 1.0,
        liquid: false,
        density: 2,
        squash: (1.0, 1.0),
        water_lift: 1.0,
    },
    Spec {
        name: "gorb",
        points: 16,
        phase: 0.0,
        stiffness: 1.0,
        shell_min: 0.94,
        shell_max: 1.06,
        twist: 0.20,
        mode: Mode::Body,
        radius: None,
        eyes: false,
        shine: true,
        glyph: None,
        // reddish orange through to yellowish, no two quite alike
        palette: &[202, 208, 214, 220, 209, 215, 166, 172, 178, 216, 221, 203],
        buoyancy: 1.0,
        scale: 1.0,
        liquid: false,
        density: 2,
        squash: (1.0, 1.0),
        water_lift: 1.0,
    },
    Spec {
        name: "box",
        points: 4,
        phase: std::f64::consts::FRAC_PI_4,
        stiffness: 1.0,
        shell_min: 0.97,
        shell_max: 1.03,
        twist: 0.04,
        mode: Mode::Body,
        radius: None,
        eyes: false,
        shine: false,
        glyph: None,
        palette: &[236, 238, 240, 242, 244, 246, 248, 250, 252, 254, 239, 245],
        buoyancy: 1.0,
        scale: 1.0,
        liquid: false,
        density: 2,
        squash: (1.0, 1.0),
        water_lift: 0.15,   // a box sinks
    },
    Spec {
        name: "amoeba",
        points: 18,
        phase: 0.0,
        stiffness: 0.30,
        shell_min: 0.12,
        shell_max: 2.40,
        twist: 1.40,
        mode: Mode::Body,
        radius: None,
        eyes: true,
        shine: false,
        glyph: None,
        palette: &[71, 77, 78, 83, 107, 113, 114, 120, 148, 150, 79, 85],
        buoyancy: 1.0,
        scale: 1.0,
        liquid: false,
        density: 2,
        squash: (1.0, 1.0),
        water_lift: 1.0,
    },
    Spec {
        name: "string",
        points: 16,
        phase: 0.0,
        stiffness: 1.0,
        shell_min: 0.0,
        shell_max: 0.0,
        twist: 0.0,
        mode: Mode::Chain,
        radius: None,
        eyes: false,
        shine: false,
        glyph: None,
        palette: ROPE_COLOURS,
        buoyancy: 1.0,
        scale: 1.0,
        liquid: false,
        density: 2,
        squash: (1.0, 1.0),
        water_lift: 1.0,
    },
    Spec {
        name: "sand",
        points: 1,
        phase: 0.0,
        stiffness: 1.0,
        shell_min: 0.0,
        shell_max: 0.0,
        twist: 0.0,
        mode: Mode::Grain,
        radius: Some(1.0),
        eyes: false,
        shine: false,
        glyph: None,
        palette: SAND_COLOURS,
        buoyancy: 1.0,
        scale: 1.0,
        liquid: false,
        density: 2,
        squash: (1.0, 1.0),
        water_lift: 1.0,
    },
    Spec {
        name: "bigsand",
        points: 1,
        phase: 0.0,
        stiffness: 1.0,
        shell_min: 0.0,
        shell_max: 0.0,
        twist: 0.0,
        mode: Mode::BigGrain,
        radius: Some(4.0),
        eyes: false,
        shine: false,
        glyph: Some('#'),
        palette: SAND_COLOURS,
        buoyancy: 1.0,
        scale: 1.0,
        liquid: false,
        density: 2,
        squash: (1.0, 1.0),
        water_lift: 1.0,
    },
    Spec {
        name: "balloon",
        points: 12,
        phase: std::f64::consts::FRAC_PI_2,
        stiffness: 1.0,
        shell_min: 0.88,
        shell_max: 1.12,
        twist: 0.30,
        mode: Mode::Body,
        radius: None,
        eyes: false,
        shine: true,
        glyph: None,
        // vibrant, every one of them: no two balloons at a party match
        palette: &[
            196, 202, 208, 214, 220, 226, 190, 154, 118, 82, 46, 51, 45, 39, 33, 27, 21, 57, 93,
            129, 165, 201, 199, 205, 213, 219,
        ],
        buoyancy: -1.35,
        scale: 0.42,        // a balloon is a small thing on a long string
        liquid: false,
        density: 1,
        squash: BALLOON_SQUASH,
        water_lift: 1.0,
    },
    Spec {
        name: "bigballoon",
        points: 14,
        phase: std::f64::consts::FRAC_PI_2,
        stiffness: 1.0,
        shell_min: 0.88,
        shell_max: 1.12,
        twist: 0.30,
        mode: Mode::Body,
        radius: None,
        eyes: false,
        shine: true,
        glyph: None,
        palette: &[
            196, 202, 208, 214, 220, 226, 190, 154, 118, 82, 46, 51, 45, 39, 33, 27, 21, 57, 93,
            129, 165, 201, 199, 205, 213, 219,
        ],
        buoyancy: -1.35,
        scale: 0.95,        // the one that carries off whatever you like
        liquid: false,
        density: 1,
        squash: (1.0, 1.0),
        water_lift: 1.0,
    },
    Spec {
        name: "water",
        points: 1,
        phase: 0.0,
        stiffness: 1.0,
        shell_min: 0.0,
        shell_max: 0.0,
        twist: 0.0,
        mode: Mode::Grain,
        radius: Some(1.0),
        eyes: false,
        shine: false,
        glyph: None,
        palette: WATER_COLOURS,
        buoyancy: 1.0,
        scale: 1.0,
        liquid: true,
        density: 1,
        squash: (1.0, 1.0),
        water_lift: 1.0,
    },
    Spec {
        name: "bigwater",
        points: 1,
        phase: 0.0,
        stiffness: 1.0,
        shell_min: 0.0,
        shell_max: 0.0,
        twist: 0.0,
        mode: Mode::BigGrain,
        radius: Some(4.0),
        eyes: false,
        shine: false,
        glyph: Some('~'),
        palette: WATER_COLOURS,
        buoyancy: 1.0,
        scale: 1.0,
        liquid: true,
        density: 1,
        squash: (1.0, 1.0),
        water_lift: 1.0,
    },
];

pub fn lookup(name: &str) -> Option<&'static Spec> {
    BESTIARY.iter().find(|s| s.name == name)
}

pub fn names() -> String {
    BESTIARY.iter().map(|s| s.name).collect::<Vec<_>>().join("|")
}

/// One gorb, two gorbs -- but never two sands, and never two waters.
pub fn plural(name: &str, n: usize) -> String {
    if n == 1 || name.ends_with("sand") || name.ends_with("water") {
        name.to_string()
    } else {
        format!("{}s", name)
    }
}
