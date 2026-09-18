//! The terminal: raw mode, its size, and the one buffer everything is drawn into.
//!
//! Nothing here knows what a skob is.  It knows how to take the keyboard away
//! from the shell, how to give it back, and how to put coloured cells on a
//! screen in a single write so the picture never tears.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

pub const ESC: char = '\x1b';

/// Screen-on: alternate buffer, no wrap, mouse reporting in SGR form.
const ENTER_ALT: &str = "\x1b[?1049h\x1b[?7l\x1b[?1003h\x1b[?1006h";
const HIDE_CURSOR: &str = "\x1b[?25l";
/// Screen-off, in the opposite order.
const LEAVE_ALT: &str = "\x1b[?1003l\x1b[?1006l\x1b[?7h\x1b[?25h\x1b[?1049l";

/// Kept where a signal handler can reach it: if we are killed outright there
/// is no unwinding and no destructor, only this.
static mut SAVED_TERMIOS: Option<libc::termios> = None;

/// Put the terminal back and go, without allocating or unwinding.
extern "C" fn restore_and_exit(_signal: libc::c_int) {
    unsafe {
        let saved = std::ptr::addr_of!(SAVED_TERMIOS);
        if let Some(t) = *saved {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
        }
        let bye = b"\x1b[?1003l\x1b[?1006l\x1b[?7h\x1b[?25h\x1b[?1049l\x1b[0m";
        libc::write(libc::STDOUT_FILENO, bye.as_ptr() as *const libc::c_void, bye.len());
        libc::_exit(0);
    }
}

/// The terminal as we found it, so we can put it back exactly.
pub struct Term {
    saved: Option<libc::termios>,
    /// Keystrokes arrive here from a thread that is always blocked on stdin.
    keys: Receiver<u8>,
    pub raw: bool,
}

impl Term {
    pub fn new() -> Term {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut byte = [0u8; 1];
            let mut stdin = std::io::stdin();
            while stdin.read(&mut byte).unwrap_or(0) == 1 {
                if tx.send(byte[0]).is_err() {
                    break;
                }
            }
        });
        Term { saved: None, keys: rx, raw: false }
    }

    /// Cooked -> raw.  Keys reach us one at a time, unechoed, and ^C is ours.
    pub fn raw_mode(&mut self) {
        let fd = std::io::stdin().as_raw_fd();
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut t) != 0 {
                return;
            }
            if self.saved.is_none() {
                self.saved = Some(t);
                let slot = std::ptr::addr_of_mut!(SAVED_TERMIOS);
                *slot = Some(t);
                // However we are told to go, the terminal is handed back first.
                for signal in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT, libc::SIGQUIT] {
                    libc::signal(signal, restore_and_exit as *const () as libc::sighandler_t);
                }
            }
            let mut raw = t;
            raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
            raw.c_iflag &= !(libc::IXON);
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            libc::tcsetattr(fd, libc::TCSANOW, &raw);
        }
        self.raw = true;
    }

    /// Hand the terminal back, for a foreground job or for good.
    pub fn cooked_mode(&mut self) {
        if let Some(saved) = self.saved {
            unsafe {
                libc::tcsetattr(std::io::stdin().as_raw_fd(), libc::TCSANOW, &saved);
            }
        }
        self.raw = false;
    }

    pub fn enter_screen(&self, show_cursor: bool) {
        let mut out = String::from(ENTER_ALT);
        if !show_cursor {
            out.push_str(HIDE_CURSOR);
        }
        print!("{}", out);
        let _ = std::io::stdout().flush();
    }

    pub fn leave_screen(&self) {
        print!("{}{}[0m", LEAVE_ALT, ESC);
        let _ = std::io::stdout().flush();
    }

    /// Wait up to `timeout` for one byte of input.
    ///
    /// When stdin is not a terminal the reader has already hit end of file and
    /// the channel is closed, which would otherwise hand back `None` the
    /// instant it is asked and let the frame loop free-run.  Wait out the rest
    /// of the frame by hand instead, so a recording keeps the same pace as a
    /// person watching it.
    pub fn key(&self, timeout: Duration) -> Option<u8> {
        match self.keys.recv_timeout(timeout) {
            Ok(byte) => Some(byte),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                std::thread::sleep(timeout);
                None
            }
        }
    }
}

/// How many columns and rows we have to play with.
pub fn size() -> (usize, usize) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            return (ws.ws_col as usize, ws.ws_row as usize);
        }
    }
    (80, 24)
}

/// One frame, built up in memory and written once.
///
/// The screen is never cleared wholesale.  Blanking everything and painting it
/// again shows the terminal a half-drawn picture, which with a hundred things
/// on screen reads as the whole display shivering.  Instead each frame
/// remembers which cells it lit, and the next one blanks only the cells it no
/// longer needs -- so a still scene sends almost nothing at all.
pub struct Screen {
    buf: String,
    /// Every cell this frame lit, and what it took to light it, so a cell can
    /// be put back after the row under it has been wiped.
    lit: HashMap<(usize, usize), String>,
    was_lit: HashMap<(usize, usize), String>,
    /// Rows redrawn in full this frame, which therefore need no tidying.
    whole_rows: HashSet<usize>,
    was_whole: HashSet<usize>,
    /// Where the terminal cursor belongs when the frame is done.  It is put
    /// there last of all: the tidying at the end of a frame moves the cursor
    /// about, and wherever it stops is where the terminal will draw it.
    cursor: Option<(usize, usize)>,
    size: (usize, usize),
}

impl Screen {
    pub fn new() -> Screen {
        Screen {
            buf: String::with_capacity(64 * 1024),
            lit: HashMap::new(),
            was_lit: HashMap::new(),
            whole_rows: HashSet::new(),
            was_whole: HashSet::new(),
            cursor: None,
            size: (0, 0),
        }
    }

    /// Start a frame.  Only a resize justifies clearing the screen.
    pub fn begin(&mut self, cols: usize, rows: usize) {
        self.buf.clear();
        if self.size != (cols, rows) {
            self.size = (cols, rows);
            self.was_lit.clear();
            self.was_whole.clear();
            self.buf.push_str("\x1b[H\x1b[2J");
        }
        self.lit.clear();
        self.whole_rows.clear();
        self.cursor = None;
    }

    /// Where to leave the cursor sitting once everything else is drawn.
    pub fn cursor_at(&mut self, row: usize, col: usize) {
        self.cursor = Some((row, col));
    }

    /// A row written end to end, cleared to the margin: nothing left of what
    /// was there before it can survive, so nothing needs tidying afterwards.
    pub fn row(&mut self, row: usize, contents: &str) {
        self.buf.push_str(&format!("\x1b[{};1H{}\x1b[0m\x1b[K", row, contents));
        self.whole_rows.insert(row);
    }

    /// Every cell states its own colour and weight, so no attribute can bleed
    /// from the cell drawn before it into the cell drawn after.
    pub fn put(&mut self, row: usize, col: usize, colour: u8, bold: bool, ch: char) {
        let cell = format!(
            "\x1b[{};{}H\x1b[0;{}38;5;{}m{}",
            row,
            col,
            if bold { "1;" } else { "" },
            colour,
            ch
        );
        self.buf.push_str(&cell);
        self.lit.insert((row, col), cell);
    }

    /// Finish the frame: take back whatever the last one put on screen and this
    /// one did not.
    pub fn end(&mut self) {
        let mut tidy = String::new();

        // A row written end to end last frame and not written at all this one
        // has to be wiped, or a banner that has been dismissed, or a line of
        // scrollback that has been cleared, simply stays there.
        let gone: Vec<usize> =
            self.was_whole.difference(&self.whole_rows).copied().collect();
        for row in &gone {
            tidy.push_str(&format!("\x1b[{};1H\x1b[K", row));
            // ...and anything this frame drew on that row goes back down.
            for ((r, _), cell) in self.lit.iter() {
                if r == row {
                    tidy.push_str(cell);
                }
            }
        }

        for &(row, col) in self.was_lit.keys() {
            if self.lit.contains_key(&(row, col))
                || self.whole_rows.contains(&row)
                || gone.contains(&row)
            {
                continue;
            }
            tidy.push_str(&format!("\x1b[{};{}H ", row, col));
        }

        if !tidy.is_empty() {
            self.buf.push_str("\x1b[0m");
            self.buf.push_str(&tidy);
        }
        std::mem::swap(&mut self.was_lit, &mut self.lit);
        std::mem::swap(&mut self.was_whole, &mut self.whole_rows);

        if let Some((row, col)) = self.cursor {
            self.buf.push_str(&format!("\x1b[{};{}H", row, col));
        }
    }

    pub fn flush(&self) {
        let mut out = std::io::stdout();
        let _ = out.write_all(self.buf.as_bytes());
        let _ = out.flush();
    }
}
