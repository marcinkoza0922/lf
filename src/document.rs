use std::{fs::File, io::Read, path::PathBuf};

pub const MAX_BYTES: u64 = 16 * 1024 * 1024;

pub struct Document {
    pub path: PathBuf,
    pub text: String,
    pub error: Option<String>,
    pub changed: bool,
}

impl Document {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let text = read(&path)?;
        Ok(Self {
            path,
            text,
            error: None,
            changed: false,
        })
    }

    pub fn check_for_changes(&mut self) {
        match read(&self.path) {
            Ok(text) => {
                self.error = None;
                self.changed = text != self.text;
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub fn reload(&mut self) -> bool {
        match read(&self.path) {
            Ok(text) => {
                self.error = None;
                self.changed = false;
                if text != self.text {
                    self.text = text;
                    return true;
                }
            }
            Err(error) => self.error = Some(error),
        }
        false
    }
}

fn read(path: &std::path::Path) -> Result<String, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular Markdown file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Document exceeds the 16 MiB limit".into());
    }
    String::from_utf8(bytes).map_err(|_| "Document must be UTF-8 text".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "md-reader-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> PathBuf {
            self.0.join("test.md")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn reload_and_recover_from_atomic_save_and_deletion() {
        let f = Fixture::new();
        fs::write(f.path(), "first").unwrap();
        let mut doc = Document::open(f.path()).unwrap();
        assert!(!doc.reload());
        fs::write(f.0.join("new"), "other").unwrap();
        fs::rename(f.0.join("new"), f.path()).unwrap();
        assert!(doc.reload());
        assert_eq!(doc.text, "other");
        fs::remove_file(f.path()).unwrap();
        assert!(!doc.reload());
        assert!(doc.error.is_some());
        assert_eq!(doc.text, "other");
        fs::write(f.path(), "recovered").unwrap();
        assert!(doc.reload());
        assert!(doc.error.is_none());
    }
    #[test]
    fn checking_changes_preserves_reading_until_reload() {
        let f = Fixture::new();
        fs::write(f.path(), "first").unwrap();
        let mut doc = Document::open(f.path()).unwrap();
        doc.check_for_changes();
        assert!(!doc.changed);
        fs::write(f.path(), "second").unwrap();
        doc.check_for_changes();
        assert!(doc.changed);
        assert_eq!(doc.text, "first");
        fs::write(f.path(), "first").unwrap();
        doc.check_for_changes();
        assert!(!doc.changed);
        fs::write(f.path(), "third").unwrap();
        doc.check_for_changes();
        assert!(doc.reload());
        assert_eq!(doc.text, "third");
        assert!(!doc.changed);
    }

    #[test]
    fn rejects_invalid_utf8_and_oversized_files() {
        let f = Fixture::new();
        fs::write(f.path(), [0xff]).unwrap();
        assert!(Document::open(f.path()).is_err());
        File::create(f.path())
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(Document::open(f.path()).is_err());
    }
}
