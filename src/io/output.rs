use std::{
    cell::RefCell,
    fs,
    io::{self, BufWriter, Write},
    rc::Rc,
};

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use {std::path::Path, tempfile::NamedTempFile};

pub enum Output {
    Nop,
    Stdout,
    Bytes {
        buf: Rc<RefCell<Vec<u8>>>,
    },
    File {
        file: BufWriter<fs::File>,
    },
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    AtomicFile {
        target_path: String,
        file: BufWriter<NamedTempFile>,
    },
}

impl Output {
    pub fn new_nop() -> Self {
        Output::Nop
    }

    pub fn new_stdout() -> Self {
        Output::Stdout
    }

    pub fn new_string_buffer(buf: Rc<RefCell<Vec<u8>>>) -> Self {
        Output::Bytes { buf }
    }

    pub fn new_file(path: String) -> io::Result<Self> {
        let file = fs::File::create(&path)?;
        Ok(Output::File {
            file: BufWriter::new(file),
        })
    }

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub fn new_inplace(target_path: &str) -> io::Result<Self> {
        // Create the temp file next to the target so the final rename stays
        // on the same filesystem (required for it to be atomic).
        let dir = Path::new(target_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file = NamedTempFile::new_in(dir)?;
        Ok(Output::AtomicFile {
            target_path: target_path.to_string(),
            file: BufWriter::new(file),
        })
    }

    pub fn finish(mut self) -> io::Result<()> {
        self.flush()?;
        match self {
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            Output::AtomicFile { target_path, file } => {
                let tmp = file
                    .into_inner()
                    .map_err(|e| io::Error::other(e.to_string()))?;
                tmp.persist(&target_path)
                    .map_err(|e| io::Error::other(e.to_string()))?;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

impl Write for Output {
    fn write(&mut self, content: &[u8]) -> io::Result<usize> {
        match self {
            Output::Nop => Ok(content.len()),
            Output::Stdout => super::stdout().write(content),
            Output::Bytes { buf: b } => {
                b.borrow_mut().extend_from_slice(content);
                Ok(content.len())
            }
            Output::File { file, .. } => file.write(content),
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            Output::AtomicFile { file, .. } => file.write(content),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Output::Nop => Ok(()),
            Output::Stdout => super::stdout().flush(),
            Output::Bytes { .. } => Ok(()),
            Output::File { file, .. } => file.flush(),
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            Output::AtomicFile { file, .. } => file.flush(),
        }
    }
}
