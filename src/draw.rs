//! Turning the field into characters.
//!
//! The field is four times taller and twice wider than the screen, because one
//! braille cell holds a 2x4 grid of dots.  Everything is painted into a canvas
//! of cells first, in a settled order, so that two things overlapping the same
//! cell always resolve the same way and nothing flickers between frames.

use crate::kinds::Mode;
use crate::world::{World, CELL_H, CELL_W};
use std::collections::HashMap;

/// Balloon string: pale enough to read as thread rather than as a thing.
const STRING_COLOUR: u8 = 246;

/// Which bit of a braille cell each dot of the 2x4 grid is.
const DOT: [u8; 8] = [1, 2, 4, 64, 8, 16, 32, 128];

#[derive(Clone, Copy)]
pub struct Cell {
    pub dots: u8,
    pub colour: u8,
    /// Set when the thing draws itself as a letter rather than as dots.
    pub glyph: Option<char>,
}

pub struct Face {
    pub col: i32,
    pub row: i32,
    pub eyes: bool,
    pub radius: f64,
}

pub struct Canvas {
    pub cells: HashMap<(i32, i32), Cell>,
    /// Where each thing with a face is, and how big it is, so its eyes can be
    /// set as far apart as the face they are in.
    pub faces: Vec<Face>,
}

impl Canvas {
    pub fn paint(world: &World) -> Canvas {
        let mut canvas = Canvas { cells: HashMap::with_capacity(512), faces: Vec::new() };
        // Painted oldest first: the draw order never depends on iteration luck,
        // so an overlap keeps one colour instead of fighting for it every frame.
        for (bi, body) in world.bodies.iter().enumerate() {
            match body.spec.mode {
                Mode::Grain => {
                    canvas.dot(body.x[0] as i32, body.y[0] as i32, body.colour);
                }
                Mode::BigGrain => {
                    let col = body.x[0] as i32 / CELL_W;
                    let row = body.y[0] as i32 / CELL_H;
                    canvas.letter(row, col, body.colour, body.spec.glyph.unwrap_or('#'));
                }
                Mode::Chain => {
                    for i in 0..body.spec.points.saturating_sub(1) {
                        canvas.line(
                            body.x[i],
                            body.y[i],
                            body.x[i + 1],
                            body.y[i + 1],
                            body.colour,
                        );
                    }
                }
                Mode::Body => {
                    // The string first, so the balloon itself sits over the knot.
                    if body.spec.buoyancy < 0.0 {
                        let ((kx, ky), (ax, ay)) = world.string_ends(bi);
                        canvas.line(kx, ky, ax, ay, STRING_COLOUR);
                    }
                    let (top, bottom) = canvas.fill_ring(body, world);
                    if body.spec.shine {
                        canvas.glint(body);
                    }
                    let (cx, cy) = body.centre();
                    canvas.faces.push(Face {
                        col: cx as i32 / CELL_W,
                        // Kept inside the face itself, which for something only
                        // a row or two tall is not where its middle rounds to.
                        row: (cy as i32 / CELL_H).clamp(top, bottom),
                        eyes: body.spec.eyes,
                        radius: body.rest,
                    });
                }
            }
        }
        canvas
    }

    fn dot(&mut self, px: i32, py: i32, colour: u8) {
        if px < 0 || py < 0 {
            return;
        }
        let key = (py / CELL_H, px / CELL_W);
        let bit = DOT[((px % CELL_W) * CELL_H + py % CELL_H) as usize];
        let cell = self.cells.entry(key).or_insert(Cell { dots: 0, colour, glyph: None });
        cell.dots |= bit;
        cell.colour = colour;
    }

    fn letter(&mut self, row: i32, col: i32, colour: u8, glyph: char) {
        if row < 0 || col < 0 {
            return;
        }
        self.cells.insert((row, col), Cell { dots: 0, colour, glyph: Some(glyph) });
    }

    /// The highlight on something polished: a curved sweep up in the top left,
    /// drawn a few shades lighter than the thing itself and sized to it, so a
    /// big gorb gets a big smear of light and a small one gets a small one.
    fn glint(&mut self, body: &crate::world::Body) {
        let (cx, cy) = body.centre();
        let radius = body.rest;
        let colour = lighten(body.colour);
        let (sx, sy) = body.spec.squash;
        let arc = radius * 0.62;
        // Up and to the left, which is where the light is, by convention.
        let centre = -2.25;
        let span = 1.05;
        let steps = ((arc * span * 2.0).ceil() as usize).max(6);
        let thickness = ((radius / 7.0).round() as i32).clamp(1, 3);
        for i in 0..=steps {
            let a = centre - span / 2.0 + span * i as f64 / steps as f64;
            for t in 0..thickness {
                let r = arc - t as f64;
                self.dot((cx + r * a.cos() * sx) as i32, (cy + r * a.sin() * sy) as i32, colour);
            }
        }
    }

    fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, colour: u8) {
        let steps = ((x1 - x0).abs() + (y1 - y0).abs()).round().max(1.0) as i32;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            self.dot((x0 + (x1 - x0) * t) as i32, (y0 + (y1 - y0) * t) as i32, colour);
        }
    }

    /// Scanline fill of the ring, one field row at a time.
    ///
    /// A balloon is drawn through a squash, so it reads as an ellipse on a
    /// string while its physics stays the plain circle everything else uses.
    fn fill_ring(&mut self, body: &crate::world::Body, world: &World) -> (i32, i32) {
        let n = body.spec.points;
        let (cx, cy) = body.centre();
        let (sx, sy_) = body.spec.squash;
        let px: Vec<f64> = (0..n).map(|i| cx + (body.x[i] - cx) * sx).collect();
        let py: Vec<f64> = (0..n).map(|i| cy + (body.y[i] - cy) * sy_).collect();

        let top = py.iter().cloned().fold(f64::MAX, f64::min) as i32;
        let bottom = py.iter().cloned().fold(f64::MIN, f64::max) as i32;
        let mut crossings: Vec<f64> = Vec::with_capacity(8);

        for sy in top.max(0)..=bottom.min(world.height - 1) {
            crossings.clear();
            let y = sy as f64;
            for i in 0..n {
                let j = (i + 1) % n;
                let (yi, yj) = (py[i], py[j]);
                if (yi <= y && yj > y) || (yj <= y && yi > y) {
                    crossings.push(px[i] + (y - yi) / (yj - yi) * (px[j] - px[i]));
                }
            }
            crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in crossings.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                for px in pair[0] as i32..=pair[1] as i32 {
                    self.dot(px, sy, body.colour);
                }
            }
        }
        (top.max(0) / CELL_H, bottom.min(world.height - 1) / CELL_H)
    }
}

/// The same colour, a few steps lighter: a highlight is the thing's own colour
/// with more light on it, never a smear of white.
pub fn lighten(c: u8) -> u8 {
    match c {
        16..=231 => {
            let (r, g, b) = ((c - 16) / 36, ((c - 16) % 36) / 6, (c - 16) % 6);
            16 + (r + 2).min(5) * 36 + (g + 2).min(5) * 6 + (b + 2).min(5)
        }
        232..=249 => c + 6,
        250..=255 => 255,
        0..=7 => c + 8,
        _ => c,
    }
}

/// The braille character for a dot pattern.
pub fn braille(dots: u8) -> char {
    char::from_u32(0x2800 + dots as u32).unwrap_or(' ')
}
