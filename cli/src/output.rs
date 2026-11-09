//! File-system output that names the path it failed on.
//!
//! A bare `io::Error` carries only the operating-system text ("No such file or directory"), so
//! a command that reads an input file and writes an output folder cannot tell the user which
//! of the two is missing or unwritable. Every output the CLI makes goes through here.

use std::fs::{self, File};
use std::io;
use std::path::Path;

fn name_path(action: &str, path: &Path, source: io::Error) -> io::Error {
    io::Error::new(
        source.kind(),
        format!("cannot {action} '{}': {source}", path.display()),
    )
}

/// `fs::create_dir_all`, naming the directory on failure.
pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    fs::create_dir_all(path).map_err(|e| name_path("create output directory", path, e))
}

/// `fs::write`, naming the file on failure.
pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let path = path.as_ref();
    fs::write(path, contents).map_err(|e| name_path("write output file", path, e))
}

/// `File::create`, naming the file on failure.
pub fn create(path: impl AsRef<Path>) -> io::Result<File> {
    let path = path.as_ref();
    File::create(path).map_err(|e| name_path("create output file", path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("unpdf-cli-output-{}-{name}", std::process::id()))
    }

    #[test]
    fn a_failure_names_the_path_and_the_role() {
        // A file where a directory is needed: neither can be created below it.
        let blocker = scratch("blocker");
        fs::write(&blocker, b"x").unwrap();
        let below = blocker.join("out");

        let dir_err = create_dir_all(&below).unwrap_err();
        assert!(
            dir_err
                .to_string()
                .contains("cannot create output directory"),
            "{dir_err}"
        );
        assert!(dir_err.to_string().contains("blocker"), "{dir_err}");

        let file_err = create(below.join("extract.md")).unwrap_err();
        assert!(
            file_err.to_string().contains("cannot create output file"),
            "{file_err}"
        );

        let write_err = write(below.join("a.txt"), b"y").unwrap_err();
        assert!(
            write_err.to_string().contains("cannot write output file"),
            "{write_err}"
        );
        assert!(write_err.to_string().contains("a.txt"), "{write_err}");

        fs::remove_file(&blocker).unwrap();
    }

    #[test]
    fn success_is_unchanged() {
        let dir = scratch("ok");
        create_dir_all(&dir).unwrap();
        write(dir.join("a.txt"), b"hello").unwrap();
        assert_eq!(fs::read(dir.join("a.txt")).unwrap(), b"hello");
        drop(create(dir.join("b.txt")).unwrap());
        fs::remove_dir_all(&dir).unwrap();
    }
}
