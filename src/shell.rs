//! The shell he lives in.
//!
//! A real scrolling terminal, one line at a time: the text rises, the skob is
//! shoved up with it, and the bottom rows become ground he can stand on.  Jobs
//! run in the background so he keeps squishing while they work; the handful of
//! commands that need the real terminal get it back for as long as they run.

use std::collections::HashMap;
use std::io::Read;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

pub const ESC: &str = "\x1b";

/// Commands that want the terminal itself: an editor, a pager, anything that
/// draws its own screen, and anything that prints by moving the cursor about
/// rather than simply going along.  These are run in the foreground with
/// everything handed back to them, because the alternative -- reading their
/// output as though it were text -- makes nonsense of them.
const NEEDS_TERMINAL: &[&str] = &[
    // editors and pagers
    "vi", "vim", "nvim", "nano", "emacs", "pico", "helix", "hx", "kak", "micro", "joe", "ed",
    "less", "more", "most", "man", "info", "bat",
    // watching the machine
    "top", "htop", "btop", "btm", "atop", "bpytop", "bashtop", "gotop", "glances", "iotop",
    "iftop", "nload", "bmon", "nvtop", "radeontop", "powertop", "s-tui", "watch", "dstat",
    // getting about
    "ranger", "nnn", "lf", "yazi", "vifm", "mc", "ncdu", "fzf", "tig", "lazygit", "lazydocker",
    "k9s", "gdu", "duf",
    // elsewhere, or as someone else
    "ssh", "sudo", "su", "doas", "telnet", "mosh", "tmux", "screen", "zellij",
    // prompts of their own
    "python", "python3", "ipython", "node", "irb", "psql", "mysql", "sqlite3", "redis-cli",
    "gdb", "lldb", "crontab", "visudo", "nmtui", "bluetuith", "alsamixer", "pulsemixer",
    "dialog", "whiptail", "cfdisk", "fdisk", "parted", "wavemon",
    // mail, chat, music, the web
    "mutt", "neomutt", "alpine", "irssi", "weechat", "cmus", "ncmpcpp", "moc", "mocp", "cava",
    "w3m", "lynx", "links", "elinks", "newsboat", "castero",
    // things whose whole point is the screen
    "cmatrix", "asciiquarium", "pipes.sh", "pipes", "sl", "cbonsai", "unimatrix", "tty-clock",
    "nethack", "moon-buggy", "bastet", "ninvaders", "2048",
    // and the fetchers, which draw themselves beside their own logo
    "neofetch", "fastfetch", "screenfetch", "pfetch", "macchina", "hyfetch", "nitch", "ufetch",
];

/// `tree /` can talk faster than any terminal can listen.  Hold at most this
/// much of a job's voice, and show at most this many of its lines a frame: the
/// ones in between would have scrolled past before you could read them.
const MAX_HELD_BYTES: usize = 256 * 1024;
const MAX_LINES_PER_FRAME: usize = 200;

/// A pipe has neither colours nor a width, so hand the job both back before it
/// starts: otherwise `ls` comes out grey and one file to a line.
const JOB_PREAMBLE: &str = r#"export COLUMNS=__COLS__
ls(){ command ls --color=always -C "$@"; }
dir(){ command dir --color=always "$@"; }
grep(){ command grep --color=always "$@"; }
egrep(){ command grep -E --color=always "$@"; }
fgrep(){ command grep -F --color=always "$@"; }
diff(){ command diff --color=always "$@"; }
tree(){ command tree -C "$@"; }
"#;

const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "function", "in", "select", "time", "coproc", "return",
];

const BUILTINS: &[&str] = &[
    "cd", "echo", "export", "alias", "unalias", "set", "unset", "source", "eval", "exec", "exit",
    "read", "printf", "pwd", "test", "true", "false", "kill", "jobs", "fg", "bg", "wait", "type",
    "shift", "local", "declare", "trap", "umask", "history", "help", "let", "ulimit", "command",
];

/// What a keystroke asked the rest of the program to do.
pub enum Action {
    Nothing,
    Leave,
    /// `skob ...` typed at the prompt: a command for the creatures, not the shell.
    Creature(String),
    /// Needs the real terminal; the caller must stand aside and run it.
    Foreground(String),
}

/// A command running in the background, with its output arriving as it comes.
pub struct Job {
    child: Child,
    output: Arc<Mutex<Vec<u8>>>,
    meta: PathBuf,
}

pub struct Shell {
    pub cwd: PathBuf,
    pub line: String,
    pub cursor: usize,
    pub history: Vec<String>,
    hist_at: usize,
    hist_saved: String,
    /// Every line ever printed: the plain text, and the text with its colours.
    pub scrollback: Vec<(String, String)>,
    /// Lines printed since the last frame, so the creatures can be shoved up.
    pub fresh: usize,
    pub last_rc: i32,
    pub job: Option<Job>,
    partial: String,
    /// Said once per job, when we start having to skip its output.
    flooded: bool,
    known: HashMap<String, Kind>,
    /// The colour of the first thing alive, used to tint the prompt.
    pub accent: u8,
    /// How wide the screen is, so jobs can lay their output out to fit.
    pub width: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Keyword,
    Builtin,
    File,
    Creature,
    Unknown,
}

impl Shell {
    pub fn new(accent: u8) -> Shell {
        Shell {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            line: String::new(),
            cursor: 0,
            history: Vec::new(),
            hist_at: 0,
            hist_saved: String::new(),
            scrollback: Vec::new(),
            fresh: 0,
            last_rc: 0,
            job: None,
            partial: String::new(),
            flooded: false,
            known: HashMap::new(),
            accent,
            width: 80,
        }
    }

    // ------------------------------------------------------------ scrollback

    pub fn print(&mut self, plain: &str, coloured: &str) {
        let plain = plain.replace('\t', "        ");
        let coloured = coloured.replace('\t', "        ");
        self.scrollback.push((plain, coloured));
        self.fresh += 1;
        if self.scrollback.len() > 2000 {
            self.scrollback.drain(..600);
        }
    }

    pub fn note(&mut self, text: &str) {
        let coloured = format!("{}[38;5;244m{}{}[0m", ESC, text, ESC);
        self.print(text, &coloured);
    }

    /// A line of a job's output, keeping whatever colours it arrived with.
    fn print_output(&mut self, line: &str) {
        let line = line.trim_end_matches('\r');
        if line.contains('\x1b') || line.bytes().any(|b| b < 32 && b != b'\t') {
            let shown = as_text(line);
            let plain = strip_colour(&shown);
            self.print(&plain, &shown);
        } else {
            self.print(line, line);
        }
    }

    // ---------------------------------------------------------------- prompt

    /// The prompt is the directory you are in, and nothing else.
    pub fn prompt(&self) -> (String, String) {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut dir = self.cwd.display().to_string();
        if !home.is_empty() {
            if dir == home {
                dir = "~".into();
            } else if let Some(rest) = dir.strip_prefix(&format!("{}/", home)) {
                dir = format!("~/{}", rest);
            }
        }
        let rc = if self.last_rc != 0 { format!(" ✗{}", self.last_rc) } else { String::new() };
        let plain = format!("{}{} $ ", dir, rc);
        let mut coloured = format!("{}[0;1;38;5;110m{}{}[0m", ESC, dir, ESC);
        if !rc.is_empty() {
            coloured.push_str(&format!("{}[38;5;203m{}{}[0m", ESC, rc, ESC));
        }
        coloured.push_str(&format!(" {}[1;38;5;220m${}[0m ", ESC, ESC));
        (plain, coloured)
    }

    // ------------------------------------------------------------- the keys

    pub fn key(&mut self, byte: u8) -> Action {
        // While a job runs the keyboard belongs to it, except for the stop key.
        if self.job.is_some() {
            if byte == 3 {
                self.stop_job();
            }
            return Action::Nothing;
        }
        match byte {
            b'\r' | b'\n' => return self.enter(),
            4 => {
                if self.line.is_empty() {
                    return Action::Leave;
                }
            }
            3 => {
                let (plain, coloured) = self.prompt();
                let line = self.line.clone();
                self.print(&format!("{}{}^C", plain, line), &format!("{}{}^C", coloured, line));
                self.line.clear();
                self.cursor = 0;
                self.last_rc = 130;
            }
            9 => self.complete(),
            12 => self.scrollback.clear(),
            1 => self.cursor = 0,
            5 => self.cursor = self.line.chars().count(),
            21 => {
                let rest: String = self.line.chars().skip(self.cursor).collect();
                self.line = rest;
                self.cursor = 0;
            }
            11 => {
                self.line = self.line.chars().take(self.cursor).collect();
            }
            23 => self.kill_word(),
            127 | 8 => {
                if self.cursor > 0 {
                    let mut chars: Vec<char> = self.line.chars().collect();
                    chars.remove(self.cursor - 1);
                    self.line = chars.into_iter().collect();
                    self.cursor -= 1;
                }
            }
            b if b >= 32 => {
                let mut chars: Vec<char> = self.line.chars().collect();
                chars.insert(self.cursor.min(chars.len()), b as char);
                self.line = chars.into_iter().collect();
                self.cursor += 1;
            }
            _ => {}
        }
        Action::Nothing
    }

    /// The escape sequences: arrows, home, end, delete, word jumps.
    pub fn escape_key(&mut self, seq: &str) {
        if self.job.is_some() {
            return;
        }
        let len = self.line.chars().count();
        match seq {
            "A" => self.history_back(),
            "B" => self.history_forward(),
            "C" => self.cursor = (self.cursor + 1).min(len),
            "D" => self.cursor = self.cursor.saturating_sub(1),
            "H" | "1~" | "7~" => self.cursor = 0,
            "F" | "4~" | "8~" => self.cursor = len,
            "3~" => {
                if self.cursor < len {
                    let mut chars: Vec<char> = self.line.chars().collect();
                    chars.remove(self.cursor);
                    self.line = chars.into_iter().collect();
                }
            }
            "1;5C" | "1;3C" => self.word_right(),
            "1;5D" | "1;3D" => self.word_left(),
            _ => {}
        }
    }

    fn enter(&mut self) -> Action {
        let line = self.line.clone();
        let (plain, coloured) = self.prompt();
        let highlighted = self.highlight(&line);
        self.print(&format!("{}{}", plain, line), &format!("{}{}", coloured, highlighted));
        self.line.clear();
        self.cursor = 0;

        if line.trim().is_empty() {
            return Action::Nothing;
        }
        if self.history.last().map(String::as_str) != Some(line.as_str()) {
            self.history.push(line.clone());
        }
        self.hist_at = self.history.len();

        let first = line.split_whitespace().next().unwrap_or("");
        match first {
            "exit" | "logout" => Action::Leave,
            // Ours, not the program of the same name: its escape codes would
            // land in the scrollback as rubbish and the old text would stay,
            // still solid, still something to stand on.
            "clear" => {
                self.scrollback.clear();
                self.last_rc = 0;
                Action::Nothing
            }
            "skob" => Action::Creature(line[first.len()..].trim().to_string()),
            f if NEEDS_TERMINAL.contains(&f) => Action::Foreground(line),
            _ => {
                self.start_job(&line);
                Action::Nothing
            }
        }
    }

    fn kill_word(&mut self) {
        let chars: Vec<char> = self.line.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1] == ' ' {
            i -= 1;
        }
        while i > 0 && chars[i - 1] != ' ' {
            i -= 1;
        }
        let rest: String = chars[self.cursor..].iter().collect();
        let head: String = chars[..i].iter().collect();
        self.line = format!("{}{}", head, rest);
        self.cursor = i;
    }

    fn word_left(&mut self) {
        let chars: Vec<char> = self.line.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1] == ' ' {
            i -= 1;
        }
        while i > 0 && chars[i - 1] != ' ' {
            i -= 1;
        }
        self.cursor = i;
    }

    fn word_right(&mut self) {
        let chars: Vec<char> = self.line.chars().collect();
        let mut i = self.cursor;
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
        while i < chars.len() && chars[i] != ' ' {
            i += 1;
        }
        self.cursor = i;
    }

    fn history_back(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.hist_at == self.history.len() {
            self.hist_saved = self.line.clone();
        }
        self.hist_at = self.hist_at.saturating_sub(1);
        self.line = self.history[self.hist_at].clone();
        self.cursor = self.line.chars().count();
    }

    fn history_forward(&mut self) {
        if self.hist_at >= self.history.len() {
            return;
        }
        self.hist_at += 1;
        self.line = if self.hist_at == self.history.len() {
            self.hist_saved.clone()
        } else {
            self.history[self.hist_at].clone()
        };
        self.cursor = self.line.chars().count();
    }

    // ----------------------------------------------------------- completion

    fn complete(&mut self) {
        let before: String = self.line.chars().take(self.cursor).collect();
        let start = before
            .rfind(|c: char| " |;&><=".contains(c))
            .map(|i| i + 1)
            .unwrap_or(0);
        let word = &before[start..];
        let first_word = before[..start].trim().is_empty();

        let mut matches: Vec<String> = if first_word && !word.contains('/') {
            self.commands_starting(word)
        } else {
            self.paths_starting(word)
        };
        matches.sort();
        matches.dedup();
        if matches.is_empty() {
            return;
        }

        let common = if matches.len() == 1 {
            let m = &matches[0];
            if m.ends_with('/') { m.clone() } else { format!("{} ", m) }
        } else {
            let prefix = longest_common_prefix(&matches);
            if prefix == word {
                // Nothing more to add: show what there is instead.
                let (plain, coloured) = self.prompt();
                let line = self.line.clone();
                let hl = self.highlight(&line);
                self.print(&format!("{}{}", plain, line), &format!("{}{}", coloured, hl));
                let mut row = String::new();
                for m in matches.iter().take(120) {
                    if row.len() + m.len() + 2 > 78 {
                        let plain = row.clone();
                        self.print(&plain, &format!("{}[38;5;110m{}{}[0m", ESC, plain, ESC));
                        row.clear();
                    }
                    row.push_str(m);
                    row.push_str("  ");
                }
                if !row.is_empty() {
                    self.print(&row.clone(), &format!("{}[38;5;110m{}{}[0m", ESC, row, ESC));
                }
                return;
            }
            prefix
        };

        let after: String = self.line.chars().skip(self.cursor).collect();
        self.line = format!("{}{}{}", &before[..start], common, after);
        self.cursor = before[..start].chars().count() + common.chars().count();
    }

    fn commands_starting(&self, word: &str) -> Vec<String> {
        let mut out: Vec<String> = BUILTINS
            .iter()
            .chain(KEYWORDS.iter())
            .filter(|c| c.starts_with(word))
            .map(|c| c.to_string())
            .collect();
        if let Ok(path) = std::env::var("PATH") {
            for dir in path.split(':') {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for e in entries.flatten() {
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.starts_with(word) {
                            out.push(name);
                        }
                    }
                }
            }
        }
        out.truncate(300);
        out
    }

    fn paths_starting(&self, word: &str) -> Vec<String> {
        let (dir_part, file_part) = match word.rfind('/') {
            Some(i) => (&word[..=i], &word[i + 1..]),
            None => ("", word),
        };
        let base = if dir_part.starts_with('/') {
            PathBuf::from(dir_part)
        } else {
            self.cwd.join(dir_part)
        };
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&base) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.starts_with(file_part) {
                    continue;
                }
                let is_dir = e.path().is_dir();
                out.push(format!("{}{}{}", dir_part, name, if is_dir { "/" } else { "" }));
            }
        }
        out
    }

    // ---------------------------------------------------------------- jobs

    fn start_job(&mut self, line: &str) {
        let meta = std::env::temp_dir().join(format!("rskob.{}.meta", std::process::id()));
        let _ = std::fs::remove_file(&meta);
        // The group is redirected as a whole, so a job's errors land in its
        // output in the order they happened, and the trailer records where the
        // job left us and what it thought of itself.
        let script = format!(
            "{}cd {} 2>/dev/null\n{{ {}\n}} 2>&1\nprintf '%s\\n%s\\n' \"$?\" \"$PWD\" > {}",
            JOB_PREAMBLE.replace("__COLS__", &self.width.to_string()),
            quote(&self.cwd.display().to_string()),
            line,
            quote(&meta.display().to_string())
        );
        let child = Command::new("bash")
            .arg("-c")
            .arg(&script)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                self.note(&format!("bash: {}", e));
                self.last_rc = 127;
                return;
            }
        };
        let output = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut out) = child.stdout.take() {
            let sink = Arc::clone(&output);
            std::thread::spawn(move || {
                let mut chunk = [0u8; 8192];
                while let Ok(n) = out.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut buf) = sink.lock() {
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.len() > MAX_HELD_BYTES {
                            let over = buf.len() - MAX_HELD_BYTES;
                            buf.drain(..over);
                        }
                    }
                }
            });
        }
        if let Some(mut err) = child.stderr.take() {
            let sink = Arc::clone(&output);
            std::thread::spawn(move || {
                let mut chunk = [0u8; 8192];
                while let Ok(n) = err.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut buf) = sink.lock() {
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.len() > MAX_HELD_BYTES {
                            let over = buf.len() - MAX_HELD_BYTES;
                            buf.drain(..over);
                        }
                    }
                }
            });
        }
        self.flooded = false;
        self.job = Some(Job { child, output, meta });
    }

    /// Take whatever the job has said since last time, a line at a time.
    pub fn drain_job(&mut self) {
        let chunk = match &self.job {
            Some(job) => match job.output.lock() {
                Ok(mut buf) => {
                    if buf.is_empty() {
                        return;
                    }
                    let text = String::from_utf8_lossy(&buf).to_string();
                    buf.clear();
                    text
                }
                Err(_) => return,
            },
            None => return,
        };
        let mut text = std::mem::take(&mut self.partial);
        text.push_str(&chunk);
        let ends_clean = text.ends_with('\n');
        let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        self.partial = if ends_clean { String::new() } else { lines.pop().unwrap_or_default() };
        if ends_clean {
            lines.pop();
        }
        if lines.len() > MAX_LINES_PER_FRAME {
            lines.drain(..lines.len() - MAX_LINES_PER_FRAME);
            if !self.flooded {
                self.flooded = true;
                self.note("  (output is coming faster than he can read it; showing the tail)");
            }
        }
        for line in lines {
            self.print_output(&line);
        }
    }

    /// True when the job has finished and been cleared away.
    pub fn reap_job(&mut self) -> bool {
        let finished = match &mut self.job {
            Some(job) => matches!(job.child.try_wait(), Ok(Some(_))),
            None => return false,
        };
        if !finished {
            return false;
        }
        // Give the reader threads a moment to hand over the last of it.
        std::thread::sleep(std::time::Duration::from_millis(5));
        self.drain_job();
        if !self.partial.is_empty() {
            let last = std::mem::take(&mut self.partial);
            self.print_output(&last);
        }
        let job = self.job.take().unwrap();
        if let Ok(meta) = std::fs::read_to_string(&job.meta) {
            let mut lines = meta.lines();
            self.last_rc = lines.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
            if let Some(dir) = lines.next() {
                let dir = PathBuf::from(dir.trim());
                if dir.is_dir() {
                    self.cwd = dir;
                }
            }
        } else {
            self.last_rc = 130;
        }
        let _ = std::fs::remove_file(&job.meta);
        true
    }

    pub fn stop_job(&mut self) {
        if let Some(job) = &mut self.job {
            let _ = job.child.kill();
        }
        self.last_rc = 130;
    }

    /// Run a command that wants the terminal, with everything handed to it.
    ///
    /// Handed over properly, which means a process group of its own and the
    /// terminal made over to it.  Otherwise the command is in *our* group, and
    /// a ^C typed at `cmatrix` is delivered to everything in that group at
    /// once -- so quitting cmatrix quit skob as well, and dropped the person
    /// back into the shell they started from.  While it holds the terminal we
    /// are a background group and hear nothing of the keys, which is exactly
    /// right: they are not ours to hear.
    pub fn run_foreground(&mut self, line: &str) {
        let meta = std::env::temp_dir().join(format!("rskob.{}.fg", std::process::id()));
        let script = format!(
            "cd {} 2>/dev/null\n{}\nprintf '%s\\n%s\\n' \"$?\" \"$PWD\" > {}",
            quote(&self.cwd.display().to_string()),
            line,
            quote(&meta.display().to_string())
        );
        let mut command = Command::new("bash");
        command.arg("-c").arg(&script);
        unsafe {
            command.pre_exec(|| {
                // Its own group, before it is anything else.
                libc::setpgid(0, 0);
                Ok(())
            });
        }
        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.note(&format!("bash: {}", e));
                self.last_rc = 127;
                return;
            }
        };
        let pid = child.id() as libc::pid_t;
        let ours = unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) };
        unsafe {
            // Said from both sides, because whichever of us gets there first
            // wins and the other is told the same thing twice.
            libc::setpgid(pid, pid);
            if ours >= 0 {
                hand_terminal_to(pid);
            }
        }
        let ended = child.wait();
        if ours >= 0 {
            unsafe { hand_terminal_to(ours) };
        }
        if let Ok(text) = std::fs::read_to_string(&meta) {
            let mut lines = text.lines();
            self.last_rc = lines.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
            if let Some(dir) = lines.next() {
                let dir = PathBuf::from(dir.trim());
                if dir.is_dir() {
                    self.cwd = dir;
                }
            }
        } else if let Some(signal) = ended.ok().and_then(|e| e.signal()) {
            // It was cut short and never got as far as saying where it left
            // us; the shell's way of putting that is 128 and the signal.
            self.last_rc = 128 + signal;
        }
        let _ = std::fs::remove_file(&meta);
    }

    fn kind_of(&mut self, word: &str) -> Kind {
        if let Some(k) = self.known.get(word) {
            return *k;
        }
        let kind = if word == "skob" {
            Kind::Creature
        } else if KEYWORDS.contains(&word) {
            Kind::Keyword
        } else if BUILTINS.contains(&word) {
            Kind::Builtin
        } else if word.contains('/') {
            if is_executable(&self.cwd.join(word)) { Kind::File } else { Kind::Unknown }
        } else if which(word).is_some() {
            Kind::File
        } else {
            Kind::Unknown
        };
        self.known.insert(word.to_string(), kind);
        kind
    }

    /// Colour a command line the way a shell with good manners would.
    pub fn highlight(&mut self, line: &str) -> String {
        let chars: Vec<char> = line.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        let mut at_command = true;

        while i < chars.len() {
            let c = chars[i];
            match c {
                ' ' | '\t' => {
                    out.push(c);
                    i += 1;
                }
                '#' => {
                    out.push_str(&format!("{}[38;5;244m", ESC));
                    out.extend(&chars[i..]);
                    i = chars.len();
                }
                '|' | '&' | ';' | '<' | '>' | '(' | ')' => {
                    let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
                    let token = match two.as_str() {
                        "&&" | "||" | ">>" | "<<" | ";;" | "|&" => two,
                        _ => c.to_string(),
                    };
                    out.push_str(&format!("{}[1;38;5;214m{}{}[0m", ESC, token, ESC));
                    i += token.chars().count();
                    at_command = true;
                }
                '\'' | '"' => {
                    let quote_char = c;
                    let mut j = i + 1;
                    while j < chars.len() && chars[j] != quote_char {
                        j += 1;
                    }
                    if j < chars.len() {
                        j += 1;
                    }
                    let text: String = chars[i..j].iter().collect();
                    out.push_str(&format!("{}[38;5;150m{}{}[0m", ESC, text, ESC));
                    i = j;
                    at_command = false;
                }
                _ => {
                    let mut j = i;
                    while j < chars.len() && !" \t|&;<>()'\"#".contains(chars[j]) {
                        j += 1;
                    }
                    let word: String = chars[i..j].iter().collect();
                    i = j;
                    if at_command {
                        if let Some(eq) = word.find('=') {
                            // NAME=value keeps us in command position.
                            out.push_str(&format!(
                                "{}[38;5;117m{}{}[1;38;5;214m={}[0;38;5;253m{}{}[0m",
                                ESC,
                                &word[..eq],
                                ESC,
                                ESC,
                                &word[eq + 1..],
                                ESC
                            ));
                            continue;
                        }
                        let colour = match self.kind_of(&word) {
                            Kind::Keyword | Kind::Builtin => "1;38;5;176".to_string(),
                            Kind::Creature | Kind::File => format!("1;38;5;{}", self.accent),
                            Kind::Unknown => "38;5;203".to_string(),
                        };
                        out.push_str(&format!("{}[{}m{}{}[0m", ESC, colour, word, ESC));
                        at_command = false;
                    } else if word.starts_with('-') {
                        out.push_str(&format!("{}[38;5;110m{}{}[0m", ESC, word, ESC));
                    } else if word.chars().all(|c| c.is_ascii_digit() || c == '.') {
                        out.push_str(&format!("{}[38;5;215m{}{}[0m", ESC, word, ESC));
                    } else {
                        out.push_str(&format!("{}[38;5;253m{}{}[0m", ESC, word, ESC));
                    }
                }
            }
        }
        out.push_str(&format!("{}[0m", ESC));
        out
    }
}

// ------------------------------------------------------------------ helpers

fn longest_common_prefix(items: &[String]) -> String {
    let mut prefix = items[0].clone();
    for item in items {
        while !item.starts_with(&prefix) {
            prefix.pop();
            if prefix.is_empty() {
                return prefix;
            }
        }
    }
    prefix
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var("PATH").ok().and_then(|path| {
        path.split(':')
            .map(|dir| Path::new(dir).join(name))
            .find(|p| is_executable(p))
    })
}

/// Single-quote a string for bash, the only way that is always safe.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The same text with every escape sequence taken out of it.
/// What a job said, as text and nothing but.
///
/// A job's output is something we print into a screen of our own; it is not a
/// terminal handed over for it to drive.  A program that hides the cursor, or
/// throws it up the screen to draw beside itself, or asks for the alternate
/// buffer, would be doing all of that to *us* -- and it outlives the program,
/// which is how one run of something like `cmatrix` leaves the cursor gone for
/// good and the shell painted over.  So colour is kept, because a colour is
/// only ever a colour, and every other sequence is dropped on the way in.
///
/// Anything that wants to drive a terminal properly should be on the list that
/// gets given the real one.
pub fn as_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    let mut body = String::new();
                    let mut ended = None;
                    for c in chars.by_ref() {
                        // A control sequence runs until its final byte.
                        if ('\x40'..='\x7e').contains(&c) {
                            ended = Some(c);
                            break;
                        }
                        body.push(c);
                    }
                    // Colour, and only if that is really all it is.
                    if ended == Some('m')
                        && body.chars().all(|c| c.is_ascii_digit() || c == ';')
                    {
                        out.push_str("\x1b[");
                        out.push_str(&body);
                        out.push('m');
                    }
                }
                // A window title and the like, which runs to a bell or an ESC.
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' {
                            chars.next();
                            break;
                        }
                    }
                }
                // Everything else: keypad modes, saved cursors, and the
                // character-set escapes, which carry a byte or two of their own
                // before the one that ends them.
                _ => {
                    while let Some(&c) = chars.peek() {
                        chars.next();
                        if !('\x20'..='\x2f').contains(&c) {
                            break;
                        }
                    }
                }
            },
            // A return starts the line again -- a progress bar rewriting
            // itself -- and with no cursor here to move, the last go at it is
            // the one that counts.
            '\r' => out.clear(),
            '\x08' => {
                out.pop();
            }
            '\t' => out.push('\t'),
            c if (c as u32) < 32 || c as u32 == 127 => {}
            c => out.push(c),
        }
    }
    out
}

/// Make the terminal over to this process group, so that what is typed at it
/// goes there.  The handing over would stop us in our tracks -- a group that
/// is not the foreground one touching the terminal is what SIGTTOU is for --
/// so that is held off for exactly as long as it takes.
unsafe fn hand_terminal_to(group: libc::pid_t) {
    let was = libc::signal(libc::SIGTTOU, libc::SIG_IGN);
    libc::tcsetpgrp(libc::STDIN_FILENO, group);
    libc::signal(libc::SIGTTOU, was);
}

pub fn strip_colour(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}
