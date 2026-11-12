//! Projects: a folder each, with a tab of its own. A session belongs to the project whose
//! folder holds its working directory. The list is kept in `projects.json` (see `store`).

use std::path::{Path, PathBuf};

use kindric_core::claude_theme;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub directory: PathBuf,
    /// The account of the last session started here with Ctrl+N, which Ctrl+N offers first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The theme its sessions are shown in, chosen with Ctrl+Alt+E; none: kindric's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

impl Project {
    /// Named after its folder.
    pub fn new(directory: PathBuf) -> Self {
        let name = directory.file_name().map_or_else(|| directory.display().to_string(), |name| name.to_string_lossy().into_owned());
        Self { name, directory, account: None, theme: None }
    }
}

/// The Claude Code theme of a session working in `directory`: its project's, else kindric's. Each
/// project has one, whether it has a theme of its own or not, so a theme chosen later reaches
/// the sessions already running.
pub fn claude_theme_of(directory: &Path, projects: &[Project]) -> String {
    owner(directory, projects.iter().map(|project| (project, project)))
        .map_or_else(|| claude_theme::NAME.to_string(), |project| claude_theme::project_theme(&project.directory))
}

/// The project holding `directory`, the deepest one when projects are nested. Paths are
/// compared by component: `~/Dev/api2` is not in `~/Dev/api`.
pub fn owner<'a, T>(directory: &Path, projects: impl IntoIterator<Item = (T, &'a Project)>) -> Option<T> {
    projects
        .into_iter()
        .filter(|(_, project)| directory.starts_with(&project.directory))
        .max_by_key(|(_, project)| project.directory.components().count())
        .map(|(id, _)| id)
}

/// What a project tab's tooltip says about the memory its sessions receive, from
/// `kindric_core::launcher::memory_zone`.
pub fn memory_label(found: Result<(Option<String>, Vec<String>), String>) -> String {
    let (zone, warnings) = match found {
        Ok(found) => found,
        Err(error) => return format!("memory not read: {error}"),
    };
    let first = zone.map_or_else(|| "memory: no zone".to_string(), |zone| format!("memory: zone {zone}"));
    std::iter::once(first).chain(warnings).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tooltip_names_the_zone_and_what_could_not_be_read() {
        assert_eq!(memory_label(Ok((Some("kindric".into()), Vec::new()))), "memory: zone kindric");
        assert_eq!(memory_label(Ok((None, Vec::new()))), "memory: no zone");
        let broken = vec!["projets/api/claude.md: the header is not closed by a --- line".to_string()];
        assert_eq!(
            memory_label(Ok((None, broken))),
            "memory: no zone\nprojets/api/claude.md: the header is not closed by a --- line"
        );
        assert_eq!(memory_label(Err("HOME is not set".into())), "memory not read: HOME is not set");
    }

    fn project(directory: &str) -> Project {
        Project::new(PathBuf::from(directory))
    }

    fn owner_of(directory: &str, projects: &[Project]) -> Option<usize> {
        owner(Path::new(directory), projects.iter().enumerate())
    }

    #[test]
    fn a_project_is_named_after_its_folder() {
        assert_eq!(project("/home/u/Dev/api").name, "api");
    }

    #[test]
    fn a_session_belongs_to_the_project_holding_its_directory() {
        let projects = [project("/home/u/Dev/api"), project("/home/u/Dev/blog")];
        assert_eq!(owner_of("/home/u/Dev/api", &projects), Some(0));
        assert_eq!(owner_of("/home/u/Dev/blog/src/posts", &projects), Some(1));
        assert_eq!(owner_of("/home/u", &projects), None);
        assert_eq!(owner_of("/tmp", &projects), None);
    }

    #[test]
    fn projects_written_before_accounts_still_load_and_no_account_writes_nothing() {
        let loaded: Vec<Project> = serde_json::from_str(r#"[{"name":"api","directory":"/home/u/Dev/api"}]"#).unwrap();
        assert_eq!(loaded, [project("/home/u/Dev/api")]);
        assert_eq!(serde_json::to_string(&loaded[0]).unwrap(), r#"{"name":"api","directory":"/home/u/Dev/api"}"#);
    }

    #[test]
    fn a_theme_is_kept_and_none_writes_nothing() {
        let mut api = project("/home/u/Dev/api");
        api.theme = Some("paper".into());
        let text = serde_json::to_string(&api).unwrap();
        assert_eq!(text, r#"{"name":"api","directory":"/home/u/Dev/api","theme":"paper"}"#);
        assert_eq!(serde_json::from_str::<Project>(&text).unwrap(), api);
    }

    #[test]
    fn a_session_draws_with_the_claude_code_theme_of_its_project() {
        let projects = [project("/home/u/Dev/api"), project("/home/u/Dev/blog")];
        let api = claude_theme::project_theme(Path::new("/home/u/Dev/api"));
        assert_eq!(claude_theme_of(Path::new("/home/u/Dev/api/src"), &projects), api);
        assert_ne!(claude_theme_of(Path::new("/home/u/Dev/blog"), &projects), api);
        assert_eq!(claude_theme_of(Path::new("/tmp"), &projects), claude_theme::NAME, "no project: kindric's");
    }

    #[test]
    fn a_folder_sharing_a_prefix_is_not_inside() {
        let projects = [project("/home/u/Dev/api")];
        assert_eq!(owner_of("/home/u/Dev/api2", &projects), None);
    }

    #[test]
    fn nested_projects_give_the_session_to_the_deepest() {
        let projects = [project("/home/u/Dev"), project("/home/u/Dev/api"), project("/home/u")];
        assert_eq!(owner_of("/home/u/Dev/api/src", &projects), Some(1));
        assert_eq!(owner_of("/home/u/Dev/blog", &projects), Some(0));
        assert_eq!(owner_of("/home/u/Music", &projects), Some(2));
    }
}
