//! The memory atlas (decisions 270-280): the notes of `~/.kindric/memory` grouped for the app's
//! map. Each project's sun is its `status/` notes; every other note is a planet, in parts: its
//! knowledge, its lessons, and its decisions in runs of `PART_SIZE`, in order. A project with more
//! than `GALAXY_FROM` notes is drawn as a galaxy of those parts. A center holds what belongs to no
//! project. Each signal is read from the files alone: a status older than the last decision, a
//! lesson learned more than once, a note its zone's `INDEX.md` does not mention.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use kindric_accounts::files::files_under;
use serde::Serialize;

const PROJECTS: &str = "projets";
const INDEX: &str = "INDEX.md";
/// The files of a zone that are not notes: what sessions are given, and the index itself.
const ZONE_FILES: [&str; 3] = ["claude.md", "skills.md", INDEX];
const PLANETS: [&str; 3] = ["decisions", "knowledge", "lessons"];
const STATUS: &str = "status";
/// The folders of the center, in the order drawn.
const CENTER: [&str; 4] = ["global", "setup", "perso", "skills"];
/// Decisions are drawn in runs of this many, oldest first.
pub const PART_SIZE: usize = 24;
/// A project with more notes than this is a galaxy rather than a single system.
pub const GALAXY_FROM: usize = 30;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Note {
    /// From the memory's root, with `/` on every system.
    pub path: String,
    /// The header's `description`, else the file name.
    pub description: String,
    /// `YYYY-MM-DD`: the date the file's name starts with (a decision), else the header's `updated`.
    pub date: Option<String>,
    /// Neither the note nor its folder is in its zone's `INDEX.md` (a project's notes only).
    pub lost: bool,
    /// The file's length, which sets the planet's size.
    pub bytes: u64,
}

/// Notes drawn together around one star.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Part {
    /// "decisions", "knowledge", "lessons", "others", or a folder of the center.
    pub kind: String,
    /// Its name on the map: the kind, or a run of decisions, `001–024 · 15/10–23/10`.
    pub label: String,
    pub notes: Vec<Note>,
    /// Lessons only: the lines that carry more than one date, a correction given again.
    pub repeated: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Sun {
    pub notes: Vec<Note>,
    /// The date of the newest status note.
    pub updated: Option<String>,
    /// The date of the newest decision.
    pub last_decision: Option<String>,
    /// The newest status is older than the newest decision, or there is none while decisions exist.
    pub stale: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct System {
    pub name: String,
    /// None for the center, which has no status.
    pub sun: Option<Sun>,
    /// Knowledge, lessons, notes outside the usual folders, then the runs of decisions; only
    /// those that hold notes.
    pub parts: Vec<Part>,
    /// More than `GALAXY_FROM` notes: drawn as a galaxy of its parts.
    pub galaxy: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Atlas {
    pub center: System,
    pub systems: Vec<System>,
    /// What could not be read; the rest of the atlas is still drawn.
    pub warnings: Vec<String>,
}

/// The memory's folder: `~/.kindric/memory`, or the one `KINDRIC_MEMORY_DIR` names.
pub fn default_root() -> Result<PathBuf, String> {
    kindric_accounts::memory::Memory::default_root().map_err(|error| format!("cannot find the memory: {error}"))
}

/// Reads the whole memory under `root`. A missing root gives an empty atlas.
pub fn read(root: &Path) -> Atlas {
    let mut warnings = Vec::new();
    let center = read_center(root, &mut warnings);
    let mut systems = Vec::new();
    match fs::read_dir(root.join(PROJECTS)) {
        Ok(entries) => {
            let mut zones: Vec<PathBuf> = entries.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.is_dir()).collect();
            zones.sort_by_key(|zone| zone.file_name().map(|name| name.to_string_lossy().to_lowercase()));
            for zone in zones {
                systems.push(read_zone(root, &zone, &mut warnings));
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {},
        Err(error) => warnings.push(format!("{PROJECTS}: {error}")),
    }
    Atlas { center, systems, warnings }
}

fn read_center(root: &Path, warnings: &mut Vec<String>) -> System {
    let parts = CENTER
        .iter()
        .map(|name| Part { kind: (*name).to_owned(), label: (*name).to_owned(), notes: notes_in(root, &root.join(name), None, warnings), repeated: 0 })
        .filter(|part| !part.notes.is_empty())
        .collect();
    System { name: String::new(), sun: None, parts, galaxy: false }
}

fn read_zone(root: &Path, zone: &Path, warnings: &mut Vec<String>) -> System {
    let name = zone.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let index = fs::read_to_string(zone.join(INDEX)).unwrap_or_default();
    let indexed = Some((zone, index.as_str()));
    let status = notes_in(root, &zone.join(STATUS), indexed, warnings);
    let mut folders = PLANETS.iter().map(|planet| notes_in(root, &zone.join(planet), indexed, warnings));
    let (decisions, knowledge, lessons) = (folders.next().unwrap_or_default(), folders.next().unwrap_or_default(), folders.next().unwrap_or_default());
    let last_decision = decisions.iter().filter_map(|note| note.date.clone()).max();
    let updated = status.iter().filter_map(|note| note.date.clone()).max();
    let stale = match (&updated, &last_decision) {
        (Some(updated), Some(decision)) => updated < decision,
        (None, Some(_)) => true,
        _ => false,
    };
    let others = other_notes(root, zone, indexed, warnings);
    let repeated = repeated_lessons(&zone.join("lessons"));
    let mut parts: Vec<Part> = [("knowledge", knowledge, 0), ("lessons", lessons, repeated), ("others", others, 0)]
        .into_iter()
        .filter(|(_, notes, _)| !notes.is_empty())
        .map(|(kind, notes, repeated)| Part { kind: kind.to_owned(), label: kind.to_owned(), notes, repeated })
        .collect();
    parts.extend(decision_runs(decisions));
    let count = status.len() + parts.iter().map(|part| part.notes.len()).sum::<usize>();
    System { name, sun: Some(Sun { notes: status, updated, last_decision, stale }), parts, galaxy: count > GALAXY_FROM }
}

/// The decisions in runs of `PART_SIZE`, oldest first, each named by its numbers and its days.
fn decision_runs(mut decisions: Vec<Note>) -> Vec<Part> {
    decisions.sort_by(|one, other| one.path.cmp(&other.path));
    let mut runs = Vec::new();
    while !decisions.is_empty() {
        let rest = decisions.split_off(decisions.len().min(PART_SIZE));
        let run = std::mem::replace(&mut decisions, rest);
        let first = run.first().map(decision_mark).unwrap_or_default();
        let last = run.last().map(decision_mark).unwrap_or_default();
        let numbers = if first.0 == last.0 { first.0.clone() } else { format!("{}–{}", first.0, last.0) };
        let days = if first.1 == last.1 { first.1 } else { format!("{}–{}", first.1, last.1) };
        let label = if days.is_empty() { numbers } else { format!("{numbers} · {days}") };
        runs.push(Part { kind: "decisions".to_owned(), label, notes: run, repeated: 0 });
    }
    runs
}

/// `…/2026-10-15-002.md` → (`002`, `15/10`); a name without that shape gives itself and no day.
fn decision_mark(note: &Note) -> (String, String) {
    let name = note.path.rsplit('/').next().unwrap_or_default();
    let stem = name.strip_suffix(".md").unwrap_or(name);
    let number = stem.get(11..).filter(|_| leading_date(stem).is_some()).unwrap_or(stem).to_owned();
    let day = note.date.as_deref().filter(|date| date.len() == 10).map(|date| format!("{}/{}", &date[8..10], &date[5..7])).unwrap_or_default();
    (number, day)
}

/// The notes of a zone that are in none of its usual folders.
fn other_notes(root: &Path, zone: &Path, indexed: Option<(&Path, &str)>, warnings: &mut Vec<String>) -> Vec<Note> {
    let base = relative(root, zone);
    let usual: Vec<String> = PLANETS.iter().chain(&[STATUS]).map(|folder| format!("{base}/{folder}/")).collect();
    let zone_files: Vec<String> = ZONE_FILES.iter().map(|file| format!("{base}/{file}")).collect();
    markdown_under(root, zone, warnings)
        .into_iter()
        .filter(|path| {
            let path = relative(root, path);
            !usual.iter().any(|folder| path.starts_with(folder.as_str())) && !zone_files.contains(&path)
        })
        .map(|path| note(root, &path, indexed))
        .collect()
}

/// Every note under `folder`, sorted by path.
fn notes_in(root: &Path, folder: &Path, indexed: Option<(&Path, &str)>, warnings: &mut Vec<String>) -> Vec<Note> {
    markdown_under(root, folder, warnings).iter().map(|path| note(root, path, indexed)).collect()
}

/// Every `.md` under `folder`, sorted; a folder that cannot be read is a warning.
fn markdown_under(root: &Path, folder: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    match files_under(folder) {
        Ok(files) => files.into_iter().filter(|path| path.extension().is_some_and(|extension| extension == "md")).collect(),
        Err(error) => {
            warnings.push(format!("{}: {error}", relative(root, folder)));
            Vec::new()
        },
    }
}

fn note(root: &Path, path: &Path, indexed: Option<(&Path, &str)>) -> Note {
    let text = fs::read_to_string(path).unwrap_or_default();
    let header = header_of(&text);
    let file_name = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let description = field(header, "description").unwrap_or_else(|| file_name.clone());
    // A decision's name holds the day it was taken; its header may be the day it was converted.
    let date = leading_date(&file_name).or_else(|| field(header, "updated"));
    let lost = indexed.is_some_and(|(zone, index)| !is_indexed(&relative(zone, path), index));
    Note { path: relative(root, path), description, date, lost, bytes: text.len() as u64 }
}

/// Whether the index names the note (`knowledge/tests.md`) or its folder alone (`decisions/ :`);
/// `knowledge/tests.md` does not name the folder `knowledge/`.
fn is_indexed(from_zone: &str, index: &str) -> bool {
    if index.contains(from_zone) {
        return true;
    }
    // `- knowledge/github.md, x.md, threads.md : …` names x.md too.
    if let Some((folder, file)) = from_zone.rsplit_once('/') {
        let folder = format!("{folder}/");
        if index.lines().any(|line| line.contains(&folder) && names_file(line, file)) {
            return true;
        }
    }
    let mut folder = String::new();
    for part in from_zone.split('/').collect::<Vec<_>>().split_last().map_or(&[][..], |(_, folders)| folders) {
        folder.push_str(part);
        folder.push('/');
        if names_alone(index, &folder) {
            return true;
        }
    }
    false
}

/// Whether `file` appears in `line` as a whole name: `x.md` is not in `index.md`.
fn names_file(line: &str, file: &str) -> bool {
    line.match_indices(file).any(|(start, _)| {
        line[..start].chars().next_back().is_none_or(|before| !(before.is_alphanumeric() || "-_.".contains(before)))
    })
}

/// Whether `folder` appears in `index` with no file name right after it.
fn names_alone(index: &str, folder: &str) -> bool {
    index.match_indices(folder).any(|(start, _)| {
        index[start + folder.len()..].chars().next().is_none_or(|next| !(next.is_alphanumeric() || "-_.".contains(next)))
    })
}

/// The header between the opening `---` and the next one; empty without one.
fn header_of(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else {
        return "";
    };
    rest.find("\n---").map_or("", |end| &rest[..end])
}

fn field(header: &str, name: &str) -> Option<String> {
    header.lines().find_map(|line| {
        let value = line.strip_prefix(name)?.strip_prefix(':')?.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

/// `2026-11-12-266.md` → `2026-11-12`.
fn leading_date(name: &str) -> Option<String> {
    let date = name.get(..10)?;
    is_date(date).then(|| date.to_owned())
}

fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| if index == 4 || index == 7 { *byte == b'-' } else { byte.is_ascii_digit() })
}

/// A lesson is `- 2026-10-29 subject : what happened`; given again, it gets one more date before
/// the ` : ` (`- 2026-11-09 README, 2026-11-11 … : …`). Dates after it are references, unless
/// the line says « Récidive ».
fn repeated_lessons(folder: &Path) -> usize {
    let Ok(files) = files_under(folder) else {
        return 0;
    };
    files
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .map(|text| text.lines().filter(|line| is_repeated_lesson(line)).count())
        .sum()
}

/// A lesson given again: one more date before its subject, or written as « Récidive du … ».
fn is_repeated_lesson(line: &str) -> bool {
    line.starts_with("- ") && (dates_before_subject(line) > 1 || line.to_lowercase().contains("récidive"))
}

fn dates_before_subject(line: &str) -> usize {
    let head = line.find(" : ").map_or(line, |end| &line[..end]);
    (0..head.len().saturating_sub(9)).filter(|start| head.get(*start..start + 10).is_some_and(is_date)).count()
}

/// `path` from `base`, with `/` between its parts on every system.
fn relative(base: &Path, path: &Path) -> String {
    let inside = path.strip_prefix(base).unwrap_or(path);
    inside.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// The paths of the notes whose text holds every word of `query`, case ignored.
pub fn search(root: &Path, query: &str) -> Vec<String> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let Ok(files) = files_under(root) else {
        return Vec::new();
    };
    files
        .iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .filter(|path| fs::read_to_string(path).is_ok_and(|text| {
            let text = text.to_lowercase();
            words.iter().all(|word| text.contains(word.as_str()))
        }))
        .map(|path| relative(root, path))
        .collect()
}

/// The text of the note at `path`, from the memory's root. Only a `.md` file inside the root.
pub fn read_note(root: &Path, path: &str) -> Result<String, String> {
    let inside = Path::new(path);
    let plain = inside.components().all(|part| matches!(part, Component::Normal(_)));
    if !plain || inside.extension().is_none_or(|extension| extension != "md") {
        return Err(format!("{path}: not a note of the memory"));
    }
    let full = root.join(inside);
    let real_root = fs::canonicalize(root).map_err(|error| format!("{}: {error}", root.display()))?;
    let real = fs::canonicalize(&full).map_err(|error| format!("{path}: {error}"))?;
    if !real.starts_with(&real_root) {
        return Err(format!("{path}: outside the memory"));
    }
    fs::read_to_string(&real).map_err(|error| format!("{path}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-atlas-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(root: &Path, path: &str, text: &str) {
        let file = root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }

    fn header(description: &str, updated: &str) -> String {
        format!("---\ndescription: {description}\nauthor: claude\nupdated: {updated}\n---\n\n- a fact\n")
    }

    #[test]
    fn a_status_older_than_the_last_decision_is_stale() {
        let root = scratch("stale");
        write(&root, "projets/api/status/api.md", &header("where it stands", "2026-11-06"));
        write(&root, "projets/api/decisions/2026-11-12-001.md", "no header");
        write(&root, "projets/web/status/web.md", &header("where it stands", "2026-11-12"));
        write(&root, "projets/web/decisions/2026-11-12-001.md", "no header");
        write(&root, "projets/new/decisions/2026-11-01-001.md", "no header");
        // Converted on 06/11, taken on 01/11: the status of 03/11 came after it.
        write(&root, "projets/old/status/old.md", &header("where it stands", "2026-11-03"));
        write(&root, "projets/old/decisions/2026-11-01-001.md", &header("a choice", "2026-11-06"));
        let atlas = read(&root);
        let sun = |name: &str| atlas.systems.iter().find(|system| system.name == name).unwrap().sun.as_ref().unwrap();
        assert!(sun("api").stale, "status of 06/11, decision of 12/11");
        assert_eq!(sun("api").updated.as_deref(), Some("2026-11-06"));
        assert_eq!(sun("api").last_decision.as_deref(), Some("2026-11-12"));
        assert!(!sun("web").stale, "the same day is not stale");
        assert!(sun("new").stale, "decisions without any status");
        assert!(!sun("old").stale, "a decision's date is in its name");
        assert_eq!(sun("old").last_decision.as_deref(), Some("2026-11-01"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_newest_status_note_sets_the_sun() {
        let root = scratch("newest");
        write(&root, "projets/api/status/old-campaign.md", &header("done long ago", "2026-10-01"));
        write(&root, "projets/api/status/api.md", &header("where it stands", "2026-11-12"));
        write(&root, "projets/api/decisions/2026-11-10-001.md", "no header");
        let atlas = read(&root);
        assert!(!atlas.systems[0].sun.as_ref().unwrap().stale, "an old finished campaign does not pale the sun");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_dates_before_the_subject_make_a_lesson_repeated() {
        assert_eq!(dates_before_subject("- 2026-10-29 menu : radio buttons (voir 2026-11-03)"), 1);
        assert_eq!(dates_before_subject("- 2026-11-09 README, 2026-11-11 (fusion), 2026-11-12 commit : git refused"), 3);
        let root = scratch("lessons");
        let lessons = "---\ndescription: corrections\n---\n\n- 2026-10-29 a : once (voir 2026-11-03)\n- 2026-11-01 b, 2026-11-05 b : twice\n- 2026-11-02 c : once\n- 2026-11-05 d : again. Récidive du 2026-11-02 (one point at a time)\n";
        write(&root, "projets/api/lessons/lecons.md", lessons);
        let atlas = read(&root);
        let part = atlas.systems[0].parts.iter().find(|part| part.kind == "lessons").unwrap();
        assert_eq!(part.repeated, 2);
        assert_eq!(part.notes[0].description, "corrections");
        assert_eq!(part.notes[0].bytes, lessons.len() as u64);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn decisions_come_in_runs_named_by_their_numbers_and_days() {
        let root = scratch("runs");
        write(&root, "projets/big/status/big.md", &header("where", "2026-11-12"));
        for number in 1..=50 {
            let day = if number <= 24 { "2026-10-15" } else { "2026-10-23" };
            write(&root, &format!("projets/big/decisions/{day}-{number:03}.md"), "a choice");
        }
        write(&root, "projets/small/decisions/2026-11-01-001.md", "a choice");
        let atlas = read(&root);
        let big = &atlas.systems[0];
        let labels: Vec<&str> = big.parts.iter().map(|part| part.label.as_str()).collect();
        assert_eq!(labels, ["001–024 · 15/10", "025–048 · 23/10", "049–050 · 23/10"]);
        assert_eq!(big.parts[0].notes.len(), PART_SIZE);
        assert!(big.galaxy, "51 notes");
        assert!(!atlas.systems[1].galaxy);
        assert_eq!(atlas.systems[1].parts[0].label, "001 · 01/11");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_note_its_index_does_not_mention_is_lost() {
        let root = scratch("lost");
        write(&root, "projets/api/INDEX.md", "- status/api.md : where\n- decisions/ : 2 decisions\n- knowledge/tests.md, x.md : tests\n");
        write(&root, "projets/api/knowledge/x.md", &header("named after its folder's line", "2026-11-12"));
        write(&root, "projets/api/knowledge/max.md", &header("max.md is not x.md", "2026-11-12"));
        write(&root, "projets/api/status/api.md", &header("where", "2026-11-12"));
        write(&root, "projets/api/decisions/2026-11-12-001.md", "no header");
        write(&root, "projets/api/knowledge/tests.md", &header("tests", "2026-11-12"));
        write(&root, "projets/api/knowledge/forgotten.md", &header("forgotten", "2026-11-12"));
        write(&root, "projets/api/claude.md", "given to sessions");
        let atlas = read(&root);
        let system = &atlas.systems[0];
        let lost: Vec<&str> = system
            .parts
            .iter()
            .flat_map(|part| &part.notes)
            .chain(&system.sun.as_ref().unwrap().notes)
            .filter(|note| note.lost)
            .map(|note| note.path.as_str())
            .collect();
        assert_eq!(lost, ["projets/api/knowledge/forgotten.md", "projets/api/knowledge/max.md"]);
        assert!(!system.parts.iter().any(|part| part.kind == "others"), "claude.md and INDEX.md are not notes: {:?}", system.parts);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_center_holds_what_belongs_to_no_project() {
        let root = scratch("center");
        write(&root, "global/regles.md", &header("rules", "2026-11-01"));
        write(&root, "skills/rust/SKILL.md", &header("rust pitfalls", "2026-11-02"));
        write(&root, "projets/api/notes/idea.md", &header("an idea", "2026-11-03"));
        let atlas = read(&root);
        let names: Vec<&str> = atlas.center.parts.iter().map(|part| part.label.as_str()).collect();
        assert_eq!(names, ["global", "skills"], "empty folders are left out");
        assert_eq!(atlas.center.parts[0].notes[0].description, "rules");
        assert_eq!(atlas.center.parts[1].notes[0].path, "skills/rust/SKILL.md");
        assert!(!atlas.center.parts[0].notes[0].lost, "the center has no index to miss");
        let others = atlas.systems[0].parts.iter().find(|part| part.kind == "others").unwrap();
        assert_eq!(others.notes[0].path, "projets/api/notes/idea.md", "a project's own folder is kept, not dropped");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_memory_is_an_empty_atlas() {
        let atlas = read(Path::new("/no/such/memory"));
        assert!(atlas.systems.is_empty());
        assert!(atlas.warnings.is_empty(), "{:?}", atlas.warnings);
    }

    #[test]
    fn search_needs_every_word() {
        let root = scratch("search");
        write(&root, "projets/api/decisions/2026-11-12-001.md", "Le thème Paper de Claude Code");
        write(&root, "projets/api/decisions/2026-11-12-002.md", "le thème ember");
        assert_eq!(search(&root, "THÈME paper"), ["projets/api/decisions/2026-11-12-001.md"]);
        assert_eq!(search(&root, "thème").len(), 2);
        assert!(search(&root, "  ").is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_a_note_inside_the_memory_is_read() {
        let root = scratch("read");
        write(&root, "global/regles.md", "the rules");
        write(&root, "global/secret.txt", "not a note");
        assert_eq!(read_note(&root, "global/regles.md").unwrap(), "the rules");
        assert!(read_note(&root, "../outside.md").is_err());
        assert!(read_note(&root, "/etc/passwd.md").is_err());
        assert!(read_note(&root, "global/secret.txt").is_err());
        assert!(read_note(&root, "global/missing.md").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
