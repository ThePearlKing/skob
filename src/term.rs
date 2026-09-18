//! The terminal: raw mode, its size, and the one buffer everything is drawn into.
//!
//! Nothing here knows what a skob is.  It knows how to take the keyboard away
//! from the shell, how to give it back, and how to put coloured cells on a
//! screen in a single write so the picture never tears.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::os::unix::io::AsRawFd;
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

/// Hand the terminal back exactly as it was found, allocating nothing and
/// unwinding nothing, so it is safe from a signal handler or a panic alike.
///
/// The order matters.  Mouse reporting goes off first, so the terminal stops
/// producing reports; then the input queue is thrown away, so any report it
/// already sent cannot survive us and be typed into whatever shell we came
/// from as `;38;36;1M`; only then does the old terminal state go back.
pub fn emergency_restore() {
    unsafe {
        let bye = b"\x1b[?1003l\x1b[?1006l\x1b[?7h\x1b[?25h\x1b[?1049l\x1b[0m";
        libc::write(libc::STDOUT_FILENO, bye.as_ptr() as *const libc::c_void, bye.len());
        libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
        let saved = std::ptr::addr_of!(SAVED_TERMIOS);
        if let Some(t) = *saved {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
        }
    }
}

extern "C" fn restore_and_exit(_signal: libc::c_int) {
    emergency_restore();
    unsafe { libc::_exit(0) }
}

/// The terminal as we found it, so we can put it back exactly.
pub struct Term {
    saved: Option<libc::termios>,
    /// Set once stdin has ended: a recording, or something piped in.
    spent: Cell<bool>,
    pub raw: bool,
}

impl Term {
    pub fn new() -> Term {
        Term { saved: None, spent: Cell::new(false), raw: false }
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
                // However we are told to go -- asked nicely, killed, or having
                // fallen over -- the terminal is handed back before we leave.
                for signal in [
                    libc::SIGTERM,
                    libc::SIGHUP,
                    libc::SIGINT,
                    libc::SIGQUIT,
                    libc::SIGABRT,
                    libc::SIGSEGV,
                    libc::SIGBUS,
                    libc::SIGILL,
                    libc::SIGFPE,
                    libc::SIGPIPE,
                ] {
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

    /// Throw away anything the terminal has sent and we have not read.
    pub fn drop_pending_input(&self) {
        unsafe {
            libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
        }
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
    /// Asked for, one byte at a time, on the same thread that draws -- never
    /// by a reader sitting on stdin of its own accord.  That matters for the
    /// one moment we hand the terminal over: while `nvim` has it, this is not
    /// called at all, so every key the person types is nvim's and none of them
    /// are quietly eaten on the way past.  A reader of our own would go on
    /// taking its share regardless, and would be finished for good the first
    /// time that read came back empty -- which is a `skob` that cannot be
    /// typed at and, being in raw mode, cannot be interrupted either.
    ///
    /// When stdin is not a terminal at all it ends immediately, which would
    /// let the frame loop free-run; wait the frame out by hand instead, so a
    /// recording keeps the same pace as a person watching it.
    pub fn key(&self, timeout: Duration) -> Option<u8> {
        if self.spent.get() {
            std::thread::sleep(timeout);
            return None;
        }
        let mut waiting = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let ready = unsafe { libc::poll(&mut waiting, 1, ms) };
        if ready <= 0 {
            // Nothing, or interrupted on the way: either way, no key.
            return None;
        }
        let mut byte = [0u8; 1];
        let got = unsafe {
            libc::read(libc::STDIN_FILENO, byte.as_mut_ptr() as *mut libc::c_void, 1)
        };
        match got {
            1 => Some(byte[0]),
            0 => {
                self.spent.set(true);
                None
            }
            _ => None,
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
