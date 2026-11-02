//! Delivery where nothing runs deliver.py (howldrop.exe on Windows gives
//! `--deliver`): each finished item moves from the staging folder into the
//! PC's inbox the way deliver.py moves it, shared text into `text/` and a
//! name taken getting `-1`, and the event `received REL SENDER KIND` tells
//! howldrop.exe, as announce.py's did. A link stays the inert `.url`
//! shortcut the receiver wrote: nothing behind it is fetched.

use std::io;
use std::path::{Path, PathBuf};

use crate::events::{emit, say};

const IMAGE: [&str; 11] = [
    "jpg", "jpeg", "png", "heic", "heif", "gif", "webp", "tif", "tiff", "bmp", "dng",
];
const VIDEO: [&str; 7] = ["mov", "mp4", "m4v", "avi", "mkv", "webm", "hevc"];

/// Moves `item` (a file, or a folder of them) from `staging` into `dest`,
/// with a `received` event for each file.
pub fn deliver(staging: &Path, dest: &Path, item: &Path, sender: &str, url: Option<&str>) {
    for file in files(item) {
        match move_in(staging, dest, &file) {
            Ok(rel) => {
                let kind = kind_of(&rel, url);
                let rel = rel.to_string_lossy().replace('\\', "/");
                say(format!("Receive: delivered {rel}"));
                emit(&["received", &rel, sender, kind]);
            }
            Err(e) => say(format!(
                "Receive: couldn't deliver {}: {e}",
                file.strip_prefix(staging).unwrap_or(&file).display()
            )),
        }
    }
    if item.is_dir() {
        remove_empty(item);
    }
}

/// The files in `item`, itself when it's one; no dotfiles.
fn files(item: &Path) -> Vec<PathBuf> {
    if !item.is_dir() {
        return vec![item.to_path_buf()];
    }
    let mut out = Vec::new();
    for e in std::fs::read_dir(item).into_iter().flatten().flatten() {
        if !e.file_name().to_string_lossy().starts_with('.') {
            out.extend(files(&e.path()));
        }
    }
    out.sort();
    out
}

/// `file` moved to its place under `dest`; where, relative to `dest`.
fn move_in(staging: &Path, dest: &Path, file: &Path) -> io::Result<PathBuf> {
    let rel = file.strip_prefix(staging).unwrap_or(file);
    let name = rel
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| io::Error::other("no file name"))?;
    let folder = match rel.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(p) => p.to_path_buf(),
        None if shared_text(&name) => PathBuf::from("text"),
        None => PathBuf::new(),
    };
    std::fs::create_dir_all(dest.join(&folder))?;
    let path = Path::new(&name);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let target = folder.join(free_name(&dest.join(&folder), &stem, &ext));
    let to = dest.join(&target);
    if std::fs::rename(file, &to).is_err() {
        // Another drive: copied aside, then moved in.
        let partial = to.with_file_name(format!(
            ".{}.partial",
            to.file_name().unwrap_or_default().to_string_lossy()
        ));
        std::fs::copy(file, &partial)?;
        std::fs::rename(&partial, &to)?;
        std::fs::remove_file(file)?;
    }
    Ok(target)
}

/// `stem + ext`, or `stem-N + ext` beside one already there.
fn free_name(dir: &Path, stem: &str, ext: &str) -> String {
    let mut name = format!("{stem}{ext}");
    let mut n = 1;
    while dir.join(&name).exists() {
        name = format!("{stem}-{n}{ext}");
        n += 1;
    }
    name
}

/// What iOS names text it shares: text-430B-AD01-9A-0.txt.
fn shared_text(name: &str) -> bool {
    name.strip_prefix("text-")
        .and_then(|n| n.strip_suffix(".txt"))
        .is_some_and(|middle| {
            middle
                .split('-')
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric()))
        })
}

/// link, text, image, video or file, as announce.py's kind_of.
fn kind_of(rel: &Path, url: Option<&str>) -> &'static str {
    let ext = rel
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if url.is_some() || ext == "url" {
        "link"
    } else if rel.parent().and_then(|p| p.file_name()) == Some("text".as_ref()) {
        "text"
    } else if IMAGE.contains(&ext.as_str()) {
        "image"
    } else if VIDEO.contains(&ext.as_str()) {
        "video"
    } else {
        "file"
    }
}

/// `dir` and the folders in it, those left empty.
fn remove_empty(dir: &Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.path().is_dir() {
            remove_empty(&e.path());
        }
    }
    let _ = std::fs::remove_dir(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_move_into_the_inbox() {
        let staging = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let s = staging.path();
        std::fs::write(s.join("IMG_1.JPG"), b"a").unwrap();
        std::fs::write(dest.path().join("IMG_1.JPG"), b"there").unwrap();
        deliver(s, dest.path(), &s.join("IMG_1.JPG"), "Phone", None);
        assert_eq!(
            std::fs::read(dest.path().join("IMG_1-1.JPG")).unwrap(),
            b"a"
        );
        assert!(!s.join("IMG_1.JPG").exists());

        std::fs::write(s.join("text-430B-AD01-9A-0.txt"), b"hi").unwrap();
        deliver(s, dest.path(), &s.join("text-430B-AD01-9A-0.txt"), "", None);
        assert!(
            dest.path()
                .join("text")
                .join("text-430B-AD01-9A-0.txt")
                .exists()
        );

        let album = s.join("Album");
        std::fs::create_dir_all(album.join("More")).unwrap();
        std::fs::write(album.join("a.mov"), b"v").unwrap();
        std::fs::write(album.join("More").join("b.png"), b"p").unwrap();
        deliver(s, dest.path(), &album, "", None);
        assert!(dest.path().join("Album").join("a.mov").exists());
        assert!(
            dest.path()
                .join("Album")
                .join("More")
                .join("b.png")
                .exists()
        );
        assert!(!album.exists());
    }

    #[test]
    fn kinds_and_shared_text() {
        assert_eq!(kind_of(Path::new("a.HEIC"), None), "image");
        assert_eq!(kind_of(Path::new("x/a.mov"), None), "video");
        assert_eq!(kind_of(Path::new("text/t.txt"), None), "text");
        assert_eq!(kind_of(Path::new("example.com.url"), None), "link");
        assert_eq!(kind_of(Path::new("a.pdf"), Some("https://a")), "link");
        assert_eq!(kind_of(Path::new("a.pdf"), None), "file");
        assert!(shared_text("text-430B-AD01-9A-0.txt"));
        assert!(!shared_text("text-.txt"));
        assert!(!shared_text("notes.txt"));
    }
}
