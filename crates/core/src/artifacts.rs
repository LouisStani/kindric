//! Artifacts: the files a session made that are worth keeping once it is gone, copied into
//! kindric's data folder by project, `artifacts/<project>/user/` for those the user asked for and
//! `artifacts/<project>/claude/` for those Claude will need again. A group is a subfolder.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Deserialize;

/// The folder of the sessions of no project.
pub const OTHER: &str = "other";
const LONGEST_GROUP: usize = 64;
/// `name-2.png` … at most this far before giving up.
const MOST_COPIES: u32 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    User,
    Claude,
}

impl Kind {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "user" => Ok(Self::User),
            "claude" => Ok(Self::Claude),
            other => Err(format!("\"{other}\" is not a kind of artifact: use \"user\" or \"claude\"")),
        }
    }

    pub fn folder(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Claude => "claude",
        }
    }
}

/// The folder of a project: its whole path, each character other than a letter, a digit or `-`
/// turned into `-`, as Claude Code names its transcript folders. Two projects of the same name
/// in different places stay apart.
pub fn folder_name(project_dir: &Path) -> String {
    let normal: PathBuf = project_dir.components().collect();
    normal.to_string_lossy().chars().map(|character| if character.is_ascii_alphanumeric() || character == '-' { character } else { '-' }).collect()
}

#[derive(Deserialize)]
struct ProjectEntry {
    directory: PathBuf,
}

/// The folders of the projects in `projects.json`; none when the file does not exist yet.
pub fn read_projects(data_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let path = data_dir.join("projects.json");
    let json = match fs::read_to_string(&path) {
        Ok(json) => json,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let entries: Vec<ProjectEntry> = serde_json::from_str(&json).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Ok(entries.into_iter().map(|entry| entry.directory).collect())
}

/// The project holding `cwd`, the deepest one when projects are nested, compared by component
/// (`~/Dev/api2` is not in `~/Dev/api`): the rule of the desktop's tabs.
pub fn project_of<'a>(cwd: &Path, projects: &'a [PathBuf]) -> Option<&'a Path> {
    projects.iter().filter(|project| cwd.starts_with(project)).max_by_key(|project| project.components().count()).map(PathBuf::as_path)
}

/// Where the artifacts of a session working in `cwd` go.
pub fn root_for(data_dir: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let projects = read_projects(data_dir)?;
    Ok(root_of_project(data_dir, project_of(cwd, &projects)))
}

/// The artifacts of `project`, or of the sessions of no project.
pub fn root_of_project(data_dir: &Path, project: Option<&Path>) -> PathBuf {
    data_dir.join("artifacts").join(project.map_or_else(|| OTHER.to_string(), folder_name))
}

/// A group is one folder name, valid on every system.
fn check_group(group: &str) -> Result<(), String> {
    let forbidden = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
    let valid = !group.is_empty()
        && group.chars().count() <= LONGEST_GROUP
        && group.trim() == group
        && !group.starts_with('.')
        && !group.contains(forbidden)
        && !group.chars().any(char::is_control);
    if valid {
        return Ok(());
    }
    Err(format!(
        "\"{group}\" cannot name a group: 1 to {LONGEST_GROUP} characters, no / \\ : * ? \" < > |, not starting with a dot"
    ))
}

/// `name`, then `stem-2.ext`, `stem-3.ext`…
fn numbered(name: &Path, number: u32) -> PathBuf {
    if number == 1 {
        return name.to_path_buf();
    }
    let stem = name.file_stem().map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    match name.extension() {
        Some(extension) => PathBuf::from(format!("{stem}-{number}.{}", extension.to_string_lossy())),
        None => PathBuf::from(format!("{stem}-{number}")),
    }
}

/// Copies `source` into `root/<kind>/[group/]` under its own name. A name taken by the same
/// content gives the copy already there; a name taken by another file gets `-2`, `-3`…
/// Nothing is ever written over.
pub fn keep(root: &Path, kind: Kind, group: Option<&str>, source: &Path) -> Result<PathBuf, String> {
    let meta = fs::metadata(source).map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    if meta.is_dir() {
        return Err(format!("{} is a folder: keep its files one by one", source.display()));
    }
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", source.display()));
    }
    let name = source.file_name().map(Path::new).ok_or_else(|| format!("{} has no file name", source.display()))?;
    let mut folder = root.join(kind.folder());
    if let Some(group) = group {
        check_group(group)?;
        folder.push(group);
    }
    fs::create_dir_all(&folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    let bytes = fs::read(source).map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    for number in 1..=MOST_COPIES {
        let target = folder.join(numbered(name, number));
        match OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(mut file) => {
                file.write_all(&bytes).map_err(|error| format!("cannot write {}: {error}", target.display()))?;
                return Ok(target);
            },
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                if fs::read(&target).is_ok_and(|existing| existing == bytes) {
                    return Ok(target);
                }
            },
            Err(error) => return Err(format!("cannot write {}: {error}", target.display())),
        }
    }
    Err(format!("{} already holds {MOST_COPIES} files named after {}", folder.display(), name.display()))
}

/// The artifacts of one kind, ungrouped first then by group, each by name: paths relative to
/// `root/<kind>/`.
pub fn list(root: &Path, kind: Kind) -> Result<Vec<PathBuf>, String> {
    let folder = root.join(kind.folder());
    let entries = match fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", folder.display())),
    };
    let mut ungrouped = Vec::new();
    let mut grouped = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = PathBuf::from(entry.file_name());
        if path.is_dir() {
            let files = fs::read_dir(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            grouped.extend(files.flatten().filter(|file| file.path().is_file()).map(|file| name.join(file.file_name())));
        } else if path.is_file() {
            ungrouped.push(name);
        }
    }
    ungrouped.sort();
    grouped.sort();
    ungrouped.extend(grouped);
    Ok(ungrouped)
}

/// A kept file as the artifacts window lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// None for an unsorted file.
    pub group: Option<String>,
    pub name: String,
    /// Seconds since 1970, 0 when the system cannot tell.
    pub modified: u64,
}

/// The files of one kind, unsorted first then by group, with their dates.
pub fn entries(root: &Path, kind: Kind) -> Result<Vec<Listed>, String> {
    let folder = root.join(kind.folder());
    list(root, kind)?
        .into_iter()
        .map(|relative| {
            let modified = fs::metadata(folder.join(&relative))
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_secs());
            let name = relative.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
            let group = relative.parent().filter(|parent| !parent.as_os_str().is_empty()).map(|parent| parent.to_string_lossy().into_owned());
            Ok(Listed { group, name, modified })
        })
        .collect()
}

/// The groups of one kind, empty ones included, by name.
pub fn groups(root: &Path, kind: Kind) -> Result<Vec<String>, String> {
    let folder = root.join(kind.folder());
    let entries = match fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", folder.display())),
    };
    let mut names: Vec<String> = entries.flatten().filter(|entry| entry.path().is_dir()).map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    Ok(names)
}

/// A file name the page sends back: one plain component, never `..` or a path.
fn check_name(name: &str) -> Result<(), String> {
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) if !name.contains(['/', '\\']) => Ok(()),
        _ => Err(format!("\"{name}\" is not the name of a kept file")),
    }
}

fn folder_of(root: &Path, kind: Kind, group: Option<&str>) -> Result<PathBuf, String> {
    let mut folder = root.join(kind.folder());
    if let Some(group) = group {
        check_group(group)?;
        folder.push(group);
    }
    Ok(folder)
}

/// The kept file `name` of `group` (None: unsorted), which must exist.
pub fn file_path(root: &Path, kind: Kind, group: Option<&str>, name: &str) -> Result<PathBuf, String> {
    check_name(name)?;
    let path = folder_of(root, kind, group)?.join(name);
    if !path.is_file() {
        return Err(format!("{name} is no longer there"));
    }
    Ok(path)
}

/// Moves a kept file to `to` (None: unsorted). A name taken there gets `-2`, `-3`…
pub fn move_to(root: &Path, kind: Kind, group: Option<&str>, name: &str, to: Option<&str>) -> Result<PathBuf, String> {
    let source = file_path(root, kind, group, name)?;
    let folder = folder_of(root, kind, to)?;
    if source.parent() == Some(folder.as_path()) {
        return Ok(source);
    }
    fs::create_dir_all(&folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    for number in 1..=MOST_COPIES {
        let target = folder.join(numbered(Path::new(name), number));
        if target.exists() {
            continue;
        }
        fs::rename(&source, &target).map_err(|error| format!("cannot move {name}: {error}"))?;
        return Ok(target);
    }
    Err(format!("{} already holds {MOST_COPIES} files named after {name}", folder.display()))
}

pub fn create_group(root: &Path, kind: Kind, group: &str) -> Result<(), String> {
    let folder = folder_of(root, kind, Some(group))?;
    fs::create_dir_all(&folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))
}

pub fn rename_group(root: &Path, kind: Kind, from: &str, to: &str) -> Result<(), String> {
    let source = folder_of(root, kind, Some(from))?;
    let target = folder_of(root, kind, Some(to))?;
    if !source.is_dir() {
        return Err(format!("there is no group named {from}"));
    }
    if target.exists() {
        return Err(format!("a group named {to} already exists"));
    }
    fs::rename(&source, &target).map_err(|error| format!("cannot rename {from}: {error}"))
}

/// Removes a group: its files go back to the unsorted ones, nothing is deleted.
pub fn remove_group(root: &Path, kind: Kind, group: &str) -> Result<(), String> {
    let folder = folder_of(root, kind, Some(group))?;
    let files = fs::read_dir(&folder).map_err(|error| format!("cannot read {}: {error}", folder.display()))?;
    for file in files.flatten().filter(|file| file.path().is_file()) {
        move_to(root, kind, Some(group), &file.file_name().to_string_lossy(), None)?;
    }
    fs::remove_dir(&folder).map_err(|error| format!("cannot remove the group {group}: {error}"))
}

pub fn delete(root: &Path, kind: Kind, group: Option<&str>, name: &str) -> Result<(), String> {
    let path = file_path(root, kind, group, name)?;
    fs::remove_file(&path).map_err(|error| format!("cannot delete {name}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-artifacts-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[cfg(unix)]
    #[test]
    fn a_project_folder_is_named_after_its_whole_path() {
        assert_eq!(folder_name(Path::new("/home/lystriq/Dev/Proj/kindric")), "-home-lystriq-Dev-Proj-kindric");
        assert_eq!(folder_name(Path::new("/home/lystriq/Dev/Proj/app/")), "-home-lystriq-Dev-Proj-app");
        assert_eq!(folder_name(Path::new("/home/u/kindric/.claude/wt_1 é")), "-home-u-kindric--claude-wt-1--");
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_project_folder_is_named_after_its_whole_path() {
        assert_eq!(folder_name(Path::new(r"C:\Users\louis\kindric")), "C--Users-louis-kindric");
    }

    #[test]
    fn the_deepest_project_holds_the_folder() {
        let projects = [PathBuf::from("/home/u/Dev"), PathBuf::from("/home/u/Dev/api"), PathBuf::from("/home/u/Pro")];
        assert_eq!(project_of(Path::new("/home/u/Dev/api/src"), &projects), Some(Path::new("/home/u/Dev/api")));
        assert_eq!(project_of(Path::new("/home/u/Dev/api2"), &projects), Some(Path::new("/home/u/Dev")));
        assert_eq!(project_of(Path::new("/tmp/x"), &projects), None);
    }

    #[test]
    fn the_project_comes_from_projects_json_and_no_project_goes_to_other() {
        let data = scratch("root");
        let project = data.join("Dev").join("api");
        assert_eq!(root_for(&data, &project).unwrap(), data.join("artifacts").join(OTHER), "no projects.json yet");
        let json = serde_json::json!([{ "name": "api", "directory": project, "account": "louis" }]).to_string();
        fs::write(data.join("projects.json"), json).unwrap();
        assert_eq!(root_for(&data, &project.join("src")).unwrap(), data.join("artifacts").join(folder_name(&project)));
        fs::write(data.join("projects.json"), "{broken").unwrap();
        assert!(root_for(&data, &project).unwrap_err().contains("projects.json"));
        fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn a_kept_file_is_copied_and_never_written_over() {
        let root = scratch("keep");
        let made = root.join("made");
        fs::create_dir_all(&made).unwrap();
        fs::write(made.join("slide.png"), b"one").unwrap();
        let first = keep(&root.join("store"), Kind::User, None, &made.join("slide.png")).unwrap();
        assert_eq!(first, root.join("store").join("user").join("slide.png"));
        assert_eq!(keep(&root.join("store"), Kind::User, None, &made.join("slide.png")).unwrap(), first, "same content: the copy already there");
        fs::write(made.join("slide.png"), b"two").unwrap();
        let second = keep(&root.join("store"), Kind::User, None, &made.join("slide.png")).unwrap();
        assert_eq!(second, root.join("store").join("user").join("slide-2.png"));
        assert_eq!(fs::read(&first).unwrap(), b"one");
        assert_eq!(fs::read(&second).unwrap(), b"two");
        assert!(made.join("slide.png").exists(), "the source stays where it is");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn groups_are_folders_and_bad_names_or_folders_are_refused() {
        let root = scratch("groups");
        fs::write(root.join("make.py"), b"print()").unwrap();
        let kept = keep(&root.join("store"), Kind::Claude, Some("carrousels"), &root.join("make.py")).unwrap();
        assert_eq!(kept, root.join("store").join("claude").join("carrousels").join("make.py"));
        for group in ["", "../x", "a/b", r"a\b", ".hidden", " padded", "a:b"] {
            assert!(keep(&root.join("store"), Kind::Claude, Some(group), &root.join("make.py")).is_err(), "{group:?}");
        }
        assert!(keep(&root.join("store"), Kind::User, None, &root).unwrap_err().contains("is a folder"));
        assert!(keep(&root.join("store"), Kind::User, None, &root.join("missing.png")).unwrap_err().contains("cannot read"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_list_gives_ungrouped_files_first_then_groups() {
        let root = scratch("list");
        for (group, name) in [(None, "b.png"), (Some("logos"), "logo.svg"), (None, "a.md"), (Some("carrousels"), "s1.png")] {
            let source = root.join(name);
            fs::write(&source, name).unwrap();
            keep(&root.join("store"), Kind::User, group, &source).unwrap();
        }
        let listed = list(&root.join("store"), Kind::User).unwrap();
        let expected: Vec<PathBuf> =
            [PathBuf::from("a.md"), PathBuf::from("b.png"), Path::new("carrousels").join("s1.png"), Path::new("logos").join("logo.svg")].into();
        assert_eq!(listed, expected);
        assert!(list(&root.join("store"), Kind::Claude).unwrap().is_empty(), "no folder yet");
        fs::remove_dir_all(root).unwrap();
    }

    fn store_with(label: &str, files: &[(Option<&str>, &str)]) -> PathBuf {
        let root = scratch(label);
        for (group, name) in files {
            let source = root.join(name);
            fs::write(&source, name).unwrap();
            keep(&root.join("store"), Kind::User, *group, &source).unwrap();
            fs::remove_file(source).unwrap();
        }
        root
    }

    fn names(root: &Path) -> Vec<(Option<String>, String)> {
        entries(&root.join("store"), Kind::User).unwrap().into_iter().map(|listed| (listed.group, listed.name)).collect()
    }

    #[test]
    fn the_window_lists_files_with_their_group_and_empty_groups_too() {
        let root = store_with("entries", &[(None, "a.png"), (Some("logos"), "l.svg")]);
        create_group(&root.join("store"), Kind::User, "empty").unwrap();
        assert_eq!(names(&root), [(None, "a.png".to_string()), (Some("logos".to_string()), "l.svg".to_string())]);
        assert!(entries(&root.join("store"), Kind::User).unwrap()[0].modified > 0);
        assert_eq!(groups(&root.join("store"), Kind::User).unwrap(), ["empty", "logos"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_file_moves_between_groups_without_writing_over_another() {
        let root = store_with("move", &[(None, "s.png"), (Some("carrousels"), "s.png")]);
        let store = root.join("store");
        let moved = move_to(&store, Kind::User, None, "s.png", Some("carrousels")).unwrap();
        assert_eq!(moved, store.join("user").join("carrousels").join("s-2.png"));
        assert_eq!(fs::read(store.join("user").join("carrousels").join("s.png")).unwrap(), b"s.png");
        let back = move_to(&store, Kind::User, Some("carrousels"), "s-2.png", None).unwrap();
        assert_eq!(back, store.join("user").join("s-2.png"));
        assert_eq!(move_to(&store, Kind::User, None, "s-2.png", None).unwrap(), back, "already there");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn names_from_the_page_cannot_leave_the_store() {
        let root = store_with("names", &[(None, "a.png")]);
        let store = root.join("store");
        for name in ["../a.png", "/etc/passwd", "x/a.png", "..", ""] {
            assert!(file_path(&store, Kind::User, None, name).is_err(), "{name:?}");
            assert!(delete(&store, Kind::User, None, name).is_err(), "{name:?}");
        }
        assert!(file_path(&store, Kind::User, Some(".."), "a.png").is_err());
        assert!(move_to(&store, Kind::User, None, "a.png", Some("../out")).is_err());
        assert!(file_path(&store, Kind::User, None, "a.png").is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn groups_are_renamed_and_a_removed_group_gives_its_files_back() {
        let root = store_with("rename", &[(Some("logos"), "l.svg"), (Some("logos"), "a.png"), (None, "a.png")]);
        let store = root.join("store");
        create_group(&store, Kind::User, "taken").unwrap();
        assert!(rename_group(&store, Kind::User, "logos", "taken").unwrap_err().contains("already exists"));
        rename_group(&store, Kind::User, "logos", "brand").unwrap();
        assert!(rename_group(&store, Kind::User, "logos", "other").is_err(), "gone");
        remove_group(&store, Kind::User, "brand").unwrap();
        assert_eq!(names(&root), [(None, "a-2.png".to_string()), (None, "a.png".to_string()), (None, "l.svg".to_string())]);
        assert_eq!(groups(&store, Kind::User).unwrap(), ["taken"]);
        delete(&store, Kind::User, None, "l.svg").unwrap();
        assert!(delete(&store, Kind::User, None, "l.svg").unwrap_err().contains("no longer there"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn kinds_are_read_from_their_names() {
        assert_eq!(Kind::parse("user"), Ok(Kind::User));
        assert_eq!(Kind::parse(" claude "), Ok(Kind::Claude));
        assert!(Kind::parse("llm").is_err());
    }
}
