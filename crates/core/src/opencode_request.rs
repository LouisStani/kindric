//! The permissions and questions OpenCode sessions wait for, as requests of kindric's page
//! (decision 441). Nothing is written for them: each reading of the page's requests asks the
//! service of the session's account, and an answer goes back to it. Answered in OpenCode's TUI, a
//! request is gone at the next reading. Read on OpenCode 2.3.2: a permission is
//! `{id "per_…", action, resources}`, a question of the model a form `{id "frm_…", metadata.kind
//! "question", fields [{key, title, description, type "string"|"multiselect", options}]}`.

use std::collections::BTreeMap;
use std::io;

use kindric_accounts::Paths;
use kindric_accounts::agent::Agent;
use serde_json::{Value, json};

use crate::accounts::{SessionInfo, SessionState};
use crate::opencode::Service;
use crate::overview::{Answer, Body, Request};

/// The start of the id of such a request: `opencode:<account>:<session>:<per_… or frm_…>`.
const PREFIX: &str = "opencode:";
const PERMISSION: &str = "per_";
const FORM: &str = "frm_";
/// What `waiting_for` says of a session waiting for the user (see `accounts::opencode_session`).
const WAITING_FOR: [&str; 2] = ["permission prompt", "input needed"];
/// The kind of form a question of the model opens.
const QUESTION: &str = "question";
const NO_LONGER_OPEN: &str = "this question is no longer open: it was answered in the session";

fn request_id(account: &str, session: &str, item: &str) -> String {
    format!("{PREFIX}{account}:{session}:{item}")
}

/// The account, session and item of a request id made by `request_id`; `None` for any other.
fn parse_id(id: &str) -> Option<(&str, &str, &str)> {
    let mut parts = id.strip_prefix(PREFIX)?.splitn(3, ':');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(account), Some(session), Some(item))
            if !account.is_empty() && !session.is_empty() && (item.starts_with(PERMISSION) || item.starts_with(FORM)) =>
        {
            Some((account, session, item))
        },
        _ => None,
    }
}

/// Whether `id` names a permission or a question of an OpenCode session.
pub fn is_opencode_request(id: &str) -> bool {
    parse_id(id).is_some()
}

/// Whether `session` runs OpenCode and waits for a permission or an answer.
pub fn asks(session: &SessionInfo) -> bool {
    session.agent == Agent::OpenCode
        && session.state == SessionState::Waiting
        && session.waiting_for.as_deref().is_some_and(|waiting| WAITING_FOR.contains(&waiting))
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// The page's form of a permission: a command shows as Claude Code's Bash does, a change of files
/// as its Edit, any other action under its own name with what it reaches.
fn permission_body(permission: &Value) -> Body {
    let action = text(permission, "action");
    let resources: Vec<String> =
        permission.get("resources").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
    let joined = resources.join("\n");
    match action.as_str() {
        "shell" => Body::Permission { tool: "Bash".into(), input: json!({ "command": joined }) },
        "edit" => Body::Permission { tool: "Edit".into(), input: json!({ "description": format!("change {}", resources.join(", ")) }) },
        _ => Body::Permission { tool: action, input: json!({ "description": joined }) },
    }
}

/// The questions of a form in the form of Claude Code's AskUserQuestion, which the page shows;
/// `None` for a form that is not a question of the model, or with a field the page cannot fill.
fn form_questions(form: &Value) -> Option<Value> {
    if form.get("metadata").and_then(|metadata| metadata.get("kind")).and_then(Value::as_str) != Some(QUESTION) {
        return None;
    }
    let fields = form.get("fields")?.as_array()?;
    let questions = fields
        .iter()
        .map(|field| {
            let multiple = match field.get("type").and_then(Value::as_str) {
                Some("string") => false,
                Some("multiselect") => true,
                _ => return None,
            };
            let options: Vec<Value> = field
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|option| json!({ "label": text(option, "label"), "description": text(option, "description") }))
                .collect();
            Some(json!({ "question": text(field, "description"), "header": text(field, "title"), "options": options, "multiSelect": multiple }))
        })
        .collect::<Option<Vec<Value>>>()?;
    (!questions.is_empty()).then_some(Value::Array(questions))
}

fn request_of(session: &SessionInfo, item: &str, body: Body, asked_at: i64) -> Request {
    Request {
        id: request_id(&session.account, &session.session_id, item),
        session_id: session.session_id.clone(),
        cwd: session.cwd.clone(),
        asked_at,
        body,
    }
}

/// What the service of `session` asks the user now.
fn requests_of(service: &Service, session: &SessionInfo, now: i64) -> io::Result<Vec<Request>> {
    let mut requests: Vec<Request> = service
        .permissions(&session.session_id)?
        .iter()
        .map(|permission| request_of(session, &text(permission, "id"), permission_body(permission), now))
        .collect();
    for form in service.forms(&session.session_id)? {
        if let Some(questions) = form_questions(&form) {
            requests.push(request_of(session, &text(&form, "id"), Body::Question { questions }, now));
        }
    }
    Ok(requests)
}

/// The permissions and questions the OpenCode sessions of `sessions` wait for, read from their
/// services now, asked at `now`. A service that cannot be asked leaves its session out: its
/// question stays on its screen.
pub fn requests(sessions: &[SessionInfo], now: i64) -> Vec<Request> {
    let Ok(paths) = Paths::from_env() else {
        return Vec::new();
    };
    sessions
        .iter()
        .filter(|session| asks(session))
        .filter_map(|session| {
            let service = Service::running(&paths.account_dir(&session.account)).ok()??;
            requests_of(&service, session, now).ok()
        })
        .flatten()
        .collect()
}

/// The answer OpenCode takes for `form`, from the page's `answers` (question → label chosen, or
/// the words typed; several labels of a multiple choice joined by ", "): a value for each field
/// key, the option's value for a label. `None` when a question is left without an answer.
fn form_answer(form: &Value, answers: &BTreeMap<String, String>) -> Option<Value> {
    let mut answer = serde_json::Map::new();
    for field in form.get("fields")?.as_array()? {
        let given = answers.get(&text(field, "description"))?.trim();
        if given.is_empty() {
            return None;
        }
        let options: Vec<&Value> = field.get("options").and_then(Value::as_array).into_iter().flatten().collect();
        let value_of = |label: &str| {
            options.iter().find(|option| text(option, "label") == label).map_or_else(|| label.to_string(), |option| text(option, "value"))
        };
        let value = if field.get("type").and_then(Value::as_str) == Some("multiselect") {
            Value::Array(given.split(", ").map(|label| Value::from(value_of(label.trim()))).collect())
        } else {
            Value::from(value_of(given))
        };
        answer.insert(text(field, "key"), value);
    }
    Some(Value::Object(answer))
}

/// What to tell the user when the service refused an answer.
fn refused(item: &str, error: io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::AlreadyExists => NO_LONGER_OPEN.into(),
        _ => format!("cannot answer {item}: {error}"),
    }
}

/// Answers the permission or question named by `id` with `answer`, through the service of its
/// account.
pub fn answer(id: &str, answer: &Answer) -> Result<(), String> {
    let (account, session, item) = parse_id(id).ok_or_else(|| format!("{id} is not a request of an OpenCode session"))?;
    let paths = Paths::from_env().map_err(|error| error.to_string())?;
    let service = Service::running(&paths.account_dir(account))
        .map_err(|error| format!("{account}: {error}"))?
        .ok_or_else(|| format!("the OpenCode service of {account} does not run: the question went with it"))?;
    answer_with(&service, session, item, answer)
}

fn answer_with(service: &Service, session: &str, item: &str, answer: &Answer) -> Result<(), String> {
    if item.starts_with(PERMISSION) {
        let allow = match answer {
            Answer::Allow => true,
            Answer::Deny => false,
            _ => return Err("an OpenCode session asks to allow or deny".into()),
        };
        return service.reply_permission(session, item, allow).map_err(|error| refused(item, error));
    }
    match answer {
        Answer::Deny => service.cancel_form(session, item).map_err(|error| refused(item, error)),
        Answer::Choices { answers } => {
            let forms = service.forms(session).map_err(|error| refused(item, error))?;
            let form = forms.iter().find(|form| text(form, "id") == item).ok_or_else(|| NO_LONGER_OPEN.to_string())?;
            let value = form_answer(form, answers).ok_or("every question needs an answer")?;
            service.reply_form(session, item, value).map_err(|error| refused(item, error))
        },
        _ => Err("an OpenCode question is answered with its choices".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opencode::fake;

    fn session(state: SessionState, waiting_for: Option<&str>, agent: Agent) -> SessionInfo {
        SessionInfo {
            account: "oc".into(),
            short_id: Some("ses_a".into()),
            session_id: "ses_a".into(),
            name: String::new(),
            cwd: "/home/u/api".into(),
            state,
            isolated: false,
            progress: None,
            tasks: None,
            waiting_for: waiting_for.map(String::from),
            agent,
        }
    }

    const FORM_JSON: &str = r#"{"data":[{"id":"frm_1","sessionID":"ses_a","title":"Questions","metadata":{"kind":"question"},
        "fields":[{"key":"q0","title":"Couleur","description":"Quelle couleur ?","type":"string","options":[{"value":"bleu","label":"Bleu","description":"b"},{"value":"rouge","label":"Rouge","description":"r"}],"custom":true},
                  {"key":"q1","title":"Formes","description":"Quelles formes ?","type":"multiselect","options":[{"value":"rond","label":"Rond","description":""},{"value":"carre","label":"Carré","description":""}],"custom":true}]}]}"#;

    #[test]
    fn ids_name_the_account_the_session_and_the_request() {
        let id = request_id("oc", "ses_a", "per_1");
        assert_eq!(parse_id(&id), Some(("oc", "ses_a", "per_1")));
        assert!(is_opencode_request(&request_id("oc", "ses_a", "frm_1")));
        assert!(!is_opencode_request("codex:cx:t:i"));
        assert!(!is_opencode_request("opencode:oc:ses_a:other"));
        assert!(!is_opencode_request("1795284418968-4242"), "a request of Claude Code");
    }

    #[test]
    fn only_opencode_sessions_waiting_for_the_user_ask() {
        assert!(asks(&session(SessionState::Waiting, Some("permission prompt"), Agent::OpenCode)));
        assert!(asks(&session(SessionState::Waiting, Some("input needed"), Agent::OpenCode)));
        assert!(!asks(&session(SessionState::Working, None, Agent::OpenCode)));
        assert!(!asks(&session(SessionState::Waiting, Some("permission prompt"), Agent::Codex)));
    }

    #[test]
    fn a_permission_reads_as_the_tool_it_is() {
        let shell = json!({"id": "per_1", "action": "shell", "resources": ["touch perm_ok"]});
        assert_eq!(permission_body(&shell), Body::Permission { tool: "Bash".into(), input: json!({ "command": "touch perm_ok" }) });
        let edit = json!({"id": "per_2", "action": "edit", "resources": ["src/a.rs", "src/b.rs"]});
        assert_eq!(permission_body(&edit), Body::Permission { tool: "Edit".into(), input: json!({ "description": "change src/a.rs, src/b.rs" }) });
        let fetch = json!({"id": "per_3", "action": "webfetch", "resources": ["https://example.org"]});
        assert_eq!(permission_body(&fetch), Body::Permission { tool: "webfetch".into(), input: json!({ "description": "https://example.org" }) });
    }

    #[test]
    fn a_question_reads_as_ask_user_question_and_other_forms_stay_in_the_pane() {
        let form = serde_json::from_str::<Value>(FORM_JSON).unwrap()["data"][0].clone();
        let questions = form_questions(&form).unwrap();
        assert_eq!(questions[0]["question"], "Quelle couleur ?");
        assert_eq!(questions[0]["header"], "Couleur");
        assert_eq!(questions[0]["options"][1], json!({ "label": "Rouge", "description": "r" }));
        assert_eq!((questions[0]["multiSelect"].clone(), questions[1]["multiSelect"].clone()), (json!(false), json!(true)));
        let mut other = form.clone();
        other["metadata"]["kind"] = json!("login");
        assert_eq!(form_questions(&other), None);
        let mut unknown = form;
        unknown["fields"][0]["type"] = json!("number");
        assert_eq!(form_questions(&unknown), None, "a field the page cannot fill");
    }

    #[test]
    fn an_answer_gives_each_field_its_value_found_by_its_question() {
        let form = serde_json::from_str::<Value>(FORM_JSON).unwrap()["data"][0].clone();
        let answers = BTreeMap::from([("Quelle couleur ?".to_string(), "Rouge".to_string()), ("Quelles formes ?".to_string(), "Rond, Carré".to_string())]);
        assert_eq!(form_answer(&form, &answers), Some(json!({ "q0": "rouge", "q1": ["rond", "carre"] })));
        let typed = BTreeMap::from([("Quelle couleur ?".to_string(), "vert".to_string()), ("Quelles formes ?".to_string(), "étoile".to_string())]);
        assert_eq!(form_answer(&form, &typed), Some(json!({ "q0": "vert", "q1": ["étoile"] })), "the words typed");
        let missing = BTreeMap::from([("Quelle couleur ?".to_string(), "Rouge".to_string())]);
        assert_eq!(form_answer(&form, &missing), None);
    }

    #[test]
    fn the_requests_of_a_session_come_from_its_service() {
        let (port, _seen) = fake::service(vec![
            ("/api/session/ses_a/permission", r#"{"data":[{"id":"per_1","action":"shell","resources":["ls"]}]}"#),
            ("/api/session/ses_a/form", FORM_JSON),
        ]);
        let asking = session(SessionState::Waiting, Some("permission prompt"), Agent::OpenCode);
        let found = requests_of(&Service::on_port(port), &asking, 1_795_000_000).unwrap();
        let ids: Vec<&str> = found.iter().map(|request| request.id.as_str()).collect();
        assert_eq!(ids, ["opencode:oc:ses_a:per_1", "opencode:oc:ses_a:frm_1"]);
        assert!(matches!(found[1].body, Body::Question { .. }));
        assert_eq!((found[0].session_id.as_str(), found[0].cwd.as_str(), found[0].asked_at), ("ses_a", "/home/u/api", 1_795_000_000));
    }

    fn body_of(request: &str) -> Value {
        serde_json::from_str(request.split_once("\r\n\r\n").map_or("", |(_, body)| body)).unwrap_or(Value::Null)
    }

    #[test]
    fn answers_go_back_to_the_service() {
        let (port, seen) = fake::service(vec![
            ("/api/session/ses_a/permission/per_1/reply", ""),
            ("/api/session/ses_a/form", FORM_JSON),
            ("/api/session/ses_a/form/frm_1/reply", ""),
            ("/api/session/ses_a/form/frm_1", ""),
        ]);
        let service = Service::on_port(port);
        answer_with(&service, "ses_a", "per_1", &Answer::Allow).unwrap();
        let request = seen.recv().unwrap();
        assert!(request.starts_with("POST /api/session/ses_a/permission/per_1/reply "), "{request}");
        assert_eq!(body_of(&request), json!({ "decision": "once" }));
        answer_with(&service, "ses_a", "per_1", &Answer::Deny).unwrap();
        assert_eq!(body_of(&seen.recv().unwrap()), json!({ "decision": "reject" }));
        let answers = BTreeMap::from([("Quelle couleur ?".to_string(), "Bleu".to_string()), ("Quelles formes ?".to_string(), "Rond".to_string())]);
        answer_with(&service, "ses_a", "frm_1", &Answer::Choices { answers }).unwrap();
        assert!(seen.recv().unwrap().starts_with("GET /api/session/ses_a/form "));
        assert_eq!(body_of(&seen.recv().unwrap()), json!({ "answer": { "q0": "bleu", "q1": ["rond"] } }));
        answer_with(&service, "ses_a", "frm_1", &Answer::Deny).unwrap();
        assert!(seen.recv().unwrap().starts_with("DELETE /api/session/ses_a/form/frm_1 "));
        assert!(answer_with(&service, "ses_a", "per_1", &Answer::Text { text: "x".into() }).is_err());
    }

    #[test]
    fn a_request_answered_in_the_session_says_so() {
        let (port, _seen) = fake::service(vec![("/api/session/ses_a/form", r#"{"data":[]}"#)]);
        let service = Service::on_port(port);
        assert_eq!(answer_with(&service, "ses_a", "per_9", &Answer::Allow), Err(NO_LONGER_OPEN.to_string()), "404");
        let answers = BTreeMap::from([("Quelle couleur ?".to_string(), "Bleu".to_string())]);
        assert_eq!(answer_with(&service, "ses_a", "frm_1", &Answer::Choices { answers }), Err(NO_LONGER_OPEN.to_string()));
    }
}
