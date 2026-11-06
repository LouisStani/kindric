//! What a new session of a project receives from the memory: its `CLAUDE.md` and `AGENTS.md`
//! written, its startup text and its skills passed as options. Nothing here stops a session
//! from starting: what goes wrong comes back as a warning.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::files::{create_private_dir, write_atomic};

use crate::generate::{self, Outcome};
use crate::memory::{Memory, WARNING_BYTES};

/// Skills appear in the session as `memory:<name>`.
const PLUGIN_NAME: &str = "memory";
const STARTUP_DIR: &str = "startup";
const PLUGINS_DIR: &str = "plugins";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Injection {
    pub args: Vec<OsString>,
    pub warnings: Vec<String>,
}

/// Prepares a session of `project`, which must have a zone, working in `directory`.
pub fn prepare(memory: &Memory, data_dir: &Path, project: &str, directory: &Path) -> io::Result<Injection> {
    let mut injection = Injection::default();
    injection.warnings.extend(write_instructions(memory, project, directory));

    let startup = write_startup_file(memory, data_dir, project)?;
    if let Some(file) = startup {
        injection.args.push("--append-system-prompt-file".into());
        injection.args.push(file.into_os_string());
    }

    let (plugin, skill_warnings) = build_plugin(memory, data_dir, project)?;
    injection.warnings.extend(skill_warnings);
    if let Some(dir) = plugin {
        injection.args.push("--plugin-dir".into());
        injection.args.push(dir.into_os_string());
    }

    let size = memory.startup_size(project, WARNING_BYTES)?;
    if size.over_warning {
        injection.warnings.push(format!(
            "memory opened at startup: {} KB, above the {} KB warning",
            size.bytes / 1024,
            WARNING_BYTES / 1024
        ));
    }
    Ok(injection)
}

fn write_instructions(memory: &Memory, project: &str, directory: &Path) -> Vec<String> {
    match generate::write_instructions(memory, project, directory) {
        Ok(outcomes) => outcomes.into_iter().filter_map(|(file, outcome)| outcome_warning(file, outcome)).collect(),
        Err(error) => vec![format!("CLAUDE.md and AGENTS.md not generated: {error}")],
    }
}

fn outcome_warning(file: &str, outcome: Outcome) -> Option<String> {
    match outcome {
        Outcome::Written | Outcome::Unchanged | Outcome::EmptySource => None,
        Outcome::EditedByHand => Some(format!("{file}: edited by hand, not regenerated")),
        Outcome::NotOurs => Some(format!("{file}: not written by kindric, left as it is")),
    }
}

/// The file is written even when empty, and never removed: a session started earlier with this
/// path reads it again when it resumes.
fn write_startup_file(memory: &Memory, data_dir: &Path, project: &str) -> io::Result<Option<PathBuf>> {
    let text = memory.startup_text(project)?;
    let dir = data_dir.join(STARTUP_DIR);
    create_private_dir(&dir)?;
    let file = dir.join(format!("{project}.md"));
    write_atomic(&file, &text)?;
    Ok((!text.is_empty()).then_some(file))
}

/// Rebuilds the plugin of `project` from its `skills.md`: a skill taken off the list goes away.
/// The folder stays even without skills, for the same reason as the startup file.
fn build_plugin(memory: &Memory, data_dir: &Path, project: &str) -> io::Result<(Option<PathBuf>, Vec<String>)> {
    let plugins = data_dir.join(PLUGINS_DIR);
    let building = plugins.join(format!(".{project}.{}.tmp", std::process::id()));
    remove_if_present(&building)?;
    let manifest = building.join(".claude-plugin");
    create_private_dir(&manifest)?;
    write_atomic(&manifest.join("plugin.json"), &format!("{{\"name\":\"{PLUGIN_NAME}\",\"version\":\"0.0.0\"}}\n"))?;

    let mut warnings = Vec::new();
    let mut copied = 0;
    for name in memory.skill_names(project)? {
        match memory.skill_dir(&name) {
            Some(skill) => {
                copy_dir(&skill, &building.join("skills").join(&name))?;
                copied += 1;
            }
            None => warnings.push(format!("skills.md: no skill named {name}")),
        }
    }

    let finished = plugins.join(project);
    remove_if_present(&finished)?;
    fs::rename(&building, &finished)?;
    Ok(((copied > 0).then_some(finished), warnings))
}

fn remove_if_present(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Copies files and folders; links are left out, so a skill cannot pull in a file from elsewhere.
fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    create_private_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Setup {
        root: PathBuf,
        memory: Memory,
        data: PathBuf,
        project: PathBuf,
    }

    /// A memory with a zone for project `p`, and an empty project folder.
    fn setup(label: &str) -> Setup {
        let root = std::env::temp_dir().join(format!("kindric-injection-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let memory = Memory::at(root.join("memory"));
        memory.ensure_project("p").unwrap();
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        let data = root.join("data");
        Setup { root, memory, data, project }
    }

    impl Setup {
        fn memory_file(&self, relative: &str, text: &str) {
            let path = self.root.join("memory").join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn prepare(&self) -> Injection {
            prepare(&self.memory, &self.data, "p", &self.project).unwrap()
        }

        fn plugin(&self) -> PathBuf {
            self.data.join("plugins").join("p")
        }

        fn finish(self) {
            fs::remove_dir_all(self.root).unwrap();
        }
    }

    #[test]
    fn a_complete_zone_gives_the_startup_file_the_skills_and_the_instructions() {
        let s = setup("complete");
        s.memory_file("global/rules.md", "RULE");
        s.memory_file("perso/life.md", "PRIVATE");
        s.memory_file("projets/p/claude.md", "PROJECT RULES");
        s.memory_file("projets/p/skills.md", "rust\n");
        s.memory_file("skills/rust/SKILL.md", "---\nname: rust\ndescription: d\n---\nbody");

        let injection = s.prepare();

        let startup = s.data.join("startup").join("p.md");
        let expected: Vec<OsString> = vec![
            "--append-system-prompt-file".into(),
            startup.clone().into_os_string(),
            "--plugin-dir".into(),
            s.plugin().into_os_string(),
        ];
        assert_eq!(injection, Injection { args: expected, warnings: Vec::new() });
        let text = fs::read_to_string(startup).unwrap();
        assert_eq!(text, "RULE");
        assert!(fs::read_to_string(s.project.join("CLAUDE.md")).unwrap().ends_with("PROJECT RULES\n"));
        assert!(s.project.join("AGENTS.md").is_file());
        assert!(s.plugin().join(".claude-plugin/plugin.json").is_file());
        assert!(s.plugin().join("skills/rust/SKILL.md").is_file());
        s.finish();
    }

    #[test]
    fn an_empty_zone_passes_no_option_and_writes_nothing_in_the_project() {
        let s = setup("empty");
        assert_eq!(s.prepare(), Injection::default());
        assert_eq!(fs::read_dir(&s.project).unwrap().count(), 0);
        s.finish();
    }

    #[test]
    fn the_plugin_holds_only_the_skills_of_the_list() {
        let s = setup("listed");
        s.memory_file("projets/p/skills.md", "- rust\n");
        s.memory_file("skills/rust/SKILL.md", "r");
        s.memory_file("skills/rust/more/notes.md", "n");
        s.memory_file("skills/front/SKILL.md", "f");
        s.prepare();
        assert!(s.plugin().join("skills/rust/more/notes.md").is_file());
        assert!(!s.plugin().join("skills/front").exists());
        s.finish();
    }

    #[test]
    fn a_skill_taken_off_the_list_goes_away_at_the_next_launch() {
        let s = setup("removed");
        s.memory_file("projets/p/skills.md", "rust\nfront\n");
        s.memory_file("skills/rust/SKILL.md", "r");
        s.memory_file("skills/front/SKILL.md", "f");
        s.prepare();
        assert!(s.plugin().join("skills/front/SKILL.md").is_file());

        s.memory_file("projets/p/skills.md", "rust\n");
        s.prepare();
        assert!(!s.plugin().join("skills/front").exists());
        assert!(s.plugin().join("skills/rust/SKILL.md").is_file());
        assert_eq!(fs::read_dir(s.data.join("plugins")).unwrap().count(), 1, "no temporary folder left");
        s.finish();
    }

    #[test]
    fn with_no_skill_left_the_option_goes_but_the_folder_stays_for_older_sessions() {
        let s = setup("no-skill");
        s.memory_file("projets/p/skills.md", "rust\n");
        s.memory_file("skills/rust/SKILL.md", "r");
        s.prepare();
        s.memory_file("projets/p/skills.md", "");
        let injection = s.prepare();
        assert!(injection.args.is_empty());
        assert!(s.plugin().join(".claude-plugin/plugin.json").is_file());
        assert!(!s.plugin().join("skills").exists());
        s.finish();
    }

    #[test]
    fn a_skill_that_is_missing_or_leaves_its_folder_is_a_warning_and_copies_nothing() {
        let s = setup("bad-names");
        s.memory_file("perso/SKILL.md", "PRIVATE");
        s.memory_file("projets/p/skills.md", "../perso\nmissing\n");
        let injection = s.prepare();
        assert_eq!(injection.warnings, ["skills.md: no skill named ../perso", "skills.md: no skill named missing"]);
        assert!(injection.args.is_empty());
        assert!(!s.plugin().join("skills").exists());
        s.finish();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_inside_a_skill_is_not_followed() {
        let s = setup("skill-link");
        s.memory_file("perso/life.md", "PRIVATE");
        s.memory_file("projets/p/skills.md", "rust\n");
        s.memory_file("skills/rust/SKILL.md", "r");
        std::os::unix::fs::symlink(s.root.join("memory/perso/life.md"), s.root.join("memory/skills/rust/life.md")).unwrap();
        s.prepare();
        assert!(s.plugin().join("skills/rust/SKILL.md").is_file());
        assert!(fs::symlink_metadata(s.plugin().join("skills/rust/life.md")).is_err());
        s.finish();
    }

    #[test]
    fn a_refused_file_is_a_warning_and_the_rest_still_happens() {
        let s = setup("refused");
        s.memory_file("global/rules.md", "RULE");
        s.memory_file("projets/p/claude.md", "PROJECT RULES");
        fs::write(s.project.join("CLAUDE.md"), "someone else's\n").unwrap();
        let injection = s.prepare();
        assert_eq!(injection.warnings, ["CLAUDE.md: not written by kindric, left as it is"]);
        assert_eq!(fs::read_to_string(s.project.join("CLAUDE.md")).unwrap(), "someone else's\n");
        assert!(s.project.join("AGENTS.md").is_file());
        assert_eq!(injection.args.len(), 2);
        s.finish();
    }

    #[test]
    fn a_file_edited_by_hand_is_a_warning() {
        assert_eq!(outcome_warning("AGENTS.md", Outcome::EditedByHand).unwrap(), "AGENTS.md: edited by hand, not regenerated");
        assert_eq!(outcome_warning("AGENTS.md", Outcome::Written), None);
        assert_eq!(outcome_warning("AGENTS.md", Outcome::Unchanged), None);
        assert_eq!(outcome_warning("AGENTS.md", Outcome::EmptySource), None);
    }

    #[test]
    fn a_broken_header_is_a_warning_and_the_session_still_gets_the_rest() {
        let s = setup("header");
        s.memory_file("global/rules.md", "RULE");
        s.memory_file("projets/p/claude.md", "---\naccess: perso\n");
        let injection = s.prepare();
        assert_eq!(injection.warnings.len(), 1);
        assert!(injection.warnings[0].contains("not generated"));
        assert_eq!(injection.args.len(), 2);
        assert_eq!(fs::read_dir(&s.project).unwrap().count(), 0);
        s.finish();
    }

    #[test]
    fn too_much_at_startup_is_a_warning_and_nothing_is_cut() {
        let s = setup("size");
        let big = "x".repeat(usize::try_from(WARNING_BYTES).unwrap() + 1);
        s.memory_file("global/rules.md", &big);
        let injection = s.prepare();
        assert_eq!(injection.warnings, ["memory opened at startup: 30 KB, above the 30 KB warning"]);
        assert_eq!(fs::read_to_string(s.data.join("startup/p.md")).unwrap(), big);
        s.finish();
    }
}
