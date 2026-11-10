//! `kindric-desktop signal on|off`: a light haze of the theme's colour over the edges of the
//! screens, with a short message on top, for a skill that is about to take a window away for a
//! moment (desktop-pilot's `borrow`). `on` starts a small detached process that draws it, `off` ends it, and it ends by
//! itself `LIFETIME` after the last `on`, so a skill that fails cannot leave it up.
//!
//! The process keeps its id in a file of the runtime directory: the file gone, it ends. Drawing
//! over every window needs Wayland's layer shell (`wayland.rs`); elsewhere `on` says it cannot.

// The drawing helpers serve Wayland alone; their tests run everywhere.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

#[cfg(target_os = "linux")]
use kindric_core::mailbox;

use crate::settings::{self, Settings};

#[cfg(target_os = "linux")]
mod wayland;

/// The haze ends by itself this long after the last `on`.
pub const LIFETIME: Duration = Duration::from_secs(10);

const USAGE: &str = "usage: kindric-desktop signal on [--message TEXT] | off";
/// The process that draws: the same program, started by `on`.
const SERVE: &str = "serve";
const PID_FILE: &str = "kindric-signal.pid";

/// Opacity of the haze in the middle of the screen, at its edges, and the share of the smaller
/// side over which it thickens.
const MIDDLE: f32 = 0.09;
const EDGE: f32 = 0.42;
const BAND: f32 = 0.18;

/// The message without `--message`, by language of the system; English for the others.
const MESSAGES: [(&str, &str); 6] = [
    ("en", "Claude borrows a window, hands off for a second"),
    ("fr", "Claude emprunte une fenêtre, ne touche à rien une seconde"),
    ("es", "Claude toma prestada una ventana, no toques nada un segundo"),
    ("de", "Claude leiht sich ein Fenster, kurz nichts anfassen"),
    ("it", "Claude prende in prestito una finestra, non toccare niente per un secondo"),
    ("pt", "O Claude pegou uma janela emprestada, não mexa em nada por um segundo"),
];

/// The accents of the themes of `ui/themes.js`, their `active` colour.
const ACCENTS: [(&str, &str); 3] = [("ember", "#aa601f"), ("paper", "#9a5418"), ("graphite", "#3f6fb8")];

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    On(Option<String>),
    Off,
    Serve(Option<String>),
}

/// The action of `kindric-desktop signal …`; None when the arguments are not a signal command.
pub fn parse(args: &[String]) -> Option<Result<Action, String>> {
    let (first, rest) = args.split_first()?;
    if first != "signal" {
        return None;
    }
    let message = |rest: &[String]| match rest {
        [] => Ok(None),
        [flag, text] if flag == "--message" && !text.trim().is_empty() => Ok(Some(text.trim().to_string())),
        _ => Err(USAGE.to_string()),
    };
    Some(match rest {
        [word, more @ ..] if word == "on" => message(more).map(Action::On),
        [word] if word == "off" => Ok(Action::Off),
        [word, more @ ..] if word == SERVE => message(more).map(Action::Serve),
        _ => Err(USAGE.to_string()),
    })
}

/// Runs `action`: 0 when done, 1 when it failed, 2 for a wrong command line.
pub fn run(action: Result<Action, String>) -> ExitCode {
    let result = match action {
        Ok(Action::On(message)) => on(message),
        Ok(Action::Off) => off(),
        Ok(Action::Serve(message)) => serve(message),
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("kindric-desktop: {error}");
            ExitCode::FAILURE
        },
    }
}

/// Opacity of the haze at (`x`, `y`) on a `width` × `height` surface: faint over all of it, and
/// thicker, softly, toward the nearest edge.
pub fn haze_alpha(x: u32, y: u32, width: u32, height: u32) -> f32 {
    let from_edge = x.min(y).min(width.saturating_sub(1 + x)).min(height.saturating_sub(1 + y));
    let band = (width.min(height) as f32 * BAND).max(1.0);
    let closeness = (1.0 - from_edge as f32 / band).clamp(0.0, 1.0);
    MIDDLE + (EDGE - MIDDLE) * closeness * closeness
}

/// One pixel of `rgb` at `alpha`, as Wayland's ARGB8888 wants it: premultiplied, in memory order
/// blue, green, red, alpha.
pub fn premultiplied(rgb: [u8; 3], alpha: f32) -> [u8; 4] {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round();
    let scale = |channel: u8| (f32::from(channel) * a / 255.0).round() as u8;
    [scale(rgb[2]), scale(rgb[1]), scale(rgb[0]), a as u8]
}

/// `top` laid over `under`, both premultiplied.
pub fn over(under: [u8; 4], top: [u8; 4]) -> [u8; 4] {
    let keep = 255 - u16::from(top[3]);
    let mix = |channel: usize| (u16::from(top[channel]) + (u16::from(under[channel]) * keep + 127) / 255).min(255) as u8;
    [mix(0), mix(1), mix(2), mix(3)]
}

/// The default message in the language of `locale` (`fr_FR.UTF-8`, `de`…), English otherwise.
fn default_message(locale: &str) -> &'static str {
    let language = locale.split(['_', '.', '@', '-']).next().unwrap_or("");
    MESSAGES.iter().find(|(code, _)| *code == language).map_or(MESSAGES[0].1, |(_, message)| message)
}

/// The language of the messages, as POSIX sets it: `LC_ALL`, then `LC_MESSAGES`, then `LANG`.
fn system_locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa` as red, green, blue; the alpha is left out.
fn rgb_of(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.strip_prefix('#')?;
    let value = |text: &str| u8::from_str_radix(text, 16).ok();
    match digits.len() {
        3 => {
            let mut channels = digits.chars().map(|c| value(&c.to_string().repeat(2)));
            Some([channels.next()??, channels.next()??, channels.next()??])
        },
        6 | 8 => Some([value(&digits[0..2])?, value(&digits[2..4])?, value(&digits[4..6])?]),
        _ => None,
    }
}

/// The accent of the theme chosen in kindric: the `active` colour of `theme.json` for "custom",
/// ember when nothing better can be read.
fn accent(data_dir: &Path) -> [u8; 3] {
    let ember = [0xaa, 0x60, 0x1f];
    let chosen = std::fs::read_to_string(data_dir.join("settings.json"))
        .ok()
        .and_then(|json| serde_json::from_str::<Settings>(&json).ok())
        .and_then(|saved| saved.theme);
    let colour = match chosen.as_deref() {
        Some("custom") => std::fs::read_to_string(data_dir.join("theme.json"))
            .ok()
            .and_then(|json| settings::custom_theme(&json).ok())
            .and_then(|colours| colours.get("active").cloned()),
        Some(name) => ACCENTS.iter().find(|(theme, _)| *theme == name).map(|(_, hex)| hex.to_string()),
        None => None,
    };
    colour.as_deref().and_then(rgb_of).unwrap_or(ember)
}

fn pid_file() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty());
    runtime.map(|dir| PathBuf::from(dir).join(PID_FILE)).ok_or_else(|| "XDG_RUNTIME_DIR is not set".to_string())
}

fn read_pid(file: &Path) -> Option<u32> {
    std::fs::read_to_string(file).ok()?.trim().parse().ok()
}

#[cfg(target_os = "linux")]
fn on(message: Option<String>) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let file = pid_file()?;
    if let Some(pid) = read_pid(&file).filter(|pid| is_signal_process(*pid)) {
        // Shown already: it lasts `LIFETIME` from now.
        return std::fs::write(&file, format!("{pid}\n")).map_err(|error| format!("cannot write {}: {error}", file.display()));
    }
    let program = std::env::current_exe().map_err(|error| format!("cannot find this program: {error}"))?;
    let mut command = Command::new(program);
    command.args(["signal", SERVE]);
    if let Some(message) = &message {
        command.args(["--message", message]);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        // Out of the caller's process group: closing its terminal leaves the haze to `off`.
        .process_group(0)
        .spawn()
        .map_err(|error| format!("cannot start the signal: {error}"))?;
    // The process writes the file once the haze is drawn: `on` returns when it shows.
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        if read_pid(&file) == Some(child.id()) {
            return Ok(());
        }
        if let Ok(Some(_)) = child.try_wait() {
            return Err("the signal did not start".to_string());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err("the signal did not show within 2 s".to_string())
}

#[cfg(not(target_os = "linux"))]
fn on(_: Option<String>) -> Result<(), String> {
    Err("signal is not available on this system yet".to_string())
}

/// The file gone, the process ends within `wayland::WATCH`; without it there is nothing to do.
fn off() -> Result<(), String> {
    let file = pid_file()?;
    match std::fs::remove_file(&file) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot remove {}: {error}", file.display())),
    }
}

#[cfg(target_os = "linux")]
fn serve(message: Option<String>) -> Result<(), String> {
    let data_dir = mailbox::data_dir().map_err(|error| error.to_string())?;
    let message = message.unwrap_or_else(|| default_message(&system_locale()).to_string());
    wayland::show(accent(&data_dir), &message, &pid_file()?)
}

#[cfg(not(target_os = "linux"))]
fn serve(message: Option<String>) -> Result<(), String> {
    on(message)
}

/// A process of this program drawing the haze, not another that took its id since.
#[cfg(target_os = "linux")]
fn is_signal_process(pid: u32) -> bool {
    let Ok(command_line) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let words: Vec<&[u8]> = command_line.split(|byte| *byte == 0).collect();
    words.windows(2).any(|pair| pair[0] == b"signal" && pair[1] == SERVE.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn only_signal_commands_are_taken() {
        assert_eq!(parse(&args(&[])), None);
        assert_eq!(parse(&args(&["--help"])), None);
        assert_eq!(parse(&args(&["signal", "on"])), Some(Ok(Action::On(None))));
        assert_eq!(parse(&args(&["signal", "on", "--message", " Reading the mail "])), Some(Ok(Action::On(Some("Reading the mail".into())))));
        assert_eq!(parse(&args(&["signal", "off"])), Some(Ok(Action::Off)));
        assert_eq!(parse(&args(&["signal", "serve", "--message", "x"])), Some(Ok(Action::Serve(Some("x".into())))));
        let wrong: [&[&str]; 6] = [
            &["signal"],
            &["signal", "blink"],
            &["signal", "on", "now"],
            &["signal", "on", "--message"],
            &["signal", "on", "--message", "  "],
            &["signal", "off", "--message", "x"],
        ];
        for wrong in wrong {
            assert_eq!(parse(&args(wrong)), Some(Err(USAGE.to_string())), "{wrong:?}");
        }
    }

    #[test]
    fn the_haze_is_faint_in_the_middle_and_thickens_toward_the_edges() {
        let (width, height) = (1000, 600);
        let middle = haze_alpha(500, 300, width, height);
        assert!((middle - MIDDLE).abs() < 1e-6, "{middle}");
        for (x, y) in [(0, 300), (999, 300), (500, 0), (500, 599)] {
            assert!((haze_alpha(x, y, width, height) - EDGE).abs() < 1e-6, "({x}, {y})");
        }
        let toward_edge: Vec<f32> = (0..=80).rev().map(|x| haze_alpha(x, 300, width, height)).collect();
        assert!(toward_edge.windows(2).all(|pair| pair[0] <= pair[1]), "{toward_edge:?}");
    }

    #[test]
    fn pixels_are_premultiplied_in_memory_order() {
        assert_eq!(premultiplied([0xaa, 0x60, 0x1f], 1.0), [0x1f, 0x60, 0xaa, 0xff]);
        assert_eq!(premultiplied([0xff, 0x80, 0x00], 0.0), [0, 0, 0, 0]);
        let half = premultiplied([0xff, 0xff, 0xff], 0.5);
        assert_eq!(half, [128, 128, 128, 128]);
    }

    #[test]
    fn the_message_speaks_the_language_of_the_system() {
        assert_eq!(default_message("fr_FR.UTF-8"), "Claude emprunte une fenêtre, ne touche à rien une seconde");
        assert_eq!(default_message("de_DE"), "Claude leiht sich ein Fenster, kurz nichts anfassen");
        assert_eq!(default_message("pt_BR.UTF-8@x"), MESSAGES[5].1);
        for other in ["", "C", "C.UTF-8", "ja_JP.UTF-8", "en_US.UTF-8"] {
            assert_eq!(default_message(other), MESSAGES[0].1, "{other:?}");
        }
    }

    #[test]
    fn a_layer_over_another_keeps_what_shows_through() {
        let under = premultiplied([0xaa, 0x60, 0x1f], 0.4);
        assert_eq!(over(under, [0, 0, 0, 0]), under, "a transparent top changes nothing");
        assert_eq!(over(under, [10, 20, 30, 255]), [10, 20, 30, 255], "an opaque top hides all");
        let half = over([0, 0, 0, 255], premultiplied([0xff, 0xff, 0xff], 0.5));
        assert_eq!(half, [128, 128, 128, 255]);
    }

    #[test]
    fn colours_are_read_in_every_form_the_themes_use() {
        assert_eq!(rgb_of("#aa601f"), Some([0xaa, 0x60, 0x1f]));
        assert_eq!(rgb_of("#fff"), Some([0xff, 0xff, 0xff]));
        assert_eq!(rgb_of("#c04020cc"), Some([0xc0, 0x40, 0x20]));
        assert_eq!(rgb_of("red"), None);
        assert_eq!(rgb_of("#12345"), None);
    }

    #[test]
    fn the_accent_follows_the_theme_chosen() {
        let dir = std::env::temp_dir().join(format!("kindric-signal-accent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(accent(&dir), [0xaa, 0x60, 0x1f], "ember without settings");
        std::fs::write(dir.join("settings.json"), r#"{"theme":"graphite"}"#).unwrap();
        assert_eq!(accent(&dir), [0x3f, 0x6f, 0xb8]);
        std::fs::write(dir.join("settings.json"), r#"{"theme":"custom"}"#).unwrap();
        std::fs::write(dir.join("theme.json"), r##"{"active":"#123456"}"##).unwrap();
        assert_eq!(accent(&dir), [0x12, 0x34, 0x56]);
        std::fs::write(dir.join("theme.json"), "{broken").unwrap();
        assert_eq!(accent(&dir), [0xaa, 0x60, 0x1f], "ember when theme.json does not load");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_accents_are_those_of_the_themes_of_the_page() {
        let themes = include_str!("../ui/themes.js");
        for (name, hex) in ACCENTS {
            let block = &themes[themes.find(&format!("  {name}: {{")).unwrap_or_else(|| panic!("no theme {name}"))..];
            let block = &block[..block.find("  },").unwrap()];
            assert!(block.contains(&format!("active: \"{hex}\"")), "{name} has another active colour than {hex}");
        }
    }
}
