//! skob.
//!
//! A skob is a soft thing: a ring of point masses held together by springs,
//! with spokes to its middle to keep it plump.  Verlet integrated -- a skob has
//! no stored velocity, only the gap between where it is and where it just was.

mod app;
mod draw;
mod kinds;
mod shell;
mod term;
mod world;

use app::{App, Mode2, Options};
use draw::{braille, Canvas};
use shell::{strip_colour, Action, Shell};
use std::time::{Duration, Instant};
use term::{Screen, Term};
use world::{Ground, CELL_H, CELL_W};

const TICK: Duration = Duration::from_millis(55);
/// A burst of output shoves him up a few rows a frame, not all at once.
const MAX_SHOVE_ROWS: usize = 3;

pub const USAGE: &str = "\
 s k o b

 A skob is a soft thing: a ring of point masses held together by springs,
 with spokes to its middle to keep it plump.  Verlet integrated -- a skob
 has no stored velocity, only the gap between where it is and where it
 just was.  Fast enough to keep a few thousand grains of sand in the air
 at once, and to let a skob swim in them.

 FLAGS
   -r, --random         start with random colours
   -n, --count <n>      how many skobs to spawn at the start
   -c, --colour <n>     start with this 256-colour index
       --shell          live in a bash shell (see below)
   -s, --solid <n>      how many shell rows he collides with (default 6)
   -z, --size <n>       how many rows tall he is (default: a seventh)
   -w, --white-eyes     white eyes, and darker skins for them to sit in
   -u, --uniform        every skob exactly --size, no variation
       --command <cmd>  run a command at startup, as often as you like
       --frames <n>     run n frames and quit (for recording)
   -h, --help           this

 MOUSE  press and drag -> grab the skin of a thing and fling it
 KEYS   space pause · g gravity · b bouncier · s softer
        h harder · r reset · q quit
 ;  or  :   open the command line, vim style

 THINGS   skob        a soft ball, the original
          gorb        an orange sphere, rigid and shining, no two alike
          box         four corners, no give at all, and it sinks
          amoeba      a bag of fluid: holds its area, not its shape, and
                      crawls about on pseudopods
          string      an open chain, a rope with no inside
          sand        one grain, and it piles the way sand does
          bigsand     a coarser grain, written #
          wall        a block of stone that stays where you put it
          water       finds its own level, and can be swum in
          bigwater    a coarser drop, written ~
          balloon     small and vibrant; three of them lift a skob
          bigballoon  round, and takes a skob away on its own

 COMMANDS
   :summon <thing> [n] [size]  n of them, dropped in at random, that many
                               rows tall
   :place  <thing> [n] [size]  then draw with the mouse -- hold the button
                               down and sweep -- until :stop.  Nothing is
                               put down inside anything already there.
   :erase  [r]          then click or drag to rub things out, r columns across
                        (4 by default), until :stop
   :stop                stop placing or erasing
   :clear :reset        banish everything · back to one skob
   :gravity <x>  :stiffness <x>  :bounce <x>  :colour <n>  :random  :pause
   :q  :quit  :exit     leave
   a ; b ; c            several at once, in the order written:
                        :clear ; summon amoeba 3 ; gravity 0.1

 --shell
   The whole screen becomes a bash shell, scrolling upward the way a
   terminal does.  In the bottom 6 rows (-s) the text itself is the
   ground: things land on the letters and fall through the gaps between
   the words.  Everything above is scrollback they drift through, and any
   text they are on top of is redrawn in their own colour.  When new
   output arrives the buffer scrolls, and the rising lines shove up only
   the things that actually have text underneath them.

   SHELL KEYS  enter run · tab complete · up/down history
               left/right & ctrl-arrow move · ctrl-a/e ends
               ctrl-u/k/w kill · ctrl-l clear · ctrl-c abort
               ctrl-d leave.  Long jobs run in the background, so the
               skobs keep squishing while they work.
";

fn main() {
    let opts = match parse_args() {
        Some(o) => o,
        None => {
            print!("{}", USAGE);
            return;
        }
    };
    let (cols, rows) = term::size();
    let mut app = App::new(opts, cols, rows);

    app.reset();
    if let Some(n) = app.opts.start_count {
        app.world.banish();
        app.summon("skob", n, None);
    }
    if app.opts.shell_mode {
        let accent = app.accent();
        let mut sh = Shell::new(accent);
        let first = app.world.bodies.first().map(|b| b.colour).unwrap_or(84);
        sh.print(
            "  skob is loose in your shell.",
            &format!("\x1b[1;38;5;{}m  skob is loose in your shell.\x1b[0m", first),
        );
        sh.note("  the letters in the bottom rows are solid; he falls through the gaps between them.");
        sh.note("  `skob help` for his commands, drag him with the mouse, ctrl-d to leave.");
        app.shell = Some(sh);
    }
    // Whatever you asked for on the way in.
    for cmd in app.opts.commands.clone() {
        app.run_command(&cmd);
    }
    app.note.clear();

    let mut term = Term::new();
    // However badly this goes, the terminal is not left in raw mode with mouse
    // reporting on: that is what leaves you typing `;38;36;1M` at your shell.
    std::panic::set_hook(Box::new(|info| {
        term::emergency_restore();
        eprintln!("skob fell over: {}", info);
    }));
    term.raw_mode();
    term.enter_screen(app.opts.shell_mode);

    run(&mut app, &mut term);

    // Mouse reporting off, then a moment for anything already on its way, then
    // throw that away before the shell we came from can read it as typing.
    term.leave_screen();
    std::thread::sleep(Duration::from_millis(30));
    term.drop_pending_input();
    term.cooked_mode();
}

fn run(app: &mut App, term: &mut Term) {
    let mut screen = Screen::new();
    let mut frame = 0usize;
    let mut next = Instant::now() + TICK;
    // Rows of scrollback that are pending a shove, paid off a few per frame.
    let mut owed_rows = 0usize;

    loop {
        let (cols, rows) = term::size();
        if cols != app.cols || rows != app.rows {
            app.resize(cols, rows);
        }

        // ------------------------------------------------- the shell's turn
        let mut text_rows: Vec<(i32, String, String)> = Vec::new(); // row, plain, coloured
        let mut top_text = rows;
        if let Some(sh) = &mut app.shell {
            sh.width = cols;
            if sh.job.is_some() {
                sh.drain_job();
                sh.reap_job();
            }
            let visible = (rows - 1).min(sh.scrollback.len());
            top_text = rows - visible;
            for i in 0..visible {
                let (plain, coloured) = &sh.scrollback[sh.scrollback.len() - visible + i];
                text_rows.push(((top_text + i) as i32, plain.clone(), coloured.clone()));
            }
            owed_rows += sh.fresh;
            sh.fresh = 0;
            owed_rows = owed_rows.min(200); // a flood shoves him up, it does not bury him
        }

        // The bottom rows of text are the only ones that are solid ground.
        if app.opts.shell_mode {
            let first_solid = (rows as i32 - app.opts.solid as i32).max(1);
            let lines: Vec<(i32, String)> = text_rows
                .iter()
                .filter(|(row, _, _)| *row >= first_solid)
                .map(|(row, plain, _)| (row - 1, plain.clone()))
                .collect();
            app.world.ground =
                Ground::from_text(first_solid - 1, cols as i32, rows as i32, &lines);
        }

        let shove = owed_rows.min(MAX_SHOVE_ROWS);
        owed_rows -= shove;
        app.world.shove_up((shove * CELL_H as usize) as f64, ((top_text as i32 - 1) * CELL_H) as f64);

        app.world.step();
        render(app, &mut screen, &text_rows, top_text, cols, rows);
        screen.flush();

        frame += 1;
        if app.opts.frames > 0 && frame >= app.opts.frames {
            return;
        }

        // ------------------------------------------------------- the keys
        if next < Instant::now() {
            next = Instant::now() + TICK;
        }
        while let Some(byte) = term.key(next.saturating_duration_since(Instant::now())) {
            handle_byte(app, term, byte);
            if app.quit {
                return;
            }
            if Instant::now() >= next {
                break;
            }
        }
        next += TICK;
    }
}

// ---------------------------------------------------------------- the input

/// One byte from the keyboard, plus whatever else it turns out to need.
fn handle_byte(app: &mut App, term: &mut Term, byte: u8) {
    if byte == 0x1b {
        return handle_escape(app, term);
    }
    if app.shell.is_some() {
        let action = app.shell.as_mut().unwrap().key(byte);
        return finish_shell_action(app, term, action);
    }
    match &mut app.mode {
        Mode2::Command(buf) => match byte {
            3 => app.mode = Mode2::Normal,        // ^C: never mind
            b'\r' | b'\n' => {
                let cmd = buf.clone();
                app.mode = Mode2::Normal;
                app.remember_command(&cmd);
                app.run_command(&cmd);
            }
            127 | 8 => {
                buf.pop();
            }
            b if b >= 32 => buf.push(b as char),
            _ => {}
        },
        Mode2::Normal => match byte {
            b';' | b':' => app.mode = Mode2::Command(String::new()),
            b'q' | 3 | 4 => app.quit = true,      // q, ^C or ^D all mean leave
            b' ' => app.world.paused = !app.world.paused,
            b'g' => app.world.gravity = if app.world.gravity > 0.0 { 0.0 } else { 0.32 },
            b'b' => app.world.bounce = (app.world.bounce + 0.15).min(0.95),
            b's' => app.world.stiffness = (app.world.stiffness - 0.12).max(0.22),
            b'h' => app.world.stiffness = (app.world.stiffness + 0.12).min(0.95),
            b'r' => app.reset(),
            _ => {}
        },
    }
}

/// An escape sequence is read whole or not at all.  A mouse report that is cut
/// short would otherwise leave its tail behind to be typed into your line.
fn handle_escape(app: &mut App, term: &mut Term) {
    let intro = match term.key(Duration::from_millis(60)) {
        Some(b) => b,
        None => {
            // A bare escape: back out of the command line.
            if let Mode2::Command(_) = app.mode {
                app.mode = Mode2::Normal;
            }
            return;
        }
    };
    let mut seq = String::new();
    match intro {
        b'O' => {
            if let Some(b) = term.key(Duration::from_millis(60)) {
                seq.push(b as char);
            }
        }
        b'[' => loop {
            match term.key(Duration::from_millis(120)) {
                Some(b) => {
                    let c = b as char;
                    seq.push(c);
                    // A mouse report only ever ends at M or m; everything else
                    // ends at the first letter or tilde.
                    let done = if seq.starts_with('<') {
                        c == 'M' || c == 'm'
                    } else {
                        c.is_ascii_alphabetic() || c == '~'
                    };
                    if done || seq.len() > 32 {
                        break;
                    }
                }
                None => return, // incomplete: drop it rather than type it
            }
        },
        _ => return,
    }

    if let Some(report) = parse_mouse(&seq) {
        return handle_mouse(app, report);
    }
    // The `:` bar has a history of its own: up and down walk it.
    if let Mode2::Command(typing) = &app.mode {
        let typed = typing.clone();
        let line = match seq.as_str() {
            "A" => app.command_history(&typed, true),
            "B" => app.command_history(&typed, false),
            _ => return,
        };
        app.mode = Mode2::Command(line);
        return;
    }
    if let Some(sh) = &mut app.shell {
        sh.escape_key(&seq);
    }
}

/// button, column, row, pressed
fn parse_mouse(seq: &str) -> Option<(u32, i32, i32, bool)> {
    let body = seq.strip_prefix('<')?;
    let pressed = body.ends_with('M');
    let body = &body[..body.len() - 1];
    let mut parts = body.split(';');
    let button: u32 = parts.next()?.parse().ok()?;
    let col: i32 = parts.next()?.parse().ok()?;
    let row: i32 = parts.next()?.parse().ok()?;
    Some((button, col, row, pressed))
}

fn handle_mouse(app: &mut App, (button, col, row, pressed): (u32, i32, i32, bool)) {
    // Cell coordinates are one-based; field pixels are not.
    let px = ((col - 1) * CELL_W) as f64;
    let py = ((row - 1) * CELL_H) as f64;
    app.world.mouse = (px, py);

    // Erasing is a brush: it works while you drag, not only where you click.
    if pressed && (button == 0 || button == 32) && app.erasing.is_some() {
        app.erase_at(px, py);
        return;
    }
    // Placing is a brush too: hold the button down and draw with it.
    if pressed && (button == 0 || button == 32) && app.placing.is_some() {
        app.place_at(px, py);
        return;
    }
    if pressed && button == 0 {
        app.world.grab = app.world.nearest_skin(px, py);
        if app.world.grab.is_some() {
            app.note = "held".into();
        }
    } else if !pressed {
        app.world.grab = None;
        // Let go and the brush forgets where it was, so the next click always
        // puts one down wherever you clicked.
        app.brush = None;
    }
}

fn finish_shell_action(app: &mut App, term: &mut Term, action: Action) {
    match action {
        Action::Nothing => {}
        Action::Leave => app.quit = true,
        Action::Creature(cmd) => {
            app.run_command(&cmd);
            let note = std::mem::take(&mut app.note);
            if !note.is_empty() {
                let colour = app.accent();
                if let Some(sh) = &mut app.shell {
                    sh.print(
                        &format!("  {}", note),
                        &format!("\x1b[38;5;{}m  {}\x1b[0m", colour, note),
                    );
                }
            }
        }
        Action::Foreground(line) => {
            // Stand aside: the terminal belongs to that command until it is done.
            term.cooked_mode();
            term.leave_screen();
            if let Some(sh) = &mut app.shell {
                sh.run_foreground(&line);
            }
            term.raw_mode();
            // Something like `neofetch` says its piece and is gone in an
            // instant, and taking the screen straight back would be the same
            // as never having run it.  So the last word stays up until there
            // is a key to say it has been read.
            wait_for_a_key(term);
            // Whatever it left behind in the input queue on its way out -- a
            // half-finished escape sequence, a mouse report -- is its own, not
            // ours, and would be typed into the world as gibberish.
            term.drop_pending_input();
            term.enter_screen(true);
        }
    }
}

/// Hold whatever is on the screen until a key says it has been seen.  There is
/// a limit on the holding: a recording has nobody to press anything, and a
/// terminal that has ended cannot be waited on at all.
fn wait_for_a_key(term: &mut Term) {
    if term.at_eof() {
        return;
    }
    print!("\r\n\x1b[2m  -- any key --\x1b[0m");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let until = Instant::now() + Duration::from_secs(120);
    while Instant::now() < until {
        if term.key(Duration::from_millis(80)).is_some() || term.at_eof() {
            break;
        }
    }
    term.drop_pending_input();
}

// ------------------------------------------------------------------ drawing

fn render(
    app: &mut App,
    screen: &mut Screen,
    text_rows: &[(i32, String, String)],
    _top_text: usize,
    cols: usize,
    rows: usize,
) {
    screen.begin(cols, rows);

    // The scrollback he haunts, each line written end to end.
    for (row, _, coloured) in text_rows {
        screen.row(*row as usize, coloured);
    }

    let plain_at: std::collections::HashMap<i32, &String> =
        text_rows.iter().map(|(row, plain, _)| (*row, plain)).collect();
    let first_solid = rows as i32 - app.opts.solid as i32;

    let canvas = Canvas::paint(&app.world);
    for (&(cell_row, cell_col), cell) in &canvas.cells {
        let (row, col) = (cell_row + 1, cell_col + 1);
        if row < 1 || col < 1 || row > rows as i32 || col > cols as i32 {
            continue;
        }
        if app.opts.shell_mode {
            if let Some(text) = plain_at.get(&row) {
                if let Some(ch) = text.chars().nth(cell_col as usize) {
                    if ch != ' ' {
                        // Down in the solid rows a letter is rock: it keeps its
                        // own colour and he stays behind it.  Higher up he is a
                        // ghost, and wears the text he is standing on.
                        if row >= first_solid {
                            continue;
                        }
                        screen.put(row as usize, col as usize, cell.colour, true, ch);
                        continue;
                    }
                }
            }
        }
        match cell.glyph {
            Some(glyph) => screen.put(row as usize, col as usize, cell.colour, true, glyph),
            None => screen.put(row as usize, col as usize, cell.colour, false, braille(cell.dots)),
        }
    }

    // Faces are never covered.
    let (mouse_col, mouse_row) = (
        app.world.mouse.0 as i32 / CELL_W + 1,
        app.world.mouse.1 as i32 / CELL_H + 1,
    );
    for face in &canvas.faces {
        let (row, col) = (face.row + 1, face.col + 1);
        if !face.eyes {
            continue;
        }
        let (mut dx, mut dy) = (0, 0);
        if mouse_col > 0 {
            dx = (mouse_col - col).signum();
            dy = (mouse_row - row).signum();
        }
        // Set as wide apart as there is face to set them in, so a skob the size
        // of a pea does not wear its eyes on stalks.
        let spread = ((face.radius / 9.0).round() as i32).clamp(0, 3);
        for eye_col in [col - spread + dx, col + spread + 1 + dx] {
            if eye_col >= 1 && eye_col <= cols as i32 && row + dy >= 1 && row + dy <= rows as i32 {
                screen.put((row + dy) as usize, eye_col as usize, app.opts.eye_colour, true, '◉');
            }
        }
    }

    // You cannot miss what you are doing.
    let doing = match (app.placing, app.erasing) {
        (Some((spec, _, _)), _) => Some((format!("PLACING {}", spec.name), 220)),
        (None, Some(r)) => Some((format!("ERASING · {} columns across", r * 2.0), 203)),
        (None, None) => None,
    };
    if let Some((what, colour)) = doing {
        let how = if app.opts.shell_mode { "`skob stop`" } else { ":stop" };
        let banner = format!(" {} · {} TO STOP ", what, how);
        let text = format!("\x1b[0;1;7;38;5;{}m{}\x1b[0m", colour, cut(&banner, cols - 1));
        screen.row(1, &text);
    }

    // The bottom row: the prompt, the command line, or the tally.
    if app.opts.shell_mode {
        let accent = app.accent();
        let running = app.shell.as_ref().map_or(false, |s| s.job.is_some());
        let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];
        let sh = app.shell.as_mut().unwrap();
        sh.accent = accent;
        if running {
            let tick = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() / 120)
                .unwrap_or(0)
                % 8) as usize;
            let line = format!(
                "\x1b[0;38;5;220m{}\x1b[0m \x1b[38;5;244mrunning · ^C stops it\x1b[0m",
                spinner[tick]
            );
            screen.row(rows, &line);
            screen.cursor_at(rows, 1);
        } else {
            let (plain, coloured) = sh.prompt();
            let room = cols.saturating_sub(plain.chars().count() + 1).max(10);
            let offset = sh.cursor.saturating_sub(room);
            let visible: String = sh.line.chars().skip(offset).take(room).collect();
            let highlighted = sh.highlight(&visible);
            let line = format!("{}{}", coloured, highlighted);
            screen.row(rows, &line);
            let col = plain.chars().count() + (sh.cursor - offset) + 1;
            screen.cursor_at(rows, col);
        }
    } else if let Mode2::Command(buf) = &app.mode {
        let line = format!("\x1b[0;38;5;220m:{}\x1b[7m \x1b[0m", buf);
        screen.row(rows, &line);
    } else {
        let (count, all_skobs) = app.census();
        let word = if all_skobs { "skob" } else { "thing" };
        let plural = if count == 1 { "" } else { "s" };
        let held = if app.world.paused { "held" } else { "squishing" };
        let mut line = format!(
            " {} {}{} · gravity {} · stiffness {} · {}",
            count, word, plural, app.world.gravity, app.world.stiffness, held
        );
        if !app.note.is_empty() {
            line.push_str(&format!(" · {}", app.note));
        }
        line.push_str(" · ; for commands ");
        let text = format!("\x1b[0;7m{}\x1b[0m", cut(&line, cols - 1));
        screen.row(rows, &text);
    }
    screen.end();
}

fn cut(s: &str, width: usize) -> String {
    s.chars().take(width).collect()
}

// ------------------------------------------------------------------- flags

fn parse_args() -> Option<Options> {
    let mut opts = Options::default();
    // -rn 6 and -rn6 mean the same as -r -n 6, the way every other tool does.
    let mut args: Vec<String> = Vec::new();
    for arg in std::env::args().skip(1) {
        if arg.starts_with("--") || !arg.starts_with('-') || arg.len() <= 2 {
            args.push(arg);
            continue;
        }
        if arg[1..].chars().next().map_or(false, |c| c.is_ascii_digit()) {
            args.push(arg);
            continue;
        }
        let mut rest = arg[1..].to_string();
        while !rest.is_empty() {
            let c = rest.remove(0);
            args.push(format!("-{}", c));
            if "nczs".contains(c) && !rest.is_empty() {
                args.push(std::mem::take(&mut rest));
            }
        }
    }

    let mut i = 0;
    let next = |i: &mut usize, args: &Vec<String>| -> String {
        *i += 1;
        args.get(*i).cloned().unwrap_or_default()
    };
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return None,
            "-r" | "--random" => opts.random_colour = true,
            "-n" | "--count" | "--skobs" => {
                opts.start_count = next(&mut i, &args).parse().ok()
            }
            "-c" | "--colour" | "--color" => {
                opts.base_colour = next(&mut i, &args).parse().unwrap_or(84);
                opts.colour_given = true;
            }
            "-s" | "--solid" => opts.solid = next(&mut i, &args).parse().unwrap_or(6).max(2),
            "-z" | "--size" => opts.size = next(&mut i, &args).parse().ok(),
            "-w" | "--white-eyes" => {
                opts.eye_colour = 231;
                opts.dark = true;
            }
            "-u" | "--uniform" | "--no-variation" => opts.uniform = true,
            "--shell" => opts.shell_mode = true,
            "--command" | "--cmd" => opts.commands.push(next(&mut i, &args)),
            "--frames" => opts.frames = next(&mut i, &args).parse().unwrap_or(0),
            other if other.starts_with('-') && other[1..].chars().all(|c| c.is_ascii_digit()) => {
                opts.start_count = other[1..].parse().ok()
            }
            _ => {}
        }
        i += 1;
    }
    Some(opts)
}

/// Only the plain text of a line, for working out what is ground and what is air.
#[allow(dead_code)]
fn plain(s: &str) -> String {
    strip_colour(s)
}
