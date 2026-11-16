//! Controller-friendly file browser (native Windows dialogs can't be driven with a gamepad).
//! The top level ("This PC") lists common folders and drives.

use std::path::{Path, PathBuf};

use cs_input::NavAction;

use crate::nav::list_move;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Executables,
    Images,
    Videos,
}

impl Filter {
    fn matches(self, path: &Path) -> bool {
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        let ext = ext.as_deref().unwrap_or("");
        match self {
            Filter::Executables => ext == "exe",
            Filter::Images => matches!(ext, "png" | "jpg" | "jpeg" | "webp" | "ico" | "bmp"),
            Filter::Videos => matches!(ext, "mp4" | "webm"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Up,
    Place,
    Drive,
    Dir,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FbOutcome {
    Continue,
    Picked(PathBuf),
    Cancel,
}

#[derive(Debug, Clone)]
pub struct FileBrowser {
    pub title: String,
    filter: Filter,
    /// `None` = the "This PC" level.
    pub dir: Option<PathBuf>,
    pub entries: Vec<Entry>,
    pub focus: usize,
    pub error: Option<String>,
}

impl FileBrowser {
    pub fn new(title: impl Into<String>, filter: Filter, start: Option<PathBuf>) -> Self {
        let mut fb = Self { title: title.into(), filter, dir: None, entries: Vec::new(), focus: 0, error: None };
        fb.open(start.filter(|p| p.is_dir()));
        fb
    }

    /// Text for the subtitle line.
    pub fn location(&self) -> String {
        match (&self.dir, &self.error) {
            (_, Some(e)) => e.clone(),
            (Some(d), None) => d.display().to_string(),
            (None, None) => "This PC".into(),
        }
    }

    pub fn handle(&mut self, action: NavAction) -> FbOutcome {
        match action {
            NavAction::Up => self.focus = list_move(self.focus, self.entries.len(), -1),
            NavAction::Down => self.focus = list_move(self.focus, self.entries.len(), 1),
            NavAction::PrevTab => self.focus = list_move(self.focus, self.entries.len(), -8),
            NavAction::NextTab => self.focus = list_move(self.focus, self.entries.len(), 8),
            NavAction::Accept => return self.activate(self.focus),
            NavAction::Back => {
                if self.dir.is_none() {
                    return FbOutcome::Cancel;
                }
                self.go_up();
            }
            NavAction::Y => self.open(None),
            _ => {}
        }
        FbOutcome::Continue
    }

    pub fn activate(&mut self, index: usize) -> FbOutcome {
        let Some(entry) = self.entries.get(index).cloned() else { return FbOutcome::Continue };
        self.focus = index;
        match entry.kind {
            EntryKind::Up => self.go_up(),
            EntryKind::Place | EntryKind::Drive | EntryKind::Dir => self.open(Some(entry.path)),
            EntryKind::File => return FbOutcome::Picked(entry.path),
        }
        FbOutcome::Continue
    }

    fn go_up(&mut self) {
        let Some(dir) = self.dir.clone() else { return };
        self.open(dir.parent().map(Path::to_path_buf));
        // Re-focus the folder we came from.
        if let Some(i) = self.entries.iter().position(|e| e.path == dir) {
            self.focus = i;
        }
    }

    fn open(&mut self, dir: Option<PathBuf>) {
        self.error = None;
        self.focus = 0;
        match dir {
            None => {
                self.dir = None;
                self.entries = top_level();
            }
            Some(dir) => match read_dir(&dir, self.filter) {
                Ok(mut entries) => {
                    entries.insert(0, Entry { kind: EntryKind::Up, name: "..".into(), path: dir.clone() });
                    self.dir = Some(dir);
                    self.entries = entries;
                }
                Err(e) => self.error = Some(format!("Can't open {}: {e}", dir.display())),
            },
        }
    }
}

fn read_dir(dir: &Path, filter: Filter) -> std::io::Result<Vec<Entry>> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip system folders like $Recycle.Bin and dotfiles.
        if name.starts_with('$') || name.starts_with('.') {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            dirs.push(Entry { kind: EntryKind::Dir, name, path });
        } else if filter.matches(&path) {
            files.push(Entry { kind: EntryKind::File, name, path });
        }
    }
    let by_name = |a: &Entry, b: &Entry| a.name.to_lowercase().cmp(&b.name.to_lowercase());
    dirs.sort_by(by_name);
    files.sort_by(by_name);
    dirs.extend(files);
    Ok(dirs)
}

fn top_level() -> Vec<Entry> {
    let mut out = Vec::new();
    let mut place = |name: &str, path: Option<PathBuf>| {
        if let Some(path) = path.filter(|p| p.is_dir()) {
            out.push(Entry { kind: EntryKind::Place, name: name.into(), path });
        }
    };
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    place("Desktop", home.as_ref().map(|h| h.join("Desktop")));
    place("Downloads", home.as_ref().map(|h| h.join("Downloads")));
    place("Pictures", home.as_ref().map(|h| h.join("Pictures")));
    place("Program Files", std::env::var_os("ProgramFiles").map(PathBuf::from));
    place("Program Files (x86)", std::env::var_os("ProgramFiles(x86)").map(PathBuf::from));

    if cfg!(windows) {
        for letter in b'A'..=b'Z' {
            let root = PathBuf::from(format!("{}:\\", letter as char));
            if root.exists() {
                out.push(Entry { kind: EntryKind::Drive, name: format!("Drive ({}:)", letter as char), path: root });
            }
        }
    } else {
        out.push(Entry { kind: EntryKind::Drive, name: "/".into(), path: PathBuf::from("/") });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!("cs-files-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Games/Celeste")).unwrap();
        std::fs::create_dir_all(root.join("$Recycle.Bin")).unwrap();
        std::fs::write(root.join("Games/Celeste/Celeste.exe"), b"").unwrap();
        std::fs::write(root.join("Games/Celeste/readme.txt"), b"").unwrap();
        std::fs::write(root.join("Games/Celeste/cover.PNG"), b"").unwrap();
        root
    }

    #[test]
    fn browse_filter_pick_and_back() {
        let root = fixture();
        let mut fb = FileBrowser::new("Pick", Filter::Executables, Some(root.clone()));
        let names: Vec<_> = fb.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["..", "Games"]); // $Recycle.Bin hidden

        fb.handle(NavAction::Down);
        fb.handle(NavAction::Accept); // Games
        fb.handle(NavAction::Down);
        fb.handle(NavAction::Accept); // Celeste
        let names: Vec<_> = fb.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["..", "Celeste.exe"]); // txt and png filtered out

        fb.handle(NavAction::Back);
        assert_eq!(fb.dir.as_deref(), Some(root.join("Games").as_path()));
        assert_eq!(fb.entries[fb.focus].name, "Celeste"); // re-focused where we came from
        fb.handle(NavAction::Accept);
        fb.handle(NavAction::Down);
        assert_eq!(fb.handle(NavAction::Accept), FbOutcome::Picked(root.join("Games/Celeste/Celeste.exe")));

        let mut img = FileBrowser::new("Pick", Filter::Images, Some(root.join("Games/Celeste")));
        assert_eq!(img.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["..", "cover.PNG"]);
        img.handle(NavAction::Y);
        assert!(img.dir.is_none());
        assert_eq!(img.handle(NavAction::Back), FbOutcome::Cancel);
        std::fs::remove_dir_all(root).unwrap();
    }
}
