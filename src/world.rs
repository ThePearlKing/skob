//! The physics.
//!
//! Bodies are verlet integrated: a point has no stored velocity, only the gap
//! between where it is and where it just was.  Everything else -- springs,
//! the hard shell, the twist limit, the walls, the letters, the sandbox --
//! is written as a position fix applied after the fact, several times a frame.

use crate::kinds::{Mode, Spec};
use std::collections::HashMap;

/// Field pixels per character cell.  Braille gives us 2 across and 4 down.
pub const CELL_W: i32 = 2;
pub const CELL_H: i32 = 4;

const SOLVER_PASSES: usize = 6;
/// How much of the sideways slip between two touching skins is taken out of
/// them each time they are pushed apart.  It is what turns sliding into
/// turning: at zero everything skates over everything else, perfectly square.
const SKIN_GRIP: f64 = 0.45;
/// How fast a thing with a wall inside it works its way off it, in field
/// pixels a frame.  Brisk enough to be out within a breath, slow enough that
/// it looks like getting unstuck rather than being fired out of a cannon.
const WALL_EJECT: f64 = 1.5;
/// Samples taken along each span of skin, so a letter cannot slip between two
/// points of the ring without touching either of them.
const SKIN_SAMPLES: usize = 3;

/// A cheap, seedable generator.  Nothing here needs to be unpredictable.
pub struct Rng(u64);

impl Rng {
    pub fn new() -> Rng {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5EED);
        Rng(seed | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
    pub fn float(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// How long a balloon string is, in field pixels: about three rows.
const STRING_LEN: f64 = 12.0;
/// How much lift a balloon has, area for area, against the weight of what it
/// is tied to.  A small balloon is about a third of a skob, so one of them only
/// makes a skob bouncy, three of them just about carry it off, and one of the
/// big ones does it on its own.
const LIFT: f64 = 2.0;
/// How hard water pushes back on what is in it.  Above 1.0 a thing that is
/// wholly under rises; it settles at the depth where the two balance out.
const WATER_LIFT: f64 = 1.8;
/// And how much the water slows it down while it is in there.
const WATER_DRAG: f64 = 0.35;

/// How much more skin a bag has than it needs to go round what is inside it.
/// This one number is the whole difference between a ball and an amoeba: a
/// circle is the most area a given length of skin can hold, so a bag carrying
/// less than that can never be a circle, and every lopsided shape that holds
/// the right area is as settled as every other.  It has nothing to spring
/// back to.
const MEMBRANE_SLACK: f64 = 0.30;
/// How hard the area is held.  1.0 is all of it at once, which shudders; this
/// is per pass, and there are six of them.
const AREA_FIX: f64 = 0.45;

/// What a grain finds when it looks at the spot it would like to move into.
enum Spot {
    Free,
    /// Another grain, which may be lighter and therefore swappable.
    Grain(usize),
    /// A letter, a body, the edge of the world: not going there.
    Solid,
}

pub struct Body {
    /// Stable for the life of the body, so a tether can name it.
    pub id: usize,
    pub spec: &'static Spec,
    pub colour: u8,
    /// A body's radius, a chain's link length, a grain's half-width.
    pub rest: f64,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub ox: Vec<f64>,
    pub oy: Vec<f64>,
    /// A balloon tied to something: the id of whatever it will carry off.
    pub tether: Option<usize>,
    /// For a bag: how far out each bit of skin is being pushed just now, as a
    /// fraction of the radius.  It is not a formula but a memory -- it wanders
    /// where the last few hundred frames left it, which is why an amoeba has
    /// no shape it goes back to.
    pub flow: Vec<f64>,
}

impl Body {
    /// Index of the middle point.  Only a `Body` has one; for everything else
    /// this is one past the end and is never asked for.
    pub fn middle(&self) -> usize {
        self.spec.points
    }
    pub fn centre(&self) -> (f64, f64) {
        match self.spec.mode {
            Mode::Body => (self.x[self.middle()], self.y[self.middle()]),
            _ => (self.x[0], self.y[0]),
        }
    }
    pub fn is_grain(&self) -> bool {
        self.spec.mode.is_grain()
    }
}

/// Which character cells are rock.  Only the bottom rows of the shell count:
/// above them a skob is a ghost and drifts straight through the text.
pub struct Ground {
    pub top_px: i32,
    pub cols: i32,
    pub rows: i32,
    solid: Vec<bool>,
}

impl Ground {
    pub fn empty() -> Ground {
        Ground { top_px: i32::MAX, cols: 0, rows: 0, solid: Vec::new() }
    }

    /// `lines` are the bottom rows of the screen, top first, each already
    /// stripped of colour.  `first_row` is where they start, counting cell
    /// rows from zero.
    pub fn from_text(first_row: i32, cols: i32, rows: i32, lines: &[(i32, String)]) -> Ground {
        let mut g = Ground {
            top_px: first_row * CELL_H,
            cols,
            rows,
            solid: vec![false; (cols.max(1) * rows.max(1)) as usize],
        };
        for (row, text) in lines {
            for (col, ch) in text.chars().enumerate() {
                if ch != ' ' && (col as i32) < cols && *row >= 0 && *row < rows {
                    let i = (row * cols + col as i32) as usize;
                    g.solid[i] = true;
                }
            }
        }
        g
    }

    pub fn is_solid(&self, row: i32, col: i32) -> bool {
        if row < 0 || col < 0 || row >= self.rows || col >= self.cols {
            return false;
        }
        self.solid[(row * self.cols + col) as usize]
    }

    /// Is there any ink at all below this row, in these columns?
    pub fn ink_below(&self, from_row: i32, col0: i32, col1: i32) -> bool {
        if !self.any() {
            return false;
        }
        (from_row.max(0)..self.rows)
            .any(|row| (col0.max(0)..=col1.min(self.cols - 1)).any(|col| self.is_solid(row, col)))
    }

    pub fn any(&self) -> bool {
        !self.solid.is_empty()
    }
}

pub struct World {
    pub bodies: Vec<Body>,
    pub width: i32,  // field pixels across
    pub height: i32, // field pixels down
    pub gravity: f64,
    pub stiffness: f64,
    pub bounce: f64,
    pub friction: f64,
    pub paused: bool,
    pub ground: Ground,
    /// Which point the mouse has hold of, if any.
    pub grab: Option<(usize, usize)>,
    /// Grains in the hand, by their own id, each with where it sat in the cell
    /// it was lifted out of -- so a handful keeps its shape while it is
    /// carried, and picking it up does not shuffle it.
    pub handful: Vec<(usize, f64, f64)>,
    pub mouse: (f64, f64),
    pub rng: Rng,
    next_id: usize,
    /// One flag per character cell, rebuilt every frame.  Sand is something to
    /// stand on; water is something to be in.  They are counted apart because
    /// they do entirely different things to whatever is above them.
    grain_cells: Vec<bool>,
    water_cells: Vec<bool>,
    mask_cols: i32,
    mask_rows: i32,
    /// Frames since the world began.  Only the things that flow care.
    pub tick: u64,
}

impl World {
    pub fn new(width: i32, height: i32) -> World {
        World {
            bodies: Vec::new(),
            width,
            height,
            gravity: 0.32,
            stiffness: 0.55,
            bounce: 0.45,
            friction: 0.995,
            paused: false,
            ground: Ground::empty(),
            grab: None,
            handful: Vec::new(),
            mouse: (-1.0, -1.0),
            rng: Rng::new(),
            next_id: 0,
            grain_cells: Vec::new(),
            water_cells: Vec::new(),
            mask_cols: 0,
            mask_rows: 0,
            tick: 0,
        }
    }

    pub fn spawn(&mut self, spec: &'static Spec, cx: f64, cy: f64, radius: f64, colour: u8) -> usize {
        let n = spec.points;
        let id = self.next_id;
        self.next_id += 1;
        let mut body = Body {
            id,
            spec,
            colour,
            rest: radius,
            x: Vec::new(),
            y: Vec::new(),
            ox: Vec::new(),
            oy: Vec::new(),
            tether: None,
            flow: vec![1.0; if spec.ooze > 0.0 { n } else { 0 }],
        };
        match spec.mode {
            Mode::Body => {
                for i in 0..n {
                    let a = std::f64::consts::TAU * i as f64 / n as f64 + spec.phase;
                    body.x.push(cx + radius * a.cos());
                    body.y.push(cy + radius * a.sin());
                }
                body.x.push(cx);
                body.y.push(cy);
            }
            Mode::Chain => {
                // A rope is laid out flat and left to fall however it likes.
                let link = (radius / 3.0).max(2.0);
                body.rest = link;
                for i in 0..n {
                    body.x.push(cx - (n as f64 - 1.0) * link / 2.0 + i as f64 * link);
                    body.y.push(cy);
                }
            }
            Mode::Grain | Mode::BigGrain => {
                body.x.push(cx);
                body.y.push(cy);
            }
        }
        body.ox = body.x.clone();
        body.oy = body.y.clone();
        self.bodies.push(body);
        id
    }

    pub fn index_of(&self, id: usize) -> Option<usize> {
        self.bodies.iter().position(|b| b.id == id)
    }

    /// The thing at this spot that a balloon could be tied to, if any.
    /// Is the character cell this point falls in already spoken for -- by a
    /// grain, a drop, a wall or the letters underneath?  What it is for is
    /// drawing with the mouse held down, where the same cell comes round again
    /// and again and nothing should be stacked inside anything else.
    pub fn cell_taken(&self, x: f64, y: f64) -> bool {
        let (row, col) = (y as i32 / CELL_H, x as i32 / CELL_W);
        self.ground.is_solid(row, col)
            || self.solid_grain_in_cell(row, col)
            || self.water_in_cell(row, col)
    }

    /// Spoken for as of now, rather than as of the next frame, so that two
    /// grains put down in the same sweep of the mouse cannot land in one cell.
    pub fn take_cell(&mut self, x: f64, y: f64) {
        let (row, col) = (y as i32 / CELL_H, x as i32 / CELL_W);
        if row >= 0 && col >= 0 && row < self.mask_rows && col < self.mask_cols {
            self.grain_cells[(row * self.mask_cols + col) as usize] = true;
        }
    }

    pub fn body_at(&self, x: f64, y: f64) -> Option<usize> {
        self.bodies
            .iter()
            .filter(|b| !b.is_grain() && b.spec.name != "balloon")
            .filter(|b| {
                let (cx, cy) = b.centre();
                (cx - x).powi(2) + (cy - y).powi(2) <= b.rest * b.rest
            })
            .map(|b| b.id)
            .last()
    }

    /// Where a balloon is holding on: the knot at the bottom of it, and the
    /// thing it is tied to, if it is tied to anything.
    pub fn string_ends(&self, bi: usize) -> ((f64, f64), (f64, f64)) {
        let b = &self.bodies[bi];
        let (cx, cy) = b.centre();
        let knot = (cx, cy + b.rest * b.spec.squash.1);
        let anchor = b
            .tether
            .and_then(|id| self.index_of(id))
            .map(|ti| self.bodies[ti].centre())
            .unwrap_or((knot.0, knot.1 + STRING_LEN));
        (knot, anchor)
    }

    /// Is any part of this thing inside a circle drawn on the screen?  The
    /// radius is in columns, and rows count double, because a character cell is
    /// about twice as tall as it is wide and a brush should look round.
    pub fn within(&self, bi: usize, x: f64, y: f64, radius_cols: f64) -> bool {
        let b = &self.bodies[bi];
        let r2 = radius_cols * radius_cols;
        (0..b.x.len()).any(|i| {
            let dx = (b.x[i] - x) / CELL_W as f64;
            let dy = (b.y[i] - y) / CELL_H as f64 * 2.0;
            dx * dx + dy * dy <= r2
        })
    }

    /// Take away everything the brush is over, and return how much that was.
    pub fn erase(&mut self, x: f64, y: f64, radius_cols: f64) -> usize {
        let doomed: Vec<bool> =
            (0..self.bodies.len()).map(|i| self.within(i, x, y, radius_cols)).collect();
        let before = self.bodies.len();
        let mut keep = doomed.iter();
        self.bodies.retain(|_| !keep.next().copied().unwrap_or(false));
        if self.grab.map_or(false, |(b, _)| doomed.get(b).copied().unwrap_or(false)) {
            self.grab = None; // whatever was held is not there any more
        }
        before - self.bodies.len()
    }

    pub fn banish(&mut self) {
        self.bodies.clear();
        self.grab = None;
    }

    /// New output has arrived and the text is scrolling up; shove everything
    /// standing on it up as well, a few rows a frame rather than all at once.
    pub fn shove_up(&mut self, pixels: f64, above: f64) {
        if pixels <= 0.0 {
            return;
        }
        // Borrowed apart so the ink can be asked about while the bodies move.
        let ground = &self.ground;
        for b in &mut self.bodies {
            let lowest = b.y.iter().cloned().fold(f64::MIN, f64::max);
            let highest = b.y.iter().cloned().fold(f64::MAX, f64::min);
            if lowest <= above {
                continue;
            }
            // Only things with text under them ride up with it.  Out over bare
            // screen there is nothing rising, so nothing to be shoved by.
            let left = b.x.iter().cloned().fold(f64::MAX, f64::min) as i32 / CELL_W;
            let right = b.x.iter().cloned().fold(f64::MIN, f64::max) as i32 / CELL_W;
            if !ground.ink_below(lowest as i32 / CELL_H, left, right) {
                continue;
            }
            let shift = pixels.min(highest);
            if shift <= 0.0 {
                continue;
            }
            for i in 0..b.y.len() {
                b.y[i] -= shift;
                b.oy[i] -= shift - 1.2;
            }
        }
    }

    pub fn step(&mut self) {
        if self.paused {
            return;
        }
        self.tick += 1;
        self.rebuild_grain_mask();
        let lift = self.tether_lift();
        let wet: Vec<f64> = (0..self.bodies.len()).map(|i| self.submersion(i)).collect();
        for i in 0..self.bodies.len() {
            if self.bodies[i].is_grain() {
                continue;
            }
            // A bag leans on its own skin once a frame -- not once a pass, or
            // it would spend the whole day churning and skidding about on the
            // spot instead of getting anywhere.
            if self.bodies[i].spec.ooze > 0.0 {
                self.drift_flow(i);
                let amount = self.bodies[i].rest * self.bodies[i].spec.ooze * 0.10;
                self.push_out_skin(i, amount);
            }
            let up = lift.get(&self.bodies[i].id).copied().unwrap_or(0.0);
            self.integrate(i, up, wet[i]);
            self.sweep_solid(i);
            self.solve(i);
        }
        self.carry_handful();
        self.tighten_strings();
        self.separate();
        self.bump_walls();
        self.cough_up_walls();
        self.sandbox();
    }

    /// A grain is a dot, but to a body it is the whole cell it sits in: a skob
    /// lands on a pile of sand rather than sinking into the gaps between the
    /// grains, and one mask of cells answers for all of them at once.
    fn rebuild_grain_mask(&mut self) {
        let cols = (self.width / CELL_W).max(1);
        let rows = (self.height / CELL_H + 1).max(1);
        if self.mask_cols != cols || self.mask_rows != rows {
            self.mask_cols = cols;
            self.mask_rows = rows;
            self.grain_cells = vec![false; (cols * rows) as usize];
            self.water_cells = vec![false; (cols * rows) as usize];
        } else {
            self.grain_cells.iter_mut().for_each(|c| *c = false);
            self.water_cells.iter_mut().for_each(|c| *c = false);
        }
        for b in &self.bodies {
            if !b.is_grain() {
                continue;
            }
            let (gw, gh) = b.spec.mode.grain_size();
            let (x, y) = (b.x[0] as i32, b.y[0] as i32);
            for i in (0..gw).step_by(CELL_W as usize).chain(std::iter::once(gw - 1)) {
                for j in (0..gh).step_by(CELL_H as usize).chain(std::iter::once(gh - 1)) {
                    let (row, col) = ((y + j) / CELL_H, (x + i) / CELL_W);
                    if row >= 0 && col >= 0 && row < rows && col < cols {
                        let cell = (row * cols + col) as usize;
                        if b.spec.liquid {
                            self.water_cells[cell] = true;
                        } else {
                            self.grain_cells[cell] = true;
                        }
                    }
                }
            }
        }
    }

    fn water_in_cell(&self, row: i32, col: i32) -> bool {
        if row < 0 || col < 0 || row >= self.mask_rows || col >= self.mask_cols {
            return false;
        }
        self.water_cells[(row * self.mask_cols + col) as usize]
    }

    /// How much of a thing is under water, from none of it to all of it.
    fn submersion(&self, bi: usize) -> f64 {
        if self.water_cells.is_empty() {
            return 0.0;
        }
        let b = &self.bodies[bi];
        let wet = (0..b.x.len())
            .filter(|&i| self.water_in_cell(b.y[i] as i32 / CELL_H, b.x[i] as i32 / CELL_W))
            .count();
        wet as f64 / b.x.len() as f64
    }

    fn solid_grain_in_cell(&self, row: i32, col: i32) -> bool {
        if row < 0 || col < 0 || row >= self.mask_rows || col >= self.mask_cols {
            return false;
        }
        self.grain_cells[(row * self.mask_cols + col) as usize]
    }

    /// Anything a body cannot be inside of: a letter of the ground, or a grain.
    fn blocked(&self, row: i32, col: i32) -> bool {
        self.ground.is_solid(row, col) || self.solid_grain_in_cell(row, col)
    }

    /// What each thing is being pulled upwards by: the balloons tied to it,
    /// weighed against how big the thing is.  A big balloon on a small skob
    /// carries it off; a small one on a big one only makes it bouncy.
    fn tether_lift(&self) -> HashMap<usize, f64> {
        let mut lift: HashMap<usize, f64> = HashMap::new();
        for balloon in &self.bodies {
            let target = match balloon.tether.and_then(|id| self.index_of(id)) {
                Some(t) => &self.bodies[t],
                None => continue,
            };
            // Area for area: what the balloon displaces against what it carries.
            let ratio = (balloon.rest * balloon.rest * LIFT) / (target.rest * target.rest);
            *lift.entry(target.id).or_insert(0.0) += self.gravity * ratio.min(4.0);
        }
        lift
    }

    /// A string is a rope, not a rod: it pulls when it runs out of slack and
    /// does nothing at all until then.
    fn tighten_strings(&mut self) {
        for bi in 0..self.bodies.len() {
            let id = match self.bodies[bi].tether {
                Some(id) => id,
                None => continue,
            };
            let ti = match self.index_of(id) {
                Some(t) => t,
                None => {
                    self.bodies[bi].tether = None; // whatever it held is gone
                    continue;
                }
            };
            let (bx, by) = self.bodies[bi].centre();
            let (tx, ty) = self.bodies[ti].centre();
            let slack = self.bodies[bi].rest + self.bodies[ti].rest + STRING_LEN;
            let (dx, dy) = (tx - bx, ty - by);
            let d = (dx * dx + dy * dy).sqrt();
            if d <= slack || d < 0.001 {
                continue;
            }
            // The balloon gives way first: it is the lighter of the two.
            let pull = (d - slack) / d;
            for i in 0..self.bodies[bi].x.len() {
                self.bodies[bi].x[i] += dx * pull * 0.8;
                self.bodies[bi].y[i] += dy * pull * 0.8;
            }
            for i in 0..self.bodies[ti].x.len() {
                self.bodies[ti].x[i] -= dx * pull * 0.2;
                self.bodies[ti].y[i] -= dy * pull * 0.2;
            }
        }
    }

    /// Where a point was, where it is, and therefore where it goes next.
    fn integrate(&mut self, bi: usize, lift: f64, wet: f64) {
        let (grab_body, grab_point) = match self.grab {
            Some((b, p)) => (b as i64, p as i64),
            None => (-1, -1),
        };
        let (gx, gy) = self.mouse;
        // Water pushes back on what is in it, and slows it down while it is
        // there: a skob dropped in one sinks, slows, and comes back up to float.
        let friction =
            self.friction * (1.0 - WATER_DRAG * wet) * self.bodies[bi].spec.drag;
        let b = &mut self.bodies[bi];
        // A balloon has its own idea of which way down is.
        let gravity =
            self.gravity * b.spec.buoyancy - lift - self.gravity * WATER_LIFT * wet * b.spec.water_lift;
        let max_speed = b.rest * if b.spec.mode == Mode::Body { 0.7 } else { 3.0 };

        for i in 0..b.x.len() {
            if grab_body == bi as i64 && grab_point == i as i64 {
                // A held point is dragged, not pushed: it forgets its momentum
                // so it cannot fling itself out of your hand.
                b.ox[i] = b.x[i];
                b.oy[i] = b.y[i];
                b.x[i] += (gx - b.x[i]) * 0.45;
                b.y[i] += (gy - b.y[i]) * 0.45;
                continue;
            }
            let mut vx = (b.x[i] - b.ox[i]) * friction;
            let mut vy = (b.y[i] - b.oy[i]) * friction;
            let speed = (vx * vx + vy * vy).sqrt();
            if speed > max_speed {
                vx *= max_speed / speed;
                vy *= max_speed / speed;
            }
            b.ox[i] = b.x[i];
            b.oy[i] = b.y[i];
            b.x[i] += vx;
            b.y[i] += vy + gravity;
        }
    }

    /// The passes only ever move points.  What a collision does to the speed of
    /// a point is settled once, at the end, from whether it touched anything at
    /// all -- reflecting its velocity six times a frame is what made a crowded
    /// pile shimmer: every point in contact changed its mind on every pass.
    fn solve(&mut self, bi: usize) {
        let n = self.bodies[bi].x.len();
        let mut struck_x = vec![false; n];
        let mut struck_y = vec![false; n];
        for _ in 0..SOLVER_PASSES {
            match self.bodies[bi].spec.mode {
                Mode::Chain => self.solve_chain(bi),
                _ if self.bodies[bi].spec.ooze > 0.0 => self.solve_blob(bi),
                _ => self.solve_ring(bi),
            }
            self.hit_walls(bi, &mut struck_x, &mut struck_y);
            self.hit_letters(bi, &mut struck_x, &mut struck_y);
        }
        let bounce = self.bounce * self.bodies[bi].spec.grip;
        let b = &mut self.bodies[bi];
        for i in 0..n {
            if struck_x[i] {
                b.ox[i] = b.x[i] + (b.ox[i] - b.x[i]) * bounce;
            }
            if struck_y[i] {
                b.oy[i] = b.y[i] + (b.oy[i] - b.y[i]) * bounce;
            }
        }
    }

    fn solve_chain(&mut self, bi: usize) {
        let n = self.bodies[bi].spec.points;
        let rest = self.bodies[bi].rest;
        for i in 0..n.saturating_sub(1) {
            self.spring(bi, i, i + 1, rest, 1.0);
        }
    }

    fn solve_ring(&mut self, bi: usize) {
        let spec = self.bodies[bi].spec;
        let n = spec.points;
        let mid = n;
        let rest = self.bodies[bi].rest;

        // The ring holds hands with itself, and every point holds the middle.
        let neighbour = rest * 2.0 * (std::f64::consts::PI / n as f64).sin();
        for i in 0..n {
            self.spring(bi, i, (i + 1) % n, neighbour, spec.stiffness);
            self.spring(bi, i, mid, rest, spec.stiffness);
        }

        let b = &mut self.bodies[bi];
        let (cx, cy) = (b.x[mid], b.y[mid]);

        // The hard shell: how far in and out the skin may go at all.
        let lo = rest * spec.shell_min;
        let hi = rest * spec.shell_max;
        for i in 0..n {
            let (mut px, mut py) = (b.x[i] - cx, b.y[i] - cy);
            let mut d = (px * px + py * py).sqrt();
            if d < 1e-6 {
                px = 1.0;
                py = 0.0;
                d = 1.0;
            }
            if d < lo {
                b.x[i] = cx + px / d * lo;
                b.y[i] = cy + py / d * lo;
            } else if d > hi {
                b.x[i] = cx + px / d * hi;
                b.y[i] = cy + py / d * hi;
            }
        }

        // It may spin freely, so find the twist the ring has as a whole...
        let mut sc = 0.0;
        let mut ss = 0.0;
        let mut angle = vec![0.0f64; n];
        for i in 0..n {
            angle[i] = (b.y[i] - cy).atan2(b.x[i] - cx);
            let off = angle[i] - std::f64::consts::TAU * i as f64 / n as f64;
            sc += off.cos();
            ss += off.sin();
        }
        let twist = ss.atan2(sc);

        // ...and then pin each point near its own wedge of that.
        let limit = std::f64::consts::PI / n as f64 * spec.twist;
        for i in 0..n {
            let nominal = std::f64::consts::TAU * i as f64 / n as f64 + twist;
            let mut off = angle[i] - nominal;
            while off > std::f64::consts::PI {
                off -= std::f64::consts::TAU;
            }
            while off < -std::f64::consts::PI {
                off += std::f64::consts::TAU;
            }
            off = off.clamp(-limit, limit);
            let r = ((b.x[i] - cx).powi(2) + (b.y[i] - cy).powi(2)).sqrt();
            b.x[i] = cx + r * (nominal + off).cos();
            b.y[i] = cy + r * (nominal + off).sin();
        }
    }

    /// A bag of fluid with a slack skin, which is what an amoeba is.
    ///
    /// Nothing here says what shape it should be.  The skin keeps its own
    /// length, the inside keeps its own area, and because there is more skin
    /// than a circle of that area needs, every shape it can fold itself into
    /// holds just as well as any other -- so its shape is only ever the sum of
    /// what has happened to it: where it landed, where you dragged it, and
    /// where it last pushed itself out.  The middle point is not a hub any of
    /// this hangs from; it only rides along at the centre of the crowd so the
    /// face has somewhere to be.
    fn solve_blob(&mut self, bi: usize) {
        let spec = self.bodies[bi].spec;
        let n = spec.points;
        let mid = n;
        let rest = self.bodies[bi].rest;

        // The skin: it holds its length, and that is the only length it holds.
        let skin = rest * 2.0 * (std::f64::consts::PI / n as f64).sin();
        for i in 0..n {
            self.spring(bi, i, (i + 1) % n, skin, spec.stiffness);
        }

        // The inside: as much of it as there always was, however it is folded.
        let target = std::f64::consts::PI * rest * rest * (1.0 - MEMBRANE_SLACK);
        self.hold_area(bi, target);

        // The face rides in the middle of whatever shape it has ended up.
        let centre = self.centroid(bi);
        let b = &mut self.bodies[bi];
        b.x[mid] = centre.0;
        b.y[mid] = centre.1;

        // A generous shell, there only so that nothing can run away with it.
        let lo = rest * spec.shell_min;
        let hi = rest * spec.shell_max;
        for i in 0..n {
            let (px, py) = (b.x[i] - centre.0, b.y[i] - centre.1);
            let d = (px * px + py * py).sqrt();
            if d > 1e-6 && (d < lo || d > hi) {
                let r = d.clamp(lo, hi);
                b.x[i] = centre.0 + px / d * r;
                b.y[i] = centre.1 + py / d * r;
            }
        }

        // Points keep their order round the skin, loosely -- a bag may fold,
        // but it may not turn itself inside out.
        self.keep_order(bi, centre);
    }

    /// Where the skin is, on average.
    fn centroid(&self, bi: usize) -> (f64, f64) {
        let b = &self.bodies[bi];
        let n = b.spec.points;
        let (mut x, mut y) = (0.0, 0.0);
        for i in 0..n {
            x += b.x[i];
            y += b.y[i];
        }
        (x / n as f64, y / n as f64)
    }

    /// The way out, for the point between these two neighbours, and how much
    /// of the skin is leaning that way.  It is the direction that changes the
    /// area fastest, which is the only honest meaning of "out" for a bag.
    fn out_of(b: &Body, i: usize, n: usize, sign: f64) -> (f64, f64) {
        let (p, q) = ((i + n - 1) % n, (i + 1) % n);
        (0.5 * (b.y[q] - b.y[p]) * sign, 0.5 * (b.x[p] - b.x[q]) * sign)
    }

    /// Which way round the skin is wound, as the sign of the area it encloses.
    fn winding(b: &Body, n: usize) -> f64 {
        let mut twice_area = 0.0;
        for i in 0..n {
            let j = (i + 1) % n;
            twice_area += b.x[i] * b.y[j] - b.x[j] * b.y[i];
        }
        if twice_area < 0.0 {
            -1.0
        } else {
            1.0
        }
    }

    /// Hold the area inside the ring, wherever the skin has got to: squeeze a
    /// bag of water in one place and it comes out in another.  Every point is
    /// moved from the one shape, not one after another, because the ways out
    /// of a closed ring cancel exactly -- which is what stops the bag from
    /// rowing itself across the screen.
    fn hold_area(&mut self, bi: usize, target: f64) {
        let n = self.bodies[bi].spec.points;
        let held = self.grab.filter(|(g, _)| *g == bi).map(|(_, p)| p);
        let b = &mut self.bodies[bi];
        let sign = Self::winding(b, n);
        let mut area = 0.0;
        for i in 0..n {
            let j = (i + 1) % n;
            area += b.x[i] * b.y[j] - b.x[j] * b.y[i];
        }
        let error = target - (area * 0.5).abs();
        let out: Vec<(f64, f64)> = (0..n).map(|i| Self::out_of(b, i, n, sign)).collect();
        let weight: f64 = out.iter().map(|(x, y)| x * x + y * y).sum();
        if weight < 1e-9 {
            return;
        }
        let lambda = error / weight * AREA_FIX;
        for i in 0..n {
            if held == Some(i) {
                continue;
            }
            b.x[i] += lambda * out[i].0;
            b.y[i] += lambda * out[i].1;
        }
    }

    /// Cytoplasm going somewhere: each bit of skin is leaned on from the
    /// inside by however much `flow` says, along its own way out.
    ///
    /// The average lean is taken off first, so the bag can push itself into a
    /// pseudopod but can never push itself along -- and the push moves where a
    /// point was as well as where it is, so it changes the shape without
    /// handing the thing any speed it did not earn.  Between them that is why
    /// it oozes instead of flying about.
    fn push_out_skin(&mut self, bi: usize, amount: f64) {
        let n = self.bodies[bi].spec.points;
        let held = self.grab.filter(|(g, _)| *g == bi).map(|(_, p)| p);
        let b = &mut self.bodies[bi];
        if b.flow.len() != n || amount <= 0.0 {
            return;
        }
        let sign = Self::winding(b, n);
        let mut step: Vec<(f64, f64)> = Vec::with_capacity(n);
        for i in 0..n {
            let (ox, oy) = Self::out_of(b, i, n, sign);
            let len = (ox * ox + oy * oy).sqrt();
            if len < 1e-6 {
                step.push((0.0, 0.0));
            } else {
                step.push((ox / len * b.flow[i] * amount, oy / len * b.flow[i] * amount));
            }
        }
        let mean = (
            step.iter().map(|s| s.0).sum::<f64>() / n as f64,
            step.iter().map(|s| s.1).sum::<f64>() / n as f64,
        );
        for i in 0..n {
            if held == Some(i) {
                continue;
            }
            let (dx, dy) = (step[i].0 - mean.0, step[i].1 - mean.1);
            b.x[i] += dx;
            b.y[i] += dy;
            b.ox[i] += dx;
            b.oy[i] += dy;
        }
    }

    /// The flow wandering on: now and then a bit of skin starts leaning out,
    /// or stops; what each bit is doing bleeds into its neighbours; and all of
    /// it fades.  There is no pattern underneath it and nothing it returns to,
    /// which is the difference between a thing that is alive and a thing that
    /// is being animated.
    fn drift_flow(&mut self, bi: usize) {
        let n = self.bodies[bi].spec.points;
        if self.bodies[bi].flow.len() != n {
            self.bodies[bi].flow = vec![0.0; n];
        }
        // A new pseudopod every second or so, somewhere, one way or the other.
        if self.rng.below(100) < 4 {
            let k = self.rng.below(n);
            let strength = self.rng.float() * 2.0 - 0.8;
            for d in -2i32..=2 {
                let i = ((k as i32 + d).rem_euclid(n as i32)) as usize;
                self.bodies[bi].flow[i] += strength * (1.0 - d.abs() as f64 / 3.0);
            }
        }
        let was = self.bodies[bi].flow.clone();
        let b = &mut self.bodies[bi];
        let mut total = 0.0;
        for i in 0..n {
            let (p, q) = ((i + n - 1) % n, (i + 1) % n);
            b.flow[i] = (was[i] * 0.90 + (was[p] + was[q]) * 0.05) * 0.995;
            total += b.flow[i];
        }
        // Leaning out everywhere at once is not a pseudopod, only a bigger
        // amoeba, and it is the area that decides how big it is.
        let mean = total / n as f64;
        for i in 0..n {
            b.flow[i] = (b.flow[i] - mean).clamp(-1.6, 1.6);
        }
    }

    /// Each point keeps to its own wedge of the ring, give or take, so the
    /// skin can fold as deeply as it likes without ever crossing itself.
    fn keep_order(&mut self, bi: usize, centre: (f64, f64)) {
        let spec = self.bodies[bi].spec;
        let n = spec.points;
        let b = &mut self.bodies[bi];
        let (cx, cy) = centre;
        let mut sc = 0.0;
        let mut ss = 0.0;
        let mut angle = vec![0.0f64; n];
        for i in 0..n {
            angle[i] = (b.y[i] - cy).atan2(b.x[i] - cx);
            let off = angle[i] - std::f64::consts::TAU * i as f64 / n as f64;
            sc += off.cos();
            ss += off.sin();
        }
        let turn = ss.atan2(sc);
        let limit = std::f64::consts::PI / n as f64 * spec.twist;
        for i in 0..n {
            let nominal = std::f64::consts::TAU * i as f64 / n as f64 + turn;
            let mut off = angle[i] - nominal;
            while off > std::f64::consts::PI {
                off -= std::f64::consts::TAU;
            }
            while off < -std::f64::consts::PI {
                off += std::f64::consts::TAU;
            }
            if off.abs() <= limit {
                continue;
            }
            let off = off.clamp(-limit, limit);
            let r = ((b.x[i] - cx).powi(2) + (b.y[i] - cy).powi(2)).sqrt();
            b.x[i] = cx + r * (nominal + off).cos();
            b.y[i] = cy + r * (nominal + off).sin();
        }
    }

    /// Pull two points towards the distance they would rather be apart.
    fn spring(&mut self, bi: usize, a: usize, b: usize, rest: f64, stiffness: f64) {
        let held = self.grab.filter(|(g, _)| *g == bi).map(|(_, p)| p);
        let body = &mut self.bodies[bi];
        let dx = body.x[b] - body.x[a];
        let dy = body.y[b] - body.y[a];
        let d = (dx * dx + dy * dy).sqrt();
        if d == 0.0 {
            return;
        }
        let diff = (d - rest) / d * 0.5 * self.stiffness * stiffness;
        let (mx, my) = (dx * diff, dy * diff);
        if held != Some(a) {
            body.x[a] += mx;
            body.y[a] += my;
        }
        if held != Some(b) {
            body.x[b] -= mx;
            body.y[b] -= my;
        }
    }

    fn hit_walls(&mut self, bi: usize, struck_x: &mut [bool], struck_y: &mut [bool]) {
        let (w, h) = (self.width as f64, self.height as f64);
        let b = &mut self.bodies[bi];
        for i in 0..b.x.len() {
            if b.x[i] < 0.0 {
                b.x[i] = 0.0;
                struck_x[i] = true;
            }
            if b.x[i] > w - 1.0 {
                b.x[i] = w - 1.0;
                struck_x[i] = true;
            }
            if b.y[i] < 0.0 {
                b.y[i] = 0.0;
                struck_y[i] = true;
            }
            if b.y[i] > h - 1.0 {
                b.y[i] = h - 1.0;
                struck_y[i] = true;
            }
        }
    }

    /// Down in the solid rows the letters are rock.  A point that finds itself
    /// inside one leaves by its nearest free edge: falling, that is the top, so
    /// it lands on the letter; walking, it is the side, so it is simply stopped.
    fn hit_letters(&mut self, bi: usize, struck_x: &mut [bool], struck_y: &mut [bool]) {
        if !self.ground.any() && self.grain_cells.is_empty() {
            return;
        }
        let n = self.bodies[bi].spec.points;

        for i in 0..self.bodies[bi].x.len() {
            let (x, y) = (self.bodies[bi].x[i], self.bodies[bi].y[i]);
            if let Some((dx, dy)) = self.push_out(x, y) {
                let b = &mut self.bodies[bi];
                b.x[i] += dx;
                b.y[i] += dy;
                if dy != 0.0 {
                    struck_y[i] = true;
                } else {
                    struck_x[i] = true;
                }
            }
        }

        // The skin between the points counts too, or a letter thinner than the
        // gap between two of them would pass straight through the body.
        if self.bodies[bi].spec.mode == Mode::Grain || n < 2 {
            return;
        }
        let closed = self.bodies[bi].spec.mode == Mode::Body;
        for i in 0..n {
            let j = if closed { (i + 1) % n } else { i + 1 };
            if j >= n {
                continue;
            }
            for t in 1..=SKIN_SAMPLES {
                let u = t as f64 / (SKIN_SAMPLES + 1) as f64;
                let b = &self.bodies[bi];
                let sx = b.x[i] + (b.x[j] - b.x[i]) * u;
                let sy = b.y[i] + (b.y[j] - b.y[i]) * u;
                if let Some((dx, dy)) = self.push_out(sx, sy) {
                    let b = &mut self.bodies[bi];
                    b.x[i] += dx * (1.0 - u);
                    b.y[i] += dy * (1.0 - u);
                    b.x[j] += dx * u;
                    b.y[j] += dy * u;
                    if dy != 0.0 {
                        struck_y[i] = true;
                        struck_y[j] = true;
                    } else {
                        struck_x[i] = true;
                        struck_x[j] = true;
                    }
                }
            }
        }
    }

    /// The shortest way out of the letter this point is inside, or None if it
    /// is not inside one.
    fn push_out(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let row = (y / CELL_H as f64).floor() as i32;
        let col = (x / CELL_W as f64).floor() as i32;
        if !self.blocked(row, col) {
            return None;
        }
        let (left, top) = ((col * CELL_W) as f64, (row * CELL_H) as f64);
        let shut = f64::INFINITY;

        let up = if self.blocked(row - 1, col) { shut } else { y - top + 0.02 };
        let down = if self.blocked(row + 1, col) || top + CELL_H as f64 >= self.height as f64 {
            shut
        } else {
            top + CELL_H as f64 - y + 0.02
        };
        let to_left = if self.blocked(row, col - 1) || col == 0 {
            shut
        } else {
            x - left + 0.02
        };
        let to_right =
            if self.blocked(row, col + 1) || left + CELL_W as f64 >= self.width as f64 {
                shut
            } else {
                left + CELL_W as f64 - x + 0.02
            };

        let mut best = (up, (0.0, -up));
        if to_left < best.0 {
            best = (to_left, (-to_left, 0.0));
        }
        if to_right < best.0 {
            best = (to_right, (to_right, 0.0));
        }
        if down < best.0 {
            best = (down, (0.0, down));
        }
        if best.0.is_infinite() {
            // Walled in on every side.  Buried in text, climb out upwards --
            // better a scramble than a point stuck inside a wall of letters.
            // Buried in sand, hold still: sand gets out of the way by itself,
            // and shoving every pass would launch the whole body at the sky.
            return if self.ground.is_solid(row, col) {
                Some((0.0, -(y - top + 0.02)))
            } else {
                None
            };
        }
        Some(best.1)
    }

    /// Bodies shove each other apart rather than overlap, and the smaller of
    /// the two gives way the most.
    fn separate(&mut self) {
        let n = self.bodies.len();
        for a in 0..n {
            if self.bodies[a].spec.mode != Mode::Body {
                continue;
            }
            for b in (a + 1)..n {
                if self.bodies[b].spec.mode != Mode::Body {
                    continue;
                }
                let (ax, ay) = self.bodies[a].centre();
                let (bx, by) = self.bodies[b].centre();
                let (dx, dy) = (bx - ax, by - ay);
                let d = (dx * dx + dy * dy).sqrt();
                let want = (self.bodies[a].rest + self.bodies[b].rest) * 0.92;
                if d <= 0.001 || d >= want {
                    continue;
                }
                let push = (want - d) / d * 0.5;
                let wa = self.bodies[b].rest / (self.bodies[a].rest + self.bodies[b].rest);
                let wb = 1.0 - wa;
                let (ux, uy) = (dx / d, dy / d);
                self.shove(a, -dx * push * wa, -dy * push * wa, (ux, uy));
                self.shove(b, dx * push * wb, dy * push * wb, (-ux, -uy));
                self.rub(a, b, (ux, uy), wa, wb);
            }
        }
    }

    /// Move a body out of another one's way -- but not all of it equally.
    ///
    /// The side that is actually touching takes most of the push and the far
    /// side takes hardly any, which is the difference between a shove and a
    /// turn: a box that lands on a skob with one corner is now pushed at that
    /// corner, and goes over the way a box would, instead of sliding off it
    /// still perfectly square.  `facing` points from this body towards the
    /// other one.
    fn shove(&mut self, bi: usize, dx: f64, dy: f64, facing: (f64, f64)) {
        let (cx, cy) = self.bodies[bi].centre();
        let reach = self.bodies[bi].rest.max(1.0);
        let skin = self.bodies[bi].spec.points;
        let b = &mut self.bodies[bi];
        let mut share = vec![1.0; b.x.len()];
        let mut total = 0.0;
        for i in 0..skin {
            // How far round towards the other thing this point lies: one at
            // the contact and nothing on the far side, because that is where
            // the two of them are touching and nowhere else.  A little is left
            // for the far side so the body is moved rather than pulled apart.
            let lean = ((b.x[i] - cx) * facing.0 + (b.y[i] - cy) * facing.1) / reach;
            share[i] = lean.max(0.0) + 0.12;
            total += share[i];
        }
        let mean = total / skin as f64;
        if mean < 1e-9 {
            return;
        }
        for i in 0..b.x.len() {
            // The middle is a hub, not a bit of skin: it takes the plain share
            // so that the body goes where it is sent even as the skin turns.
            let k = if i < skin { share[i] / mean } else { 1.0 };
            b.x[i] += dx * k;
            b.y[i] += dy * k;
        }
    }

    /// Nothing may step over a wall.
    ///
    /// A point is only ever looked at where it is.  Anything moving faster
    /// than a cell is wide can therefore be on one side of a block in one
    /// frame and out the far side the next, having never once been inside it
    /// for anybody to notice.  So before the skin is asked about anything, the
    /// line each point has just travelled is walked, and the point is left at
    /// the last free spot before the first solid face it would have crossed.
    fn sweep_solid(&mut self, bi: usize) {
        if !self.ground.any() && self.grain_cells.is_empty() {
            return;
        }
        for i in 0..self.bodies[bi].x.len() {
            let b = &self.bodies[bi];
            let (fx, fy) = (b.ox[i], b.oy[i]);
            let (dx, dy) = (b.x[i] - fx, b.y[i] - fy);
            // A short step cannot jump anything; the skin sees to those.
            let far = dx.abs().max(dy.abs());
            if far < 1.5 {
                continue;
            }
            let steps = far.ceil() as i32;
            let mut last = (fx, fy);
            for s in 1..=steps {
                let u = s as f64 / steps as f64;
                let (sx, sy) = (fx + dx * u, fy + dy * u);
                let (row, col) = (
                    (sy / CELL_H as f64).floor() as i32,
                    (sx / CELL_W as f64).floor() as i32,
                );
                if self.blocked(row, col) {
                    let b = &mut self.bodies[bi];
                    b.x[i] = last.0;
                    b.y[i] = last.1;
                    break;
                }
                last = (sx, sy);
            }
        }
    }

    /// Running into a wall.
    ///
    /// A letter has a whole line of other letters beside it and stops a thing
    /// by sheer numbers.  A block of wall stands on its own, and a point at a
    /// time is not enough: the two points touching it are simply dragged
    /// through by the dozen that are not.  So a block pushes on the whole body
    /// at once, out of the nearest corner or face of itself, and takes away
    /// whatever speed that body had towards it.
    fn bump_walls(&mut self) {
        let walls: Vec<(f64, f64)> = self
            .bodies
            .iter()
            .filter(|b| b.spec.fixed)
            .map(|b| (b.x[0] + CELL_W as f64 / 2.0, b.y[0] + CELL_H as f64 / 2.0))
            .collect();
        if walls.is_empty() {
            return;
        }
        let (half_w, half_h) = (CELL_W as f64 / 2.0, CELL_H as f64 / 2.0);
        for bi in 0..self.bodies.len() {
            if self.bodies[bi].spec.mode != Mode::Body {
                continue;
            }
            let keep = self.bounce * self.bodies[bi].spec.grip;
            let (cx, cy) = self.bodies[bi].centre();
            let reach = self.bodies[bi].rest * 0.92;
            // A floor is many blocks, and being pushed out of every one of
            // them in turn is how a thing gets fired off it.  The worst of
            // them is the one it is really in, and settling that settles the
            // rest: what is left over comes round again next frame.
            let mut worst = (0.0, 0.0, 0.0);
            for &(wx, wy) in &walls {
                if (wx - cx).abs() > reach + half_w * 2.0 || (wy - cy).abs() > reach + half_h * 2.0 {
                    continue;
                }
                // The nearest spot on the block itself to the middle of the body.
                let nx = cx.clamp(wx - half_w, wx + half_w);
                let ny = cy.clamp(wy - half_h, wy + half_h);
                let (mut ox, mut oy) = (cx - nx, cy - ny);
                let d = (ox * ox + oy * oy).sqrt();
                if d >= reach {
                    continue;
                }
                if d < 1e-6 {
                    // Dead centre of the block: out through the top.
                    ox = 0.0;
                    oy = -1.0;
                } else {
                    ox /= d;
                    oy /= d;
                }
                let out = reach - d;
                if out > worst.2 {
                    worst = (ox, oy, out);
                }
            }
            let (ox, oy, out) = worst;
            if out <= 0.0 {
                continue;
            }
            let b = &mut self.bodies[bi];
            for i in 0..b.x.len() {
                b.x[i] += ox * out;
                b.y[i] += oy * out;
                // Whatever of its speed was aimed at the wall is gone, bar the
                // bounce it is owed.
                let (vx, vy) = (b.x[i] - b.ox[i], b.y[i] - b.oy[i]);
                let into = vx * ox + vy * oy;
                if into < 0.0 {
                    b.ox[i] = b.x[i] - (vx - into * ox * (1.0 + keep));
                    b.oy[i] = b.y[i] - (vy - into * oy * (1.0 + keep));
                }
            }
        }
    }

    /// A wall that has ended up inside something.
    ///
    /// A body only ever feels the world with its skin, and a wall cannot move
    /// out of the way of anything, so a block that gets past the skin -- taken
    /// at a run, or dropped on -- would sit there inside the thing for good,
    /// with nothing in the world able to notice.  This is what notices: any
    /// block found in there is shown the nearest way out, and the body goes
    /// that way until it is out.  It is moved in the past as well as the
    /// present, so being unstuck is not the same as being launched.
    fn cough_up_walls(&mut self) {
        let walls: Vec<(f64, f64)> = self
            .bodies
            .iter()
            .filter(|b| b.spec.fixed)
            .map(|b| (b.x[0] + CELL_W as f64 / 2.0, b.y[0] + CELL_H as f64 / 2.0))
            .collect();
        if walls.is_empty() {
            return;
        }
        for bi in 0..self.bodies.len() {
            if self.bodies[bi].spec.mode != Mode::Body {
                continue;
            }
            let (cx, cy) = self.bodies[bi].centre();
            let reach = self.bodies[bi].rest * self.bodies[bi].spec.shell_max.max(1.0) + CELL_H as f64;
            for &(wx, wy) in &walls {
                if (wx - cx).abs() > reach || (wy - cy).abs() > reach {
                    continue;
                }
                if !self.inside_skin(bi, wx, wy) {
                    continue;
                }
                let (mut ox, mut oy) = (cx - wx, cy - wy);
                let d = (ox * ox + oy * oy).sqrt();
                if d < 1e-6 {
                    // Dead centre: up is as good a way out as any.
                    ox = 0.0;
                    oy = -1.0;
                } else {
                    ox /= d;
                    oy /= d;
                }
                let b = &mut self.bodies[bi];
                for i in 0..b.x.len() {
                    b.x[i] += ox * WALL_EJECT;
                    b.y[i] += oy * WALL_EJECT;
                    b.ox[i] += ox * WALL_EJECT;
                    b.oy[i] += oy * WALL_EJECT;
                }
                break;
            }
        }
    }

    /// Is this spot within the ring of skin?  Counted by crossings, walked
    /// once round.
    fn inside_skin(&self, bi: usize, x: f64, y: f64) -> bool {
        let b = &self.bodies[bi];
        let n = b.spec.points;
        let mut within = false;
        for i in 0..n {
            let j = (i + 1) % n;
            let (yi, yj) = (b.y[i], b.y[j]);
            if (yi > y) != (yj > y) && x < b.x[i] + (y - yi) / (yj - yi) * (b.x[j] - b.x[i]) {
                within = !within;
            }
        }
        within
    }

    /// Two things that are touching do not slide over one another for free.
    ///
    /// The bit of each that is actually in contact is slowed against the bit
    /// of the other, sideways on -- and because that is done at that one point
    /// rather than to the whole body, a box sliding off the top of a skob
    /// catches on its corner and goes over, which is what a box does.  Nothing
    /// here is allowed to speed anything up.
    fn rub(&mut self, a: usize, b: usize, normal: (f64, f64), wa: f64, wb: f64) {
        let (ax, ay) = self.bodies[a].centre();
        let (bx, by) = self.bodies[b].centre();
        let ia = match self.touching_point(a, (bx, by)) {
            Some(i) => i,
            None => return,
        };
        let ib = match self.touching_point(b, (ax, ay)) {
            Some(i) => i,
            None => return,
        };
        // Sideways to the way they are pressed together.
        let along = (-normal.1, normal.0);
        let slip = {
            let (pa, pb) = (&self.bodies[a], &self.bodies[b]);
            let va = (pa.x[ia] - pa.ox[ia], pa.y[ia] - pa.oy[ia]);
            let vb = (pb.x[ib] - pb.ox[ib], pb.y[ib] - pb.oy[ib]);
            (va.0 - vb.0) * along.0 + (va.1 - vb.1) * along.1
        };
        let bite = slip * SKIN_GRIP;
        let pa = &mut self.bodies[a];
        pa.x[ia] -= along.0 * bite * wa;
        pa.y[ia] -= along.1 * bite * wa;
        let pb = &mut self.bodies[b];
        pb.x[ib] += along.0 * bite * wb;
        pb.y[ib] += along.1 * bite * wb;
    }

    /// Which bit of this body's skin is nearest the thing it has run into.
    fn touching_point(&self, bi: usize, towards: (f64, f64)) -> Option<usize> {
        let b = &self.bodies[bi];
        (0..b.spec.points).min_by(|&i, &j| {
            let di = (b.x[i] - towards.0).powi(2) + (b.y[i] - towards.1).powi(2);
            let dj = (b.x[j] - towards.0).powi(2) + (b.y[j] - towards.1).powi(2);
            di.partial_cmp(&dj).unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    // ---------------------------------------------------------------- sand

    /// Classic falling sand: a grain drops if it can, slides off a shoulder if
    /// it cannot, and otherwise stays put.  That is the whole of it, and it is
    /// what makes a pile a pile.
    fn sandbox(&mut self) {
        let grains: Vec<usize> = (0..self.bodies.len()).filter(|&i| self.bodies[i].is_grain()).collect();
        if grains.is_empty() {
            return;
        }
        // Lowest first, so a grain never moves into a spot that is about to be
        // vacated and the pile settles from the bottom up.
        let mut order = grains.clone();
        order.sort_by(|&a, &b| {
            self.bodies[b].y[0].partial_cmp(&self.bodies[a].y[0]).unwrap_or(std::cmp::Ordering::Equal)
        });

        let bodies: Vec<(f64, f64, f64)> = self
            .bodies
            .iter()
            .filter(|b| b.spec.mode == Mode::Body)
            .map(|b| {
                // Well inside the skin.  Deeper than this a grain is plainly
                // stuck inside the thing and should get out; nearer the surface
                // it is simply sand the thing is resting on, and it stays put --
                // that is what holds a skob up on a sand bed.
                let (cx, cy) = b.centre();
                (cx, cy, b.rest * 0.72)
            })
            .collect();

        let mut taken: HashMap<(i32, i32), usize> = HashMap::with_capacity(grains.len() * 4);
        for &g in &order {
            // What is in the hand keeps its place in the world -- everything
            // else has to go round it -- but it is not snapped to the grid and
            // it does not fall while it is up there.
            if !self.in_hand(g) {
                self.snap_grain(g);
            }
            self.mark_grain(&mut taken, g, true);
        }

        for &g in &order {
            // A wall is a grain that has nothing to say about any of this, and
            // nor is one that has been picked up.  Both keep their place in
            // `taken`, so everything else has to go round them.
            if self.bodies[g].spec.fixed || self.in_hand(g) {
                continue;
            }
            let (gw, gh) = self.bodies[g].spec.mode.grain_size();
            self.mark_grain(&mut taken, g, false);
            let mut x = self.bodies[g].x[0] as i32;
            let mut y = self.bodies[g].y[0] as i32;

            // Something sat on it: find any way out at all, downwards and
            // sideways included, or it stays wedged there holding the thing up.
            if self.grain_blocked(&taken, &bodies, gw, gh, x, y) {
                for &(dx, dy) in &[
                    (0, gh),
                    (-gw, gh),
                    (gw, gh),
                    (-gw, 0),
                    (gw, 0),
                    (0, -gh),
                    (-gw, -gh),
                    (gw, -gh),
                    (0, 2 * gh),
                    (0, -2 * gh),
                ] {
                    if !self.grain_blocked(&taken, &bodies, gw, gh, x + dx, y + dy) {
                        x += dx;
                        y += dy;
                        break;
                    }
                }
            }

            // A fine grain shifts three dots a frame, a coarse one a whole row.
            let spec = self.bodies[g].spec;
            let steps = if gh == 1 { 3 } else { 1 };
            for _ in 0..steps {
                match self.spot(&taken, &bodies, gw, gh, x, y + gh) {
                    Spot::Free => {
                        y += gh;
                        continue;
                    }
                    // Sand sinks through water: the heavier of the two changes
                    // places with the lighter rather than resting on it.
                    Spot::Grain(o)
                        if self.bodies[o].spec.density < spec.density
                            && self.bodies[o].spec.mode == spec.mode =>
                    {
                        self.mark_grain(&mut taken, o, false);
                        let lighter = &mut self.bodies[o];
                        lighter.x[0] = x as f64;
                        lighter.y[0] = y as f64;
                        lighter.ox[0] = lighter.x[0];
                        lighter.oy[0] = lighter.y[0];
                        self.mark_grain(&mut taken, o, true);
                        y += gh;
                        continue;
                    }
                    _ => {}
                }
                let lean = if self.rng.float() < 0.5 { -gw } else { gw };
                if matches!(self.spot(&taken, &bodies, gw, gh, x + lean, y + gh), Spot::Free) {
                    x += lean;
                    y += gh;
                    continue;
                }
                if matches!(self.spot(&taken, &bodies, gw, gh, x - lean, y + gh), Spot::Free) {
                    x -= lean;
                    y += gh;
                    continue;
                }
                if spec.liquid {
                    // Water finds its own level, and will go sideways to do it.
                    // The direction is fixed per drop, or a pool would spend all
                    // day shivering between left and right.
                    let side = if self.bodies[g].id % 2 == 0 { gw } else { -gw };
                    if matches!(self.spot(&taken, &bodies, gw, gh, x + side, y), Spot::Free) {
                        x += side;
                        continue;
                    }
                    if matches!(self.spot(&taken, &bodies, gw, gh, x - side, y), Spot::Free) {
                        x -= side;
                        continue;
                    }
                }
                break;
            }

            let b = &mut self.bodies[g];
            b.x[0] = x as f64;
            b.y[0] = y as f64;
            b.ox[0] = b.x[0];
            b.oy[0] = b.y[0];
            self.mark_grain(&mut taken, g, true);
        }
    }

    /// A fine grain lives on the dot grid, a coarse one on the character grid.
    fn snap_grain(&mut self, g: usize) {
        let (gw, gh) = self.bodies[g].spec.mode.grain_size();
        let b = &mut self.bodies[g];
        b.x[0] = (b.x[0] as i32 / gw * gw) as f64;
        b.y[0] = (b.y[0] as i32 / gh * gh) as f64;
        b.x[0] = b.x[0].clamp(0.0, ((self.width - gw) / gw * gw) as f64);
        b.y[0] = b.y[0].clamp(0.0, ((self.height - gh) / gh * gh) as f64);
    }

    fn mark_grain(&self, taken: &mut HashMap<(i32, i32), usize>, g: usize, on: bool) {
        let (gw, gh) = self.bodies[g].spec.mode.grain_size();
        let (x, y) = (self.bodies[g].x[0] as i32, self.bodies[g].y[0] as i32);
        for i in 0..gw {
            for j in 0..gh {
                if on {
                    taken.insert((x + i, y + j), g);
                } else {
                    taken.remove(&(x + i, y + j));
                }
            }
        }
    }

    /// What one grain finds at a spot it fancies moving into.
    fn spot(
        &self,
        taken: &HashMap<(i32, i32), usize>,
        bodies: &[(f64, f64, f64)],
        gw: i32,
        gh: i32,
        x: i32,
        y: i32,
    ) -> Spot {
        if x < 0 || x + gw > self.width || y + gh > self.height {
            return Spot::Solid;
        }
        let mut other = None;
        for i in 0..gw {
            for j in 0..gh {
                if let Some(&o) = taken.get(&(x + i, y + j)) {
                    other = Some(o);
                }
                if self.ground.any()
                    && y + j >= self.ground.top_px
                    && self.ground.is_solid((y + j) / CELL_H, (x + i) / CELL_W)
                {
                    return Spot::Solid;
                }
            }
        }
        let (mx, my) = ((x + gw / 2) as f64, (y + gh / 2) as f64);
        if bodies.iter().any(|&(cx, cy, r)| {
            let (dx, dy) = (mx - cx, my - cy);
            dx * dx + dy * dy < r * r
        }) {
            return Spot::Solid;
        }
        match other {
            Some(o) => Spot::Grain(o),
            None => Spot::Free,
        }
    }

    /// Nothing may share the spot of a grain: no other grain, no letter, no body.
    fn grain_blocked(
        &self,
        taken: &HashMap<(i32, i32), usize>,
        bodies: &[(f64, f64, f64)],
        gw: i32,
        gh: i32,
        x: i32,
        y: i32,
    ) -> bool {
        if x < 0 || x + gw > self.width || y + gh > self.height {
            return true;
        }
        for i in 0..gw {
            for j in 0..gh {
                if taken.contains_key(&(x + i, y + j)) {
                    return true;
                }
                if self.ground.any() && y + j >= self.ground.top_px {
                    if self.ground.is_solid((y + j) / CELL_H, (x + i) / CELL_W) {
                        return true;
                    }
                }
            }
        }
        let (mx, my) = ((x + gw / 2) as f64, (y + gh / 2) as f64);
        bodies.iter().any(|&(cx, cy, r)| {
            let (dx, dy) = (mx - cx, my - cy);
            dx * dx + dy * dy < r * r
        })
    }

    /// The skin point nearest the given spot, for the mouse to take hold of.
    ///
    /// The whole of a thing is handle, not just its outline: anywhere inside it
    /// takes hold of the nearest piece of skin, because a click in the middle of
    /// a skob plainly means that skob.
    /// Take hold of whatever grains are under this point.
    ///
    /// A fine grain is one braille dot, and nobody can pick a single dot out
    /// of a cell with a fingertip -- so the whole cell comes up, all eight
    /// dots of it if that is what is in there.  A coarse one is a cell to
    /// itself already and comes up on its own.
    pub fn grab_grains(&mut self, x: f64, y: f64) -> usize {
        let (row, col) = (y as i32 / CELL_H, x as i32 / CELL_W);
        let (left, top) = ((col * CELL_W) as f64, (row * CELL_H) as f64);
        self.handful.clear();
        for b in &self.bodies {
            if !b.is_grain() || b.spec.fixed {
                continue;
            }
            let (gw, gh) = b.spec.mode.grain_size();
            // Whichever cell it is in, or -- for one that fills a cell -- the
            // cell it fills.
            let here = (b.x[0] as i32 / CELL_W == col && b.y[0] as i32 / CELL_H == row)
                || (b.x[0] <= x && x < b.x[0] + gw as f64 && b.y[0] <= y && y < b.y[0] + gh as f64);
            if here {
                self.handful.push((b.id, b.x[0] - left, b.y[0] - top));
            }
        }
        self.handful.len()
    }

    /// Carry whatever is in hand to where the mouse is now.
    ///
    /// Held grains are out of the sandbox while they are up: they do not fall
    /// and nothing swaps places with them.  What they are not is free to be
    /// put down inside something else -- a grain that is already somewhere
    /// keeps its place, so a handful carried at a pile stops at it rather than
    /// sinking into it.  Each grain of the handful is asked separately, so an
    /// edge of it can go where there is room while the rest waits.
    fn carry_handful(&mut self) {
        if self.handful.is_empty() {
            return;
        }
        let (mx, my) = self.mouse;
        let (left, top) = (
            (mx as i32 / CELL_W * CELL_W) as f64,
            (my as i32 / CELL_H * CELL_H) as f64,
        );

        // Everywhere that is spoken for by something not in the hand.
        let mut spoken_for: HashMap<(i32, i32), usize> = HashMap::new();
        for (bi, b) in self.bodies.iter().enumerate() {
            if !b.is_grain() || self.in_hand(bi) {
                continue;
            }
            let (gw, gh) = b.spec.mode.grain_size();
            for i in 0..gw {
                for j in 0..gh {
                    spoken_for.insert((b.x[0] as i32 + i, b.y[0] as i32 + j), bi);
                }
            }
        }

        let held = std::mem::take(&mut self.handful);
        for &(id, dx, dy) in &held {
            let bi = match self.index_of(id) {
                Some(bi) => bi,
                None => continue,
            };
            let (gw, gh) = self.bodies[bi].spec.mode.grain_size();
            let x = (left + dx).clamp(0.0, (self.width - gw) as f64);
            let y = (top + dy).clamp(0.0, (self.height - gh) as f64);
            let (col, row) = (x as i32, y as i32);
            let blocked = (0..gw).any(|i| {
                (0..gh).any(|j| {
                    spoken_for.contains_key(&(col + i, row + j))
                        || self.ground.is_solid((row + j) / CELL_H, (col + i) / CELL_W)
                })
            });
            if blocked {
                continue;   // it stays where it is until there is room
            }
            let b = &mut self.bodies[bi];
            b.x[0] = x;
            b.y[0] = y;
            b.ox[0] = b.x[0];
            b.oy[0] = b.y[0];
        }
        self.handful = held;
        self.handful.retain(|&(id, _, _)| self.bodies.iter().any(|b| b.id == id));
    }

    /// Is this grain in the hand?
    fn in_hand(&self, bi: usize) -> bool {
        let id = self.bodies[bi].id;
        self.handful.iter().any(|&(held, _, _)| held == id)
    }

    pub fn nearest_skin(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let mut best: Option<(f64, usize, usize)> = None;
        for (bi, b) in self.bodies.iter().enumerate() {
            if b.is_grain() {
                continue; // you cannot grab a grain
            }
            // Every point inside a body is within one radius of some point of
            // its skin, so this reach covers the inside and a little beyond.
            let reach = (b.rest * 1.1).powi(2).max(900.0);
            for i in 0..b.spec.points {
                let d = (b.x[i] - x).powi(2) + (b.y[i] - y).powi(2);
                if d <= reach && best.map_or(true, |(bd, _, _)| d < bd) {
                    best = Some((d, bi, i));
                }
            }
        }
        best.map(|(_, bi, i)| (bi, i))
    }
}
