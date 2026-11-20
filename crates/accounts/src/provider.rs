//! Accounts whose model does not come from a Claude subscription: an Anthropic API key, OpenRouter,
//! Ollama or any gateway that speaks Anthropic's API. Claude Code still runs, unmodified; only
//! where its requests go and which key they carry change.
//!
//! - `<account>/kindric-provider.json` says which provider, its address and its model. An account
//!   without it is a Claude account, signed in with `/login`.
//! - `<account>/kindric-token` holds the key, in 600 on Unix, written only by `kindric-accounts
//!   token`. Sessions get it through Claude Code's `apiKeyHelper`, which runs `kindric-accounts
//!   token <name> --print`: the key never goes into a settings file nor into the environment.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{Error, Result, fail};
use crate::files;
use crate::paths::Paths;

const PROVIDER_FILE: &str = "kindric-provider.json";
const TOKEN_FILE: &str = "kindric-token";

/// The key Claude Code sends to a provider that needs none: Ollama ignores it, and without a key
/// Claude Code asks for `/login`.
const NO_KEY: &str = "no-key";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// An Anthropic API key, on Anthropic's API.
    Api,
    OpenRouter,
    /// A local Ollama server.
    Ollama,
    /// Any other server that speaks Anthropic's Messages API.
    Gateway,
}

impl Kind {
    pub const NAMES: [&str; 4] = ["api", "openrouter", "ollama", "gateway"];

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "api" => Some(Self::Api),
            "openrouter" => Some(Self::OpenRouter),
            "ollama" => Some(Self::Ollama),
            "gateway" => Some(Self::Gateway),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::OpenRouter => "openrouter",
            Self::Ollama => "ollama",
            Self::Gateway => "gateway",
        }
    }

    /// The address used when none is given; `None` for the gateway, which has no usual one, and for
    /// the API key, which goes to Anthropic.
    fn default_url(self) -> Option<&'static str> {
        match self {
            Self::OpenRouter => Some("https://openrouter.ai/api"),
            Self::Ollama => Some("http://localhost:11434"),
            Self::Api | Self::Gateway => None,
        }
    }

    /// Only Ollama works without a key.
    pub fn needs_token(self) -> bool {
        self != Self::Ollama
    }

    /// Anthropic's API knows its models; a gateway serves the names it chooses.
    fn needs_model(self) -> bool {
        self != Self::Api
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    pub provider: Kind,
    /// Where Claude Code sends its requests, `ANTHROPIC_BASE_URL`; `None` keeps Anthropic's API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The model every request uses, as the provider names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl Provider {
    /// Checks what the user gave and fills in the usual address of the provider.
    pub fn new(kind: Kind, url: Option<&str>, model: Option<&str>) -> Result<Self> {
        let url = match url.map(str::trim).filter(|url| !url.is_empty()) {
            Some(url) => Some(valid_url(url)?),
            None if kind == Kind::Gateway => return fail("a gateway needs its address: --url https://…"),
            None => kind.default_url().map(String::from),
        };
        let model = model.map(str::trim).filter(|model| !model.is_empty());
        if let Some(model) = model
            && model.chars().any(|character| character.is_control() || character.is_whitespace())
        {
            return fail(format!("invalid model name {model:?}: no spaces or control characters"));
        }
        if model.is_none() && kind.needs_model() {
            return fail(format!("a {} account needs its model: --model <name, as {} calls it>", kind.name(), kind.name()));
        }
        Ok(Self { provider: kind, url, model: model.map(String::from) })
    }

    /// The variables and the key helper a session of this account gets, added to its settings.
    fn settings_additions(&self, key_helper: &str) -> (Map<String, Value>, Value) {
        let mut env = Map::new();
        if let Some(url) = &self.url {
            env.insert("ANTHROPIC_BASE_URL".into(), Value::from(url.as_str()));
        }
        if let Some(model) = &self.model {
            // The model of the conversation, of its subagents and of the small background
            // requests (titles, summaries): Claude Code would ask the provider for Claude models
            // by their names otherwise.
            for variable in MODEL_VARIABLES {
                env.insert(variable.into(), Value::from(model.as_str()));
            }
        }
        (env, Value::from(key_helper))
    }
}

/// The variables that name a model in Claude Code (read in the binary of 2.1.329).
const MODEL_VARIABLES: [&str; 5] = [
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
];

fn valid_url(url: &str) -> Result<String> {
    let has_scheme = url.starts_with("http://") || url.starts_with("https://");
    let host = url.split_once("://").map_or("", |(_, rest)| rest);
    if !has_scheme || host.is_empty() || url.chars().any(|character| character.is_control() || character.is_whitespace()) {
        return fail(format!("invalid address {url:?}: give it as http://host:port or https://host"));
    }
    Ok(url.trim_end_matches('/').to_string())
}

pub fn provider_file(dir: &Path) -> PathBuf {
    dir.join(PROVIDER_FILE)
}

pub fn token_file(dir: &Path) -> PathBuf {
    dir.join(TOKEN_FILE)
}

/// The provider of the account in `dir`; `None` for a Claude account.
pub fn read(dir: &Path) -> Result<Option<Provider>> {
    let file = provider_file(dir);
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error(format!("cannot read {}: {error}", file.display()))),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| Error(format!("{} is not valid: {error}; remove the account and add it again", file.display())))
}

pub fn write(dir: &Path, provider: &Provider) -> Result<()> {
    let file = provider_file(dir);
    let text = serde_json::to_string_pretty(provider).map_err(|error| Error(error.to_string()))?;
    files::write_atomic(&file, &format!("{text}\n")).map_err(|error| Error(format!("cannot write {}: {error}", file.display())))
}

/// Keeps `token`, given on one line, in 600. Spaces around it are dropped.
pub fn save_token(dir: &Path, token: &str) -> Result<()> {
    let token = token.trim();
    if token.is_empty() {
        return fail("empty key: give the key on standard input, for example: kindric-accounts token <name> < key.txt");
    }
    if token.chars().any(char::is_control) {
        return fail("the key holds a line break or a control character: give it on a single line");
    }
    let file = token_file(dir);
    files::write_atomic(&file, &format!("{token}\n")).map_err(|error| Error(format!("cannot write {}: {error}", file.display())))
}

/// The key kept for the account in `dir`, if any.
pub fn read_token(dir: &Path) -> Result<Option<String>> {
    let file = token_file(dir);
    match fs::read_to_string(&file) {
        Ok(text) => Ok(Some(text.trim().to_string()).filter(|token| !token.is_empty())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error(format!("cannot read {}: {error}", file.display()))),
    }
}

/// What `apiKeyHelper` prints for the account in `dir`: its key, or the placeholder of a provider
/// that needs none.
pub fn key_for_session(dir: &Path, provider: &Provider, name: &str) -> Result<String> {
    match read_token(dir)? {
        Some(token) => Ok(token),
        None if !provider.provider.needs_token() => Ok(NO_KEY.to_string()),
        None => fail(format!("no key for {name}: kindric-accounts token {name} < key.txt")),
    }
}

/// Whether the account can start a session: a provider that needs a key has one.
pub fn is_ready(dir: &Path, provider: &Provider) -> bool {
    !provider.provider.needs_token() || read_token(dir).is_ok_and(|token| token.is_some())
}

/// `settings` (a settings file's JSON, or none) with the provider's variables merged into `env`
/// and `apiKeyHelper` replaced: a key helper or a base address of the user's own would send the
/// session elsewhere.
pub fn with_provider(settings: Option<&str>, provider: &Provider, key_helper: &str) -> Result<String> {
    let mut settings = match settings.map(serde_json::from_str::<Map<String, Value>>) {
        Some(Ok(settings)) => settings,
        Some(Err(error)) => return fail(format!("invalid JSON: {error}")),
        None => Map::new(),
    };
    let (additions, helper) = provider.settings_additions(key_helper);
    let mut env = match settings.remove("env") {
        Some(Value::Object(env)) => env,
        _ => Map::new(),
    };
    if provider.url.is_none() {
        env.remove("ANTHROPIC_BASE_URL");
    }
    env.extend(additions);
    settings.insert("env".into(), Value::Object(env));
    settings.insert("apiKeyHelper".into(), helper);
    serde_json::to_string_pretty(&Value::Object(settings)).map_err(|error| Error(error.to_string()))
}

/// `"<kindric-accounts>" token <name> --print`, the key helper of the account's sessions.
pub fn key_helper_command(program: &Path, name: &str) -> String {
    format!("\"{}\" token {name} --print", program.display())
}

/// Writes `.shared-settings/provider/<name>.json` (a folder of its own: a theme may be called
/// `provider`, and `<name>.<theme>.json` is taken): `source` (the settings the session would get,
/// or none) with the provider added. The session cannot start without it: it would go to
/// Anthropic with whatever login the account directory holds.
pub fn write_settings(paths: &Paths, name: &str, source: Option<&Path>, provider: &Provider, program: &Path) -> Result<PathBuf> {
    let text = match source {
        None => None,
        Some(path) => match fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(Error(format!("cannot read {}: {error}", path.display()))),
        },
    };
    let content = with_provider(text.as_deref(), provider, &key_helper_command(program, name)).map_err(|error| match source {
        Some(path) => Error(format!("{}: {error}", path.display())),
        None => error,
    })?;
    let file = paths.data_root.join(".shared-settings").join("provider").join(format!("{name}.json"));
    file.parent()
        .map_or(Ok(()), files::create_private_dir)
        .and_then(|()| files::write_atomic(&file, &content))
        .map_err(|error| Error(format!("cannot write {}: {error}", file.display())))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::test_dir;

    #[test]
    fn known_providers_get_their_usual_address() {
        let openrouter = Provider::new(Kind::OpenRouter, None, Some("qwen/qwen3-coder")).unwrap();
        assert_eq!(openrouter.url.as_deref(), Some("https://openrouter.ai/api"));
        let ollama = Provider::new(Kind::Ollama, None, Some("qwen3")).unwrap();
        assert_eq!(ollama.url.as_deref(), Some("http://localhost:11434"));
        let api = Provider::new(Kind::Api, None, None).unwrap();
        assert_eq!(api.url, None);
        assert_eq!(api.model, None);
    }

    #[test]
    fn a_given_address_wins_and_loses_its_trailing_slash() {
        let ollama = Provider::new(Kind::Ollama, Some("http://box:11434/"), Some("qwen3")).unwrap();
        assert_eq!(ollama.url.as_deref(), Some("http://box:11434"));
    }

    #[test]
    fn what_is_missing_or_malformed_is_refused_with_the_option_to_give() {
        let gateway = Provider::new(Kind::Gateway, None, Some("m")).unwrap_err().to_string();
        assert!(gateway.contains("--url"), "{gateway}");
        let model = Provider::new(Kind::OpenRouter, None, None).unwrap_err().to_string();
        assert!(model.contains("--model"), "{model}");
        assert!(Provider::new(Kind::Gateway, Some("box:8080"), Some("m")).is_err());
        assert!(Provider::new(Kind::Gateway, Some("http://"), Some("m")).is_err());
        assert!(Provider::new(Kind::Ollama, None, Some("two words")).is_err());
    }

    #[test]
    fn every_kind_name_parses_back() {
        for name in Kind::NAMES {
            assert_eq!(Kind::parse(name).map(Kind::name), Some(name));
        }
        assert_eq!(Kind::parse("claude"), None);
    }

    #[test]
    fn the_provider_file_round_trips_and_its_absence_means_claude() {
        let dir = test_dir::fresh("provider-file");
        assert_eq!(read(&dir).unwrap(), None);
        let provider = Provider::new(Kind::OpenRouter, None, Some("qwen/qwen3-coder")).unwrap();
        write(&dir, &provider).unwrap();
        assert_eq!(read(&dir).unwrap(), Some(provider));
        let text = fs::read_to_string(provider_file(&dir)).unwrap();
        assert!(text.contains("\"provider\": \"openrouter\""), "{text}");
    }

    #[test]
    fn a_broken_provider_file_says_how_to_recover() {
        let dir = test_dir::fresh("provider-broken");
        fs::write(provider_file(&dir), "{").unwrap();
        let error = read(&dir).unwrap_err().to_string();
        assert!(error.contains("add it again"), "{error}");
    }

    #[test]
    fn the_token_is_kept_on_one_line_and_private() {
        let dir = test_dir::fresh("provider-token");
        assert_eq!(read_token(&dir).unwrap(), None);
        save_token(&dir, "  sk-or-v1-abc \n").unwrap();
        assert_eq!(read_token(&dir).unwrap().as_deref(), Some("sk-or-v1-abc"));
        assert!(save_token(&dir, " \n").is_err());
        assert!(save_token(&dir, "a\nb").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(token_file(&dir)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_session_gets_the_key_or_the_placeholder_of_ollama() {
        let dir = test_dir::fresh("provider-key");
        let openrouter = Provider::new(Kind::OpenRouter, None, Some("m")).unwrap();
        let ollama = Provider::new(Kind::Ollama, None, Some("m")).unwrap();
        let missing = key_for_session(&dir, &openrouter, "router").unwrap_err().to_string();
        assert!(missing.contains("kindric-accounts token router"), "{missing}");
        assert!(!is_ready(&dir, &openrouter));
        assert_eq!(key_for_session(&dir, &ollama, "local").unwrap(), NO_KEY);
        assert!(is_ready(&dir, &ollama));
        save_token(&dir, "secret").unwrap();
        assert_eq!(key_for_session(&dir, &openrouter, "router").unwrap(), "secret");
        assert!(is_ready(&dir, &openrouter));
    }

    #[test]
    fn settings_get_the_address_the_models_and_the_key_helper_over_the_users() {
        let provider = Provider::new(Kind::OpenRouter, None, Some("qwen/qwen3-coder")).unwrap();
        let user = r#"{"env": {"FOO": "1", "ANTHROPIC_BASE_URL": "http://elsewhere"}, "apiKeyHelper": "pass show x", "theme": "dark"}"#;
        let text = with_provider(Some(user), &provider, "\"/bin/kindric-accounts\" token router --print").unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["env"]["FOO"], "1");
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://openrouter.ai/api");
        for variable in MODEL_VARIABLES {
            assert_eq!(value["env"][variable], "qwen/qwen3-coder", "{variable}");
        }
        assert_eq!(value["apiKeyHelper"], "\"/bin/kindric-accounts\" token router --print");
        assert_eq!(value["theme"], "dark");
    }

    #[test]
    fn an_api_key_account_drops_a_base_address_of_the_user() {
        let provider = Provider::new(Kind::Api, None, None).unwrap();
        let text = with_provider(Some(r#"{"env": {"ANTHROPIC_BASE_URL": "http://elsewhere"}}"#), &provider, "h").unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert!(value["env"].get("ANTHROPIC_BASE_URL").is_none(), "{text}");
        assert!(value["env"].get("ANTHROPIC_MODEL").is_none(), "{text}");
    }

    #[test]
    fn invalid_user_settings_stop_the_launch() {
        let provider = Provider::new(Kind::Ollama, None, Some("m")).unwrap();
        assert!(with_provider(Some("{"), &provider, "h").is_err());
    }

    #[test]
    fn the_settings_file_is_written_beside_the_others() {
        let root = test_dir::fresh("provider-settings");
        let paths = Paths::from_roots(root.join("config"), root.join("data"), root.join("home"));
        let provider = Provider::new(Kind::Ollama, None, Some("qwen3")).unwrap();
        let program = PathBuf::from("/bin/kindric-accounts");
        let file = write_settings(&paths, "local", None, &provider, &program).unwrap();
        assert_eq!(file, paths.data_root.join(".shared-settings").join("provider").join("local.json"));
        let value: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(value["apiKeyHelper"], key_helper_command(&program, "local"));
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "http://localhost:11434");
    }
}
