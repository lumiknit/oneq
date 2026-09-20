use std::{
    cell::RefCell,
    fs,
    io::{self, BufReader, Read},
    rc::Rc,
};

use crate::strs::{self, Symbol};

/// Tracks the filename and line number of whatever bytes an `Input` is
/// currently producing, so builtins like `input_filename` and
/// `input_line_number` can report on the live cursor.
#[derive(Debug, Clone)]
pub struct InputTracker {
    /// `None` when the current source isn't a named file (stdin, a string, a pipe),
    /// matching jq's `input_filename` returning `null` in those cases.
    pub filename: Option<Symbol>,
    pub line_number: usize,
}

impl InputTracker {
    pub fn new() -> Self {
        Self {
            filename: None,
            line_number: 0,
        }
    }

    /// Wraps a fresh tracker for sharing between an `Input` and its consumer (e.g. the VM host).
    pub fn shared() -> SharedInputTracker {
        Rc::new(RefCell::new(Self::new()))
    }
}

impl Default for InputTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared handle so the `Input` chain and its host can observe the same
/// filename/line-number without an `Arc`; `Input` is never sent across threads.
pub type SharedInputTracker = Rc<RefCell<InputTracker>>;

/// StreamReader is a wrapper around various input sources using for jq.
enum InputKind<'a> {
    Empty,
    Chain {
        cur: Box<Input<'a>>,
        next: Box<Input<'a>>,
    },
    Stdin,
    Str {
        cur_string: &'a str,
        cur_pos: usize,
    },
    String {
        cur_string: String,
        cur_pos: usize,
    },
    Bytes(std::io::Cursor<Vec<u8>>),
    File {
        cur_filename: String,
        cur_file: BufReader<fs::File>,
    },
}

pub struct Input<'a> {
    kind: InputKind<'a>,
    tracker: SharedInputTracker,
}

impl<'a> Input<'a> {
    pub fn new_bytes_with_tracker(bytes: Vec<u8>, tracker: SharedInputTracker) -> Self {
        Self {
            kind: InputKind::Bytes(std::io::Cursor::new(bytes)),
            tracker,
        }
    }

    pub fn new_stdin() -> Self {
        Self::new_stdin_with_tracker(InputTracker::shared())
    }

    pub fn new_stdin_with_tracker(tracker: SharedInputTracker) -> Self {
        Input {
            kind: InputKind::Stdin,
            tracker,
        }
    }

    pub fn new_empty_with_tracker(tracker: SharedInputTracker) -> Self {
        Self::empty_with(tracker)
    }

    pub fn new_str(s: &'a str) -> Self {
        Self::new_str_with_tracker(s, InputTracker::shared())
    }

    pub fn new_str_with_tracker(s: &'a str, tracker: SharedInputTracker) -> Self {
        Input {
            kind: InputKind::Str {
                cur_string: s,
                cur_pos: 0,
            },
            tracker,
        }
    }

    pub fn new_string(s: String) -> Self {
        Self::new_string_with_tracker(s, InputTracker::shared())
    }

    pub fn new_string_with_tracker(s: String, tracker: SharedInputTracker) -> Self {
        Input {
            kind: InputKind::String {
                cur_string: s,
                cur_pos: 0,
            },
            tracker,
        }
    }

    pub fn new_files(paths: &[&str]) -> io::Result<Self> {
        Self::new_files_with_tracker(paths, InputTracker::shared())
    }

    pub fn new_files_with_tracker(paths: &[&str], tracker: SharedInputTracker) -> io::Result<Self> {
        // Reverse the paths first.
        let mut i = Input {
            kind: InputKind::Empty,
            tracker: tracker.clone(),
        };
        for p in paths.iter().rev() {
            let file = fs::File::open(p)?;
            let reader = BufReader::new(file);
            i = Input {
                kind: InputKind::Chain {
                    cur: Box::new(Input {
                        kind: InputKind::File {
                            cur_filename: p.to_string(),
                            cur_file: reader,
                        },
                        tracker: tracker.clone(),
                    }),
                    next: Box::new(i),
                },
                tracker: tracker.clone(),
            };
        }
        Ok(i)
    }

    /// Returns the shared tracker so a host can read the live filename/line number.
    pub fn tracker(&self) -> SharedInputTracker {
        self.tracker.clone()
    }

    // Returns the current filename if reading from files, or None otherwise.
    pub fn filename(&self) -> &str {
        match &self.kind {
            InputKind::Chain { cur, .. } => cur.filename(),
            InputKind::Empty => "<EMPTY>",
            InputKind::Stdin => "<STDIN>",
            InputKind::Str { .. } => "<STRING>",
            InputKind::String { .. } => "<STRING>",
            InputKind::Bytes(_) => "<STDIN>",
            InputKind::File { cur_filename, .. } => cur_filename.as_str(),
        }
    }

    /// Updates the shared tracker's filename to whatever source is currently active.
    /// Only `File` sources have a real filename; everything else reports `None`,
    /// matching jq's `input_filename` returning `null` for stdin/strings/pipes.
    fn sync_tracker_filename(&self) {
        let filename = match &self.kind {
            InputKind::Chain { cur, .. } => {
                cur.sync_tracker_filename();
                return;
            }
            InputKind::Empty => return,
            InputKind::File { cur_filename, .. } => Some(strs::intern(cur_filename)),
            InputKind::Stdin
            | InputKind::Str { .. }
            | InputKind::String { .. }
            | InputKind::Bytes(_) => None,
        };
        let mut tracker = self.tracker.borrow_mut();
        if tracker.filename != filename {
            tracker.filename = filename;
            tracker.line_number = 0;
        }
    }
}

impl<'a> Input<'a> {
    fn empty_with(tracker: SharedInputTracker) -> Self {
        Input {
            kind: InputKind::Empty,
            tracker,
        }
    }

    /// Chain delegates entirely to `cur`, which shares this tracker, so the
    /// byte counting below must not run twice for the same bytes.
    fn read_chain(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let InputKind::Chain { cur, next } = &mut self.kind else {
            unreachable!()
        };
        match cur.read(buf) {
            Err(e) => Err(e),
            Ok(z) if z > 0 => Ok(z),
            _ => {
                // Handle EOF: advance to the next input in the chain.
                let taken =
                    std::mem::replace(next, Box::new(Input::empty_with(self.tracker.clone())));
                self.kind = taken.kind;
                self.read(buf)
            }
        }
    }
}

impl<'a> Read for Input<'a> {
    /// Read the next bytes into the provided buffer.
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if matches!(self.kind, InputKind::Chain { .. }) {
            return self.read_chain(buf);
        }
        self.sync_tracker_filename();
        let n = match &mut self.kind {
            InputKind::Empty => Ok(0),
            InputKind::Chain { .. } => unreachable!(),
            InputKind::Stdin => {
                let stdin = std::io::stdin();
                let mut handle = stdin.lock();
                handle.read(buf)
            }
            InputKind::Str {
                cur_string,
                cur_pos,
            } => {
                let remaining = &cur_string.as_bytes()[*cur_pos..];
                let n = remaining.len().min(buf.len());
                buf[..n].copy_from_slice(&remaining[..n]);
                *cur_pos += n;
                Ok(n)
            }
            InputKind::String {
                cur_string,
                cur_pos,
            } => {
                let remaining = &cur_string.as_bytes()[*cur_pos..];
                let n = remaining.len().min(buf.len());
                buf[..n].copy_from_slice(&remaining[..n]);
                *cur_pos += n;
                Ok(n)
            }
            InputKind::File { cur_file, .. } => cur_file.read(buf),
            InputKind::Bytes(cursor) => cursor.read(buf),
        }?;
        if n > 0 {
            let newlines = buf[..n].iter().filter(|&&b| b == b'\n').count();
            if newlines > 0 {
                self.tracker.borrow_mut().line_number += newlines;
            }
        }
        Ok(n)
    }
}
