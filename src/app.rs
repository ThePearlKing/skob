//! Everything the program knows, and the little language you talk to it with.

use crate::kinds::{self, Mode, Spec};
use crate::shell::Shell;
use crate::world::{World, CELL_H, CELL_W};

/// How the run was asked for on the command line.
pub struct Options {
    pub random_colour: bool,
    pub base_colour: u8,
    /// True only when -c was actually given, in which case it wins over the
    /// colours a kind would otherwise come in.
    pub colour_given: bool,
    pub eye_colour: u8,
    /// -w wants white eyes, so everything else gets out of their way.
    pub dark: bool,
    /// His height in rows, or None for a seventh of the screen.
    pub size: Option<f64>,
    /// Every one exactly that size, with no variation between them.
    pub uniform: bool,
    /// How many rows of the shell he collides with.
    pub solid: usize,
    pub shell_mode: bool,
    pub frames: usize,
    pub start_count: Option<usize>,
    pub commands: Vec<String>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            random_colour: false,
            base_colour: 84,
            colour_given: false,
            eye_colour: 232,
            dark: false,
            size: None,
            uniform: false,
            solid: 6,
            shell_mode: false,
            frames: 0,
            start_count: None,
            commands: Vec::new(),
        }
    }
}

/// Where a typed command line is in its life.
pub enum Mode2 {
    Normal,
    /// Typing after `;` or `:`.
    Command(String),
}

pub struct App {
    pub world: World,
    /// What has been typed at the `:` bar, newest last.
    pub cmd_history: Vec<String>,
    pub cmd_at: usize,
    pub cmd_saved: String,
    pub shell: Option<Shell>,
    pub opts: Options,
    pub mode: Mode2,
    pub note: String,
    /// The kind being placed by hand, how many go down per click, and how big
    /// they are if you asked for a size.
    pub placing: Option<(&'static Spec, usize, Option<f64>)>,
    /// The brush that takes things away again, and how wide it is in columns.
    pub erasing: Option<f64>,
    /// Where the brush last put something down, and how far it has to travel
    /// before it may put down another.  Forgotten the moment you let go, so
    /// every fresh click always places one.
    pub brush: Option<(f64, f64)>,
    pub brush_gap: f64,
    pub quit: bool,
    pub cols: usize,
    pub rows: usize,
}

impl App {
    pub fn new(opts: Options, cols: usize, rows: usize) -> App {
        let world = World::new(cols as i32 * CELL_W, (rows as i32 - 1) * CELL_H);
        App {
            world,
            cmd_history: Vec::new(),
            cmd_at: 0,
            cmd_saved: String::new(),
            shell: None,
            opts,
            mode: Mode2::Normal,
            note: String::new(),
            placing: None,
            erasing: None,
            brush: None,
            brush_gap: 0.0,
            quit: false,
            cols,
            rows,
        }
    }

    // ------------------------------------------------------------- colours

    /// Any of the 256 terminal colours, so long as it is actually visible.
    pub fn random_colour(&mut self) -> u8 {
        loop {
            let c = 16 + self.world.rng.below(216) as u8;
            let (r, g, b) = ((c - 16) / 36, ((c - 16) % 36) / 6, (c - 16) % 6);
            if r + g + b >= 5 {
                return self.dim(c);
            }
        }
    }

    /// Halve a colour, channel by channel, so white eyes have somewhere to sit.
    pub fn dim(&self, c: u8) -> u8 {
        if !self.opts.dark {
            return c;
        }
        match c {
            16..=231 => {
                let (r, g, b) = ((c - 16) / 36, ((c - 16) % 36) / 6, (c - 16) % 6);
                16 + (r + 1) / 2 * 36 + (g + 1) / 2 * 6 + (b + 1) / 2
            }
            232..=255 => 232 + (c - 232) / 2,
            8..=15 => c - 8,
            _ => c,
        }
    }

    /// The colour a fresh skob wears: random with -r, else the base colour.
    pub fn start_colour(&mut self) -> u8 {
        if self.opts.random_colour {
            self.random_colour()
        } else {
            self.dim(self.opts.base_colour)
        }
    }

    /// Every kind comes in its own colours.  -c overrules all of them; -r is
    /// about the skob you start with, not the ones summoned afterwards.
    pub fn colour_for(&mut self, spec: &Spec) -> u8 {
        if self.opts.colour_given || spec.palette.is_empty() {
            return self.dim(self.opts.base_colour);
        }
        let c = spec.palette[self.world.rng.below(spec.palette.len())];
        // A box is grey to begin with, and halving grey only makes coal of it.
        // It has no eyes to make room for either, so with -w it keeps its own
        // colour and stays a box you can see.
        if spec.name == "box" {
            return c;
        }
        self.dim(c)
    }

    // -------------------------------------------------------------- sizes

    /// A radius in half-cells.  --size is a height in text rows; without it he
    /// is a seventh of the screen.  No two are quite the same unless -u says so.
    pub fn radius_for(&mut self, spec: &Spec, vary: bool, asked: Option<f64>) -> f64 {
        if let Some(fixed) = spec.radius {
            return fixed;   // a grain is the size a grain is
        }
        let mut r = match asked.or(self.opts.size) {
            Some(rows) => rows * 2.0,
            None => self.world.height as f64 / 7.0,
        };
        if vary && !self.opts.uniform {
            let jitter = (r / 5.0).max(1.0);
            r += self.world.rng.float() * 2.0 * jitter - jitter;
        }
        (r * spec.scale).max(2.0)
    }

    // ------------------------------------------------------------ spawning

    pub fn reset(&mut self) {
        self.world.banish();
        let spec = kinds::lookup("skob").unwrap();
        let colour = self.start_colour();
        let radius = self.radius_for(spec, false, None);
        let (w, h) = (self.world.width as f64, self.world.height as f64);
        self.world.spawn(spec, w / 2.0, h / 3.0, radius, colour);
        self.note = "a new skob".into();
    }

    /// Drop some in at random, the way `summon` does.
    pub fn summon(&mut self, kind: &str, count: usize, size: Option<f64>) {
        let spec = match kinds::lookup(kind) {
            Some(s) => s,
            None => {
                self.note = format!("never heard of a {}", kind);
                return;
            }
        };
        // Grains come by the handful, and the fine ones by the bucket: they
        // are small, so it takes a great many of them to make a beach.
        let cap = match spec.mode {
            Mode::Grain => 4000,
            Mode::BigGrain => 800,
            _ => 60,
        };
        let count = count.clamp(1, cap);
        for _ in 0..count {
            let colour = self.colour_for(spec);
            let radius = self.radius_for(spec, true, size);
            let x = 20.0 + self.world.rng.float() * (self.world.width as f64 - 40.0).max(1.0);
            let y = 6.0 + self.world.rng.float() * (self.world.height as f64 / 3.0).max(1.0);
            self.world.spawn(spec, x, y, radius, colour);
        }
        self.note = format!("summoned {} {}", count, kinds::plural(kind, count));
    }

    /// The other end of `place`: everything under the brush, gone.
    pub fn erase_at(&mut self, px: f64, py: f64) {
        let radius = match self.erasing {
            Some(r) => r,
            None => return,
        };
        let gone = self.world.erase(px, py, radius);
        if gone > 0 {
            self.note = format!("erased {}", gone);
        }
    }

    /// A handful where you clicked, rather than wherever chance says.
    pub fn place_at(&mut self, px: f64, py: f64) {
        let (spec, count, size) = match self.placing {
            Some(p) => p,
            None => return,
        };
        let mut tied = 0usize;
        let mut put = 0usize;
        for _ in 0..count {
            let colour = self.colour_for(spec);
            let radius = self.radius_for(spec, true, size);
            let (mut x, mut y) = (px, py);
            if count > 1 {
                x += self.world.rng.float() * 12.0 - 6.0;
                y += self.world.rng.float() * 12.0 - 6.0;
            }
            x = x.clamp(1.0, self.world.width as f64 - 2.0);
            y = y.clamp(1.0, self.world.height as f64 - 2.0);
            // You draw with this, holding the button down, and a held button
            // reports itself over and over whether you move it or not.  A
            // grain may go anywhere there is not already one; anything bigger
            // has to wait until the brush has travelled clear of the last one
            // it left, so a sweep lays them out side by side and holding still
            // lays down exactly one.
            if spec.mode.is_grain() {
                if self.world.cell_taken(x, y) {
                    continue;
                }
                self.world.take_cell(x, y);
            } else {
                if let Some((lx, ly)) = self.brush {
                    if ((x - lx).powi(2) + (y - ly).powi(2)).sqrt() < self.brush_gap {
                        continue;
                    }
                }
                if spec.buoyancy >= 0.0 && self.world.body_at(x, y).is_some() {
                    continue;
                }
                self.brush = Some((x, y));
                self.brush_gap = radius * 2.0;
            }
            // Put a balloon down on top of something and it is tied to it.
            let knot = if spec.buoyancy < 0.0 { self.world.body_at(px, py) } else { None };
            let id = self.world.spawn(spec, x, y, radius, colour);
            put += 1;
            if let (Some(target), Some(i)) = (knot, self.world.index_of(id)) {
                self.world.bodies[i].tether = Some(target);
                tied += 1;
            }
        }
        if tied > 0 {
            self.note = format!("tied {} {} on", tied, kinds::plural(spec.name, tied));
        } else if put > 0 {
            self.note = format!("placed {} {}", put, kinds::plural(spec.name, put));
        }
    }

    // ------------------------------------------------------- the commands

    /// Step back and forth through what has been typed at the `:` bar.
    pub fn command_history(&mut self, typing: &str, back: bool) -> String {
        if self.cmd_history.is_empty() {
            return typing.to_string();
        }
        if back {
            if self.cmd_at == self.cmd_history.len() {
                self.cmd_saved = typing.to_string();
            }
            self.cmd_at = self.cmd_at.saturating_sub(1);
            self.cmd_history[self.cmd_at].clone()
        } else {
            if self.cmd_at >= self.cmd_history.len() {
                return typing.to_string();
            }
            self.cmd_at += 1;
            match self.cmd_history.get(self.cmd_at) {
                Some(line) => line.clone(),
                None => self.cmd_saved.clone(),
            }
        }
    }

    pub fn remember_command(&mut self, line: &str) {
        if !line.trim().is_empty() && self.cmd_history.last().map(String::as_str) != Some(line) {
            self.cmd_history.push(line.to_string());
        }
        self.cmd_at = self.cmd_history.len();
        self.cmd_saved.clear();
    }

    /// One typed line, which may be several commands: `clear ; summon amoeba 3`
    /// does both, in the order written, and stops early if one of them was
    /// `quit`.
    pub fn run_command(&mut self, line: &str) {
        for part in line.split(';') {
            if part.trim().is_empty() {
                continue;
            }
            let note = std::mem::take(&mut self.note);
            self.run_one(part.trim());
            // A command with nothing to say leaves the last thing said standing,
            // so `clear ; summon amoeba` reads as the summon it ended on.
            if self.note.is_empty() {
                self.note = note;
            }
            if self.quit {
                return;
            }
        }
    }

    /// `summon gorb 3`, `gravity 0`, `place sand 20`, and the rest of it.
    fn run_one(&mut self, line: &str) {
        let mut words = line.split_whitespace();
        let verb = words.next().unwrap_or("");
        let arg1 = words.next().unwrap_or("");
        let arg2 = words.next().unwrap_or("");
        let arg3 = words.next().unwrap_or("");

        // `summon gorb 3 10` is three gorbs ten rows tall.  `summon 3` and
        // `summon gorb` still mean what they always did.
        let (kind, count, size) = match arg1.parse::<usize>() {
            Ok(n) => ("skob".to_string(), n, arg2.parse::<f64>().ok()),
            Err(_) if !arg1.is_empty() => (
                arg1.to_string(),
                arg2.parse::<usize>().unwrap_or(1),
                arg3.parse::<f64>().ok(),
            ),
            Err(_) => ("skob".to_string(), 1, None),
        };
        let size = size.filter(|s| *s > 0.0 && *s < 1000.0);

        match verb {
            "" => {}
            "summon" => self.summon(&kind, count, size),
            "place" => match kinds::lookup(&kind) {
                Some(spec) => {
                    self.placing = Some((spec, count.max(1), size));
                    self.erasing = None;
                    self.note = match size {
                        Some(z) => format!("click to place {}, {} rows tall", spec.name, z),
                        None => format!("click to place {}", spec.name),
                    };
                }
                None => self.note = format!("never heard of a {}", kind),
            },
            // `:erase` is `:place` backwards: a brush, as wide as you say.
            "erase" => {
                let radius = arg1.parse::<f64>().unwrap_or(4.0).clamp(1.0, 200.0);
                self.erasing = Some(radius);
                self.placing = None;
                self.note = format!("click to erase, {} across", radius * 2.0);
            }
            "stop" => {
                self.note = match (self.placing.take(), self.erasing.take()) {
                    (Some((spec, _, _)), _) => format!("stopped placing {}", spec.name),
                    (None, Some(_)) => "stopped erasing".into(),
                    (None, None) => "not placing anything".into(),
                };
            }
            // Everything, all of it: the things, and whatever you were in the
            // middle of doing to them.
            "clear" => {
                self.world.banish();
                self.placing = None;
                self.erasing = None;
                self.note = "all of it, gone".into();
            }
            "reset" => self.reset(),
            "gravity" => {
                self.world.gravity = arg1.parse().unwrap_or(0.32);
                self.note = format!("gravity {}", self.world.gravity);
            }
            "stiffness" => {
                self.world.stiffness = arg1.parse().unwrap_or(0.55);
                self.note = format!("stiffness {}", self.world.stiffness);
            }
            "bounce" => {
                self.world.bounce = arg1.parse().unwrap_or(0.45);
                self.note = format!("bounce {}", self.world.bounce);
            }
            "colour" | "color" => {
                let c: u8 = arg1.parse().unwrap_or(self.opts.base_colour);
                for b in &mut self.world.bodies {
                    b.colour = c;
                }
                self.note = "recoloured".into();
            }
            "random" => {
                self.opts.random_colour = true;
                for i in 0..self.world.bodies.len() {
                    let c = self.random_colour();
                    self.world.bodies[i].colour = c;
                }
                self.note = "recoloured at random".into();
            }
            "pause" => {
                self.world.paused = !self.world.paused;
                self.note = if self.world.paused { "held" } else { "squishing" }.into();
            }
            // The flags work in here too, and -h puts the whole help where you
            // can read it: down the scrollback, not squeezed onto one line.
            "-h" | "--help" => {
                let lines: Vec<String> =
                    crate::USAGE.lines().map(|l| format!("  {}", l)).collect();
                match &mut self.shell {
                    Some(sh) => {
                        for line in lines {
                            sh.note(&line);
                        }
                    }
                    None => self.run_command("help"),
                }
            }
            "-n" | "--count" | "--skobs" => self.summon("skob", count.max(1), size),
            "-r" | "--random" => self.run_command("random"),
            "-c" | "--colour" | "--color" => {
                let c = arg1.to_string();
                self.run_command(&format!("colour {}", c));
            }
            "-z" | "--size" => {
                self.opts.size = arg1.parse().ok();
                self.note = format!("size {}", arg1);
            }
            "-u" | "--uniform" => {
                self.opts.uniform = !self.opts.uniform;
                self.note = if self.opts.uniform {
                    "every one the same size"
                } else {
                    "sizes vary again"
                }
                .into();
            }
            "-s" | "--solid" => {
                self.opts.solid = arg1.parse().unwrap_or(6).max(2);
                self.note = format!("{} solid rows", self.opts.solid);
            }
            "-w" | "--white-eyes" => {
                self.opts.dark = !self.opts.dark;
                self.opts.eye_colour = if self.opts.dark { 231 } else { 232 };
                self.note = if self.opts.dark { "white eyes" } else { "dark eyes" }.into();
            }
            "help" => {
                self.note = format!(
                    "summon|place {} [n] [size] · erase [r] · stop · clear · reset · gravity x · stiffness x · bounce x · colour n · random · pause · quit",
                    kinds::names()
                );
            }
            "q" | "quit" | "exit" | "bye" => self.quit = true,
            other => self.note = format!("not a command: {}", other),
        }
    }

    /// The colour of the first thing alive, which the prompt borrows.
    pub fn accent(&self) -> u8 {
        self.world.bodies.first().map(|b| b.colour).unwrap_or(self.opts.base_colour)
    }

    /// How many things there are, and whether they are all plain skobs.
    pub fn census(&self) -> (usize, bool) {
        let all_skobs = self.world.bodies.iter().all(|b| b.spec.name == "skob");
        (self.world.bodies.len(), all_skobs)
    }

    /// The field has to follow the window when it changes size.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        self.world.width = cols as i32 * CELL_W;
        self.world.height = (rows as i32 - 1) * CELL_H;
    }
}
