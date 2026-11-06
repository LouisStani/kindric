//! The folder that holds what sessions remember, `~/.kindric/memory`: rules, machine notes and
//! personal notes at the top, skills, then one zone per project. This module creates the folders
//! and measures what a session opens when it starts; it never rewrites a file that exists.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::Paths;
use crate::files::{create_private_dir, files_under};
use crate::generate::split_header;
use crate::paths::{MEMORY_ROOT_VARIABLE, expand_home};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// Default size above which the app warns about what sessions open at startup. A warning, not a
/// cap: nothing is cut or refused above it.
pub const WARNING_BYTES: u64 = 30 * 1024;

const PROJECTS: &str = "projets";
const SKILLS_DIR: &str = "skills";
const SKILL_FILE: &str = "SKILL.md";
const TOP_LEVEL: [&str; 5] = ["global", "setup", "perso", SKILLS_DIR, PROJECTS];
const OPENED_AT_STARTUP: [&str; 2] = ["global", "setup"];
const PROJECT_DIRS: [&str; 4] = ["status", "decisions", "lessons", "knowledge"];
const INSTRUCTIONS: &str = "claude.md";
const SKILLS: &str = "skills.md";
const INDEX: &str = "INDEX.md";
const PROJECT_FILES: [&str; 3] = [INSTRUCTIONS, SKILLS, INDEX];
const DIRECTORY_KEY: &str = "directory:";

/// What a session of a project opens when it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupSize {
    pub bytes: u64,
    pub over_warning: bool,
}

/// A zone and the folder it declares that holds a session's directory.
#[derive(Debug, PartialEq, Eq)]
pub struct Zone {
    pub name: String,
    /// Where its `CLAUDE.md` and `AGENTS.md` go, whatever subfolder the session starts in.
    pub folder: PathBuf,
}

/// The zone of a session's directory, and the zones that could not be read on the way.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ZoneLookup {
    pub zone: Option<Zone>,
    pub warnings: Vec<String>,
}

pub struct Memory {
    root: PathBuf,
}

impl Memory {
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    /// `~/.kindric/memory`, or the folder named by `KINDRIC_MEMORY_DIR`.
    pub fn default_root() -> io::Result<PathBuf> {
        Self::default_root_for(&Paths::from_env().map_err(io::Error::other)?)
    }

    /// The same, for the files of `paths`.
    pub fn default_root_for(paths: &Paths) -> io::Result<PathBuf> {
        match std::env::var_os(MEMORY_ROOT_VARIABLE).filter(|value| !value.is_empty()) {
            Some(root) => Ok(PathBuf::from(root)),
            None => root_for(paths),
        }
    }

    /// Whether the memory folder exists. Without it, sessions start as if this module did not exist.
    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }

    /// Whether `project` has a zone. Nothing here creates one for a session: without a zone,
    /// a session starts as if this module did not exist.
    pub fn has_project(&self, project: &str) -> bool {
        self.zone(project).is_ok_and(|zone| zone.is_dir())
    }

    /// The zone whose folders hold `directory`, the deepest one when folders are nested; on a tie,
    /// the first zone by name. A zone gives its folders in the header of its `claude.md`:
    /// `directory: ~/Dev/api, ~/Dev/api-docs`. Paths are compared by component, like the tabs of
    /// the app. A zone whose header cannot be read is a warning, and the search goes on.
    pub fn zone_of(&self, directory: &Path, home: &Path) -> io::Result<ZoneLookup> {
        let mut lookup = ZoneLookup::default();
        let mut best: Option<(usize, Zone)> = None;
        for zone in self.zone_names()? {
            let folders = match self.declared_folders(&zone, home) {
                Ok(folders) => folders,
                Err(error) => {
                    lookup.warnings.push(format!("{PROJECTS}/{zone}/{INSTRUCTIONS}: {error}"));
                    continue;
                }
            };
            for folder in folders {
                let depth = folder.components().count();
                if directory.starts_with(&folder) && best.as_ref().is_none_or(|(deepest, _)| depth > *deepest) {
                    best = Some((depth, Zone { name: zone.clone(), folder }));
                }
            }
        }
        lookup.zone = best.map(|(_, zone)| zone);
        Ok(lookup)
    }

    /// The zones, in name order.
    fn zone_names(&self) -> io::Result<Vec<String>> {
        let entries = match fs::read_dir(self.root.join(PROJECTS)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type()?.is_dir() && is_single_folder_name(&name) {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    /// The `directory:` folders of a zone's header, `~` read against `home`. A folder that is not
    /// absolute is an error: it would depend on where the session starts.
    fn declared_folders(&self, zone: &str, home: &Path) -> io::Result<Vec<PathBuf>> {
        let source = self.instructions_source(zone)?.replace("\r\n", "\n");
        let (header, _) = split_header(&source)?;
        let mut folders = Vec::new();
        for line in header.lines() {
            let Some(value) = line.strip_prefix(DIRECTORY_KEY) else {
                continue;
            };
            for text in value.split(',').map(str::trim).filter(|text| !text.is_empty()) {
                let folder = expand_home(text, home);
                if !folder.is_absolute() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("directory {text:?} must start with ~ or be absolute"),
                    ));
                }
                folders.push(folder);
            }
        }
        Ok(folders)
    }

    /// What a session of `project` receives at startup besides its `CLAUDE.md`: `global/`,
    /// `setup/` and the index of the zone. Never `perso/`, skills or the zone's other folders.
    pub fn startup_text(&self, project: &str) -> io::Result<String> {
        let zone = self.zone(project)?;
        let mut files = self.opened_by_every_session()?;
        files.push(zone.join(INDEX));
        joined_text(files)
    }

    /// What a session outside every zone receives: `global/` and `setup/`.
    pub fn shared_startup_text(&self) -> io::Result<String> {
        joined_text(self.opened_by_every_session()?)
    }

    fn opened_by_every_session(&self) -> io::Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        for name in OPENED_AT_STARTUP {
            files.extend(files_under(&self.root.join(name))?);
        }
        Ok(files)
    }

    /// The names listed in the `skills.md` of `project`: one per line, a leading `-` accepted,
    /// blank lines and `#` lines ignored.
    pub fn skill_names(&self, project: &str) -> io::Result<Vec<String>> {
        let text = read_text_if_present(&self.zone(project)?.join(SKILLS))?;
        Ok(text
            .lines()
            .map(|line| line.trim().trim_start_matches('-').trim())
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(String::from)
            .collect())
    }

    /// The folder of a skill, if the name stays one folder under `skills/` and holds a `SKILL.md`.
    pub fn skill_dir(&self, name: &str) -> Option<PathBuf> {
        if !is_single_folder_name(name) {
            return None;
        }
        let dir = self.root.join(SKILLS_DIR).join(name);
        dir.join(SKILL_FILE).is_file().then_some(dir)
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
        if !is_single_folder_name(project) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid project name: {project:?}")));
        }
        Ok(self.root.join(PROJECTS).join(project))
    }
}

/// The non-blank files, trimmed, a blank line between them.
fn joined_text(files: Vec<PathBuf>) -> io::Result<String> {
    let mut parts = Vec::new();
    for file in files {
        let text = read_text_if_present(&file)?;
        if !text.trim().is_empty() {
            parts.push(text.trim().to_string());
        }
    }
    Ok(parts.join("\n\n"))
}

fn root_for(paths: &Paths) -> io::Result<PathBuf> {
    paths.default_memory_root().ok_or_else(|| io::Error::other("the shared configuration has no parent"))
}

/// A name must stay one folder under its parent, on every system.
pub fn is_single_folder_name(name: &str) -> bool {
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

/// A missing file reads as empty; bytes that are not text cannot stop a session from starting.
fn read_text_if_present(path: &Path) -> io::Result<String> {
    match fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

/// The files of `dir` and its subfolders in name order; none if it does not exist. Links are
/// not followed, like in `dir_size`.
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
            assert!(is_single_folder_name(name), "{name:?}");
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
    fn a_project_has_a_zone_only_once_its_folder_exists() {
        let root = scratch("has");
        let memory = Memory::at(root.clone());
        assert!(!memory.has_project("kindric"));
        assert!(!memory.has_project("../kindric"));
        memory.ensure_project("kindric").unwrap();
        assert!(memory.has_project("kindric"));
        assert!(!memory.has_project("other"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_startup_text_holds_what_is_opened_and_nothing_else() {
        let root = scratch("text");
        let memory = Memory::at(root.clone());
        memory.ensure_project("kindric").unwrap();
        fs::write(root.join("global/b.md"), "RULE-B\n").unwrap();
        fs::create_dir_all(root.join("global/a")).unwrap();
        fs::write(root.join("global/a/a.md"), "RULE-A\n").unwrap();
        fs::write(root.join("setup/machine.md"), "MACHINE\n").unwrap();
        fs::write(root.join("projets/kindric/INDEX.md"), "NOTE-LIST\n").unwrap();
        fs::write(root.join("projets/kindric/claude.md"), "IN-CLAUDE-MD\n").unwrap();
        fs::write(root.join("projets/kindric/skills.md"), "IN-SKILLS-MD\n").unwrap();
        fs::write(root.join("projets/kindric/decisions/d.md"), "IN-DECISIONS\n").unwrap();
        fs::write(root.join("perso/life.md"), "IN-PERSO\n").unwrap();
        fs::create_dir_all(root.join("skills/rust")).unwrap();
        fs::write(root.join("skills/rust/SKILL.md"), "IN-SKILL\n").unwrap();
        fs::create_dir_all(root.join("projets/other")).unwrap();
        fs::write(root.join("projets/other/INDEX.md"), "OTHER-PROJECT\n").unwrap();

        assert_eq!(memory.startup_text("kindric").unwrap(), "RULE-A\n\nRULE-B\n\nMACHINE\n\nNOTE-LIST");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bytes_that_are_not_text_do_not_stop_the_startup_text() {
        let root = scratch("bytes");
        let memory = Memory::at(root.clone());
        write(root.join("global/rules.md"), 0);
        fs::write(root.join("global/rules.md"), [b'o', b'k', 0xff]).unwrap();
        assert!(memory.startup_text("kindric").unwrap().starts_with("ok"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skill_names_are_read_one_per_line() {
        let root = scratch("names-list");
        let memory = Memory::at(root.clone());
        assert!(memory.skill_names("kindric").unwrap().is_empty());
        memory.ensure_project("kindric").unwrap();
        fs::write(root.join("projets/kindric/skills.md"), "# Skills\n\n- rust\ndev\n  - front  \r\n").unwrap();
        assert_eq!(memory.skill_names("kindric").unwrap(), ["rust", "dev", "front"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_skill_needs_its_file_and_a_name_that_stays_in_place() {
        let root = scratch("skill-dir");
        let memory = Memory::at(root.clone());
        write(root.join("skills/rust/SKILL.md"), 1);
        fs::create_dir_all(root.join("skills/empty")).unwrap();
        write(root.join("perso/SKILL.md"), 1);
        assert_eq!(memory.skill_dir("rust"), Some(root.join("skills/rust")));
        assert_eq!(memory.skill_dir("empty"), None);
        assert_eq!(memory.skill_dir("missing"), None);
        assert_eq!(memory.skill_dir("../perso"), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_memory_that_does_not_exist_yet_weighs_nothing() {
        let root = scratch("empty");
        let size = Memory::at(root).startup_size("kindric", WARNING_BYTES).unwrap();
        assert_eq!(size, StartupSize { bytes: 0, over_warning: false });
    }

    fn zone_with(root: &Path, zone: &str, instructions: &str) {
        let path = root.join("projets").join(zone).join("claude.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, instructions).unwrap();
    }

    /// The zone of `~/<relative>`, with `home` standing for `~`; no warning expected.
    fn zone_of(memory: &Memory, home: &Path, relative: &str) -> Option<String> {
        let lookup = memory.zone_of(&home.join(relative), home).unwrap();
        assert!(lookup.warnings.is_empty(), "{:?}", lookup.warnings);
        lookup.zone.map(|zone| zone.name)
    }

    #[test]
    fn a_directory_belongs_to_the_zone_declaring_the_deepest_folder_holding_it() {
        let root = scratch("zone-of");
        let home = root.join("h");
        zone_with(&root, "dev", "---\ndirectory: ~/Dev\n---\n");
        zone_with(&root, "api", "---\naccess: skills/rust\ndirectory: ~/Dev/api, ~/Srv/api\n---\nrules\n");
        zone_with(&root, "no-header", "directory: ~/Dev/api/src\n");
        let memory = Memory::at(root.clone());
        assert_eq!(zone_of(&memory, &home, "Dev/api/src").as_deref(), Some("api"));
        assert_eq!(zone_of(&memory, &home, "Srv/api").as_deref(), Some("api"));
        let found = memory.zone_of(&home.join("Dev/api/src"), &home).unwrap().zone.unwrap();
        assert_eq!(found.folder, home.join("Dev/api"), "the folder declared, not the session's");
        assert_eq!(zone_of(&memory, &home, "Dev/api2").as_deref(), Some("dev"), "compared by component");
        assert_eq!(zone_of(&memory, &home, "Dev").as_deref(), Some("dev"));
        assert_eq!(zone_of(&memory, &home, ""), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_can_be_given_as_an_absolute_path() {
        let root = scratch("zone-absolute");
        zone_with(&root, "api", "---\ndirectory: /srv/api\n---\n");
        let lookup = Memory::at(root.clone()).zone_of(Path::new("/srv/api/src"), Path::new("/h")).unwrap();
        let zone = Zone { name: "api".into(), folder: PathBuf::from("/srv/api") };
        assert_eq!(lookup, ZoneLookup { zone: Some(zone), warnings: Vec::new() });
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn two_zones_on_the_same_folder_give_the_first_by_name() {
        let root = scratch("zone-tie");
        let home = root.join("h");
        zone_with(&root, "b", "---\ndirectory: ~/x\n---\n");
        zone_with(&root, "a", "---\ndirectory: ~/x\n---\n");
        assert_eq!(zone_of(&Memory::at(root.clone()), &home, "x/y").as_deref(), Some("a"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_header_with_windows_line_endings_is_read() {
        let root = scratch("zone-crlf");
        let home = root.join("h");
        zone_with(&root, "api", "---\r\ndirectory: ~/x\r\n---\r\n");
        assert_eq!(zone_of(&Memory::at(root.clone()), &home, "x").as_deref(), Some("api"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_zone_that_cannot_be_read_is_a_warning_and_the_others_still_count() {
        let root = scratch("zone-broken");
        let home = root.join("h");
        zone_with(&root, "open", "---\ndirectory: ~/x/y\n");
        zone_with(&root, "relative", "---\ndirectory: Dev/x\n---\n");
        zone_with(&root, "x", "---\ndirectory: ~/x\n---\n");
        let lookup = Memory::at(root.clone()).zone_of(&home.join("x/y"), &home).unwrap();
        assert_eq!(lookup.zone.map(|zone| zone.name).as_deref(), Some("x"));
        assert_eq!(lookup.warnings.len(), 2);
        assert!(lookup.warnings[0].starts_with("projets/open/claude.md: "));
        assert!(lookup.warnings[1].contains("\"Dev/x\" must start with ~ or be absolute"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn without_zones_no_directory_has_one() {
        let root = scratch("zone-none");
        let home = root.join("h");
        assert_eq!(zone_of(&Memory::at(root.clone()), &home, "x"), None);
        fs::create_dir_all(root.join("projets")).unwrap();
        fs::write(root.join("projets/stray.md"), "---\ndirectory: ~/x\n---\n").unwrap();
        assert_eq!(zone_of(&Memory::at(root.clone()), &home, "x"), None, "a file is not a zone");
        fs::remove_dir_all(root).unwrap();
    }
}
