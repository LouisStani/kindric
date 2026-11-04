//! The folder that holds what sessions remember, `~/.kindric/memory`: rules, machine notes and
//! personal notes at the top, skills, then one zone per project. This module creates the folders
//! and measures what a session opens when it starts; it never rewrites a file that exists.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};

use kindric_accounts::Paths;
use kindric_accounts::files::create_private_dir;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// Default size above which the app warns about what sessions open at startup. A warning, not a
/// cap: nothing is cut or refused above it.
pub const WARNING_BYTES: u64 = 30 * 1024;

const PROJECTS: &str = "projets";
const TOP_LEVEL: [&str; 5] = ["global", "setup", "perso", "skills", PROJECTS];
const OPENED_AT_STARTUP: [&str; 2] = ["global", "setup"];
const PROJECT_DIRS: [&str; 4] = ["status", "decisions", "lessons", "knowledge"];
const INSTRUCTIONS: &str = "claude.md";
const PROJECT_FILES: [&str; 3] = [INSTRUCTIONS, "skills.md", "INDEX.md"];

/// What a session of a project opens when it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupSize {
    pub bytes: u64,
    pub over_warning: bool,
}

pub struct Memory {
    root: PathBuf,
}

impl Memory {
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn default_root() -> io::Result<PathBuf> {
        root_for(&Paths::from_env().map_err(io::Error::other)?)
    }

    pub fn ensure_tree(&self) -> io::Result<()> {
        for name in TOP_LEVEL {
            create_private_dir(&self.root.join(name))?;
        }
        Ok(())
    }

    /// Creates what is missing in the zone of `project`.
    pub fn ensure_project(&self, project: &str) -> io::Result<()> {
        let zone = self.zone(project)?;
        self.ensure_tree()?;
        for name in PROJECT_DIRS {
            create_private_dir(&zone.join(name))?;
        }
        for name in PROJECT_FILES {
            create_empty_if_missing(&zone.join(name))?;
        }
        Ok(())
    }

    /// The `claude.md` of `project`; a missing file is an empty one.
    pub fn instructions_source(&self, project: &str) -> io::Result<String> {
        match fs::read_to_string(self.zone(project)?.join(INSTRUCTIONS)) {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error),
        }
    }

    /// Counts `global/`, `setup/` and the files of the project's zone. What sessions open on
    /// demand is left out: `perso/`, `skills/` and the other folders of the zone.
    pub fn startup_size(&self, project: &str, warning_bytes: u64) -> io::Result<StartupSize> {
        let zone = self.zone(project)?;
        let mut bytes = 0;
        for name in OPENED_AT_STARTUP {
            bytes += dir_size(&self.root.join(name))?;
        }
        for name in PROJECT_FILES {
            bytes += file_size(&zone.join(name))?;
        }
        Ok(StartupSize { bytes, over_warning: bytes > warning_bytes })
    }

    fn zone(&self, project: &str) -> io::Result<PathBuf> {
        if !is_valid_project_name(project) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid project name: {project:?}")));
        }
        Ok(self.root.join(PROJECTS).join(project))
    }
}

fn root_for(paths: &Paths) -> io::Result<PathBuf> {
    let home = paths.shared_root.parent().ok_or_else(|| io::Error::other("the shared configuration has no parent"))?;
    Ok(home.join(".kindric").join("memory"))
}

/// A name must stay one folder under `projets/`, on every system.
fn is_valid_project_name(name: &str) -> bool {
    if name.contains(['/', '\\']) {
        return false;
    }
    let mut components = Path::new(name).components();
    matches!((components.next(), components.next()), (Some(Component::Normal(_)), None))
}

fn create_empty_if_missing(path: &Path) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    match options.open(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

/// A missing file weighs nothing.
fn file_size(path: &Path) -> io::Result<u64> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

/// Files of `dir` and its subfolders; none if it does not exist. Links are not followed, so a
/// link cannot make the walk loop.
fn dir_size(dir: &Path) -> io::Result<u64> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut bytes = 0;
    for entry in entries {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            bytes += dir_size(&entry.path())?;
        } else if kind.is_file() {
            bytes += entry.metadata()?.len();
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-memory-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    /// Every path under `root`, relative and sorted.
    fn listing(root: &Path) -> Vec<String> {
        fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                found.push(path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
                if path.is_dir() {
                    walk(root, &path, found);
                }
            }
        }
        let mut found = Vec::new();
        walk(root, root, &mut found);
        found.sort();
        found
    }

    fn write(path: PathBuf, size: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; size]).unwrap();
    }

    #[test]
    fn the_default_root_is_in_the_home_folder() {
        let paths = Paths::from_roots("/h/.config".into(), "/h/.local/share".into(), "/h".into());
        assert_eq!(root_for(&paths).unwrap(), Path::new("/h").join(".kindric").join("memory"));
    }

    #[test]
    fn the_tree_holds_the_five_top_folders_and_nothing_else() {
        let root = scratch("tree");
        Memory::at(root.clone()).ensure_tree().unwrap();
        assert_eq!(listing(&root), ["global", "perso", "projets", "setup", "skills"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_project_zone_holds_exactly_what_is_planned_and_a_second_call_changes_nothing() {
        let root = scratch("zone");
        let memory = Memory::at(root.clone());
        memory.ensure_project("kindric").unwrap();
        let expected = [
            "global",
            "perso",
            "projets",
            "projets/kindric",
            "projets/kindric/INDEX.md",
            "projets/kindric/claude.md",
            "projets/kindric/decisions",
            "projets/kindric/knowledge",
            "projets/kindric/lessons",
            "projets/kindric/skills.md",
            "projets/kindric/status",
            "setup",
            "skills",
        ];
        assert_eq!(listing(&root), expected);
        memory.ensure_project("kindric").unwrap();
        assert_eq!(listing(&root), expected);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_file_that_exists_keeps_its_content() {
        let root = scratch("keep");
        let memory = Memory::at(root.clone());
        let file = root.join("projets/kindric/claude.md");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "my rules").unwrap();
        memory.ensure_project("kindric").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "my rules");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn folders_and_files_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let root = scratch("private");
        Memory::at(root.clone()).ensure_project("kindric").unwrap();
        let mode = |relative: &str| fs::metadata(root.join(relative)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode("perso"), 0o700);
        assert_eq!(mode("projets/kindric/status"), 0o700);
        assert_eq!(mode("projets/kindric/claude.md"), 0o600);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_name_that_leaves_its_folder_is_refused_and_creates_nothing() {
        let root = scratch("names");
        let memory = Memory::at(root.clone());
        for name in ["", ".", "..", "a/b", "../x", "a\\b", "x/", "/x"] {
            let error = memory.ensure_project(name).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name:?}");
            assert_eq!(memory.startup_size(name, WARNING_BYTES).unwrap_err().kind(), io::ErrorKind::InvalidInput, "{name:?}");
        }
        assert!(!root.exists());
    }

    #[test]
    fn ordinary_project_names_are_accepted() {
        for name in ["kindric", "Pro", "mon projet", "a.b-c_d", "..x"] {
            assert!(is_valid_project_name(name), "{name:?}");
        }
    }

    #[test]
    fn the_startup_size_counts_only_what_is_opened_at_startup() {
        let root = scratch("size");
        let memory = Memory::at(root.clone());
        memory.ensure_project("kindric").unwrap();
        write(root.join("global/rules.md"), 100);
        write(root.join("global/sub/more.md"), 20);
        write(root.join("setup/machine.md"), 3);
        write(root.join("projets/kindric/claude.md"), 4000);
        write(root.join("projets/kindric/skills.md"), 500);
        write(root.join("projets/kindric/INDEX.md"), 60);

        write(root.join("perso/life.md"), 1_000_000);
        write(root.join("skills/rust/SKILL.md"), 1_000_000);
        write(root.join("projets/kindric/decisions/a.md"), 1_000_000);
        write(root.join("projets/kindric/status/b.md"), 1_000_000);
        write(root.join("projets/kindric/my-note.md"), 1_000_000);
        write(root.join("projets/other/claude.md"), 1_000_000);

        assert_eq!(memory.startup_size("kindric", WARNING_BYTES).unwrap().bytes, 4683);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_warning_starts_one_byte_above_the_threshold() {
        let root = scratch("threshold");
        let memory = Memory::at(root.clone());
        let limit = usize::try_from(WARNING_BYTES).unwrap();
        write(root.join("global/rules.md"), limit);
        let at_limit = memory.startup_size("kindric", WARNING_BYTES).unwrap();
        assert_eq!(at_limit, StartupSize { bytes: WARNING_BYTES, over_warning: false });

        write(root.join("setup/machine.md"), 1);
        let above = memory.startup_size("kindric", WARNING_BYTES).unwrap();
        assert_eq!(above, StartupSize { bytes: WARNING_BYTES + 1, over_warning: true });
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_threshold_comes_from_the_caller() {
        let root = scratch("setting");
        let memory = Memory::at(root.clone());
        write(root.join("global/rules.md"), 11);
        assert!(memory.startup_size("kindric", 10).unwrap().over_warning);
        assert!(!memory.startup_size("kindric", 11).unwrap().over_warning);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_memory_that_does_not_exist_yet_weighs_nothing() {
        let root = scratch("empty");
        let size = Memory::at(root).startup_size("kindric", WARNING_BYTES).unwrap();
        assert_eq!(size, StartupSize { bytes: 0, over_warning: false });
    }
}
