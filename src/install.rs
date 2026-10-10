//! Where Cepstral Swift keeps its voices, so a voice can be named rather than
//! given by path.
//!
//! On Windows the `ProgramFiles` variables already say which Windows this is
//! and how this process sees it: 32-bit Windows has only `ProgramFiles`
//! (`C:\Program Files`); on 64-bit Windows a 32-bit process gets
//! `ProgramFiles` = `C:\Program Files (x86)` and `ProgramW6432` =
//! `C:\Program Files`, and a 64-bit one sees both folders. Checking all three
//! covers a 64-bit Swift, an older 32-bit Swift, and either build of us.

use std::path::{Path, PathBuf};

/// Folders that hold voice folders, most specific first: `$CEPS_VOICE_ROOT`,
/// then Cepstral Swift's install locations. Only folders that exist.
pub fn voice_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(r) = std::env::var_os("CEPS_VOICE_ROOT") {
        roots.push(r.into());
    }
    if cfg!(windows) {
        for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(p) = std::env::var_os(var) {
                roots.push(Path::new(&p).join("Cepstral").join("voices"));
            }
        }
    } else {
        roots.push("/opt/swift/voices".into());
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    for r in roots {
        if r.is_dir() && !seen.iter().any(|s| same_dir(s, &r)) {
            seen.push(r);
        }
    }
    seen
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// `name` as a voice folder: itself if it is one, else, when it is a bare
/// name such as `David`, the folder of that name in the first voice root
/// that has one. `None` when neither holds a voice.idx.
pub fn find_voice(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.join("voice.idx").is_file() {
        return Some(p.to_path_buf());
    }
    if p.components().count() != 1 {
        return None;
    }
    voice_roots().into_iter().map(|r| r.join(name)).find(|d| d.join("voice.idx").is_file())
}
