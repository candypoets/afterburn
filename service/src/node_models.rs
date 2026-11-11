//! The model each agent runs, by Herdr pane.
//!
//! Herdr reports an agent's kind and state, never its model. The node finds it from the agent's
//! own process: every process in a pane carries `HERDR_PANE_ID`, so the CLI's command line
//! (`--model opus`) or, failing that, the CLI's own record says what it runs: Claude's session
//! transcript, Codex's config. A model that can't be found is left out; the row names none.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

/// How long a reading of the processes is reused: the client asks every few seconds.
const KEEP: Duration = Duration::from_secs(20);
/// How much of a transcript's end is read for its last model.
const TAIL: u64 = 512 * 1024;

/// The model of the agent in each pane, as shown (`Opus 5.5`), for the panes where it is known.
pub fn by_pane() -> HashMap<String, String> {
    static CACHE: OnceLock<Mutex<Option<(Instant, HashMap<String, String>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let Ok(mut cache) = cache.lock() else {
        return read();
    };
    if let Some((_, models)) = cache.as_ref().filter(|(at, _)| at.elapsed() < KEEP) {
        return models.clone();
    }
    let models = read();
    *cache = Some((Instant::now(), models.clone()));
    models
}

/// Read every agent process once. Only Linux has `/proc`; elsewhere no model is known.
fn read() -> HashMap<String, String> {
    let mut found: HashMap<String, (u32, String)> = HashMap::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return HashMap::new();
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Some(argv) = strings(&entry.path().join("cmdline")) else {
            continue;
        };
        let Some(kind) = argv.first().and_then(|program| cli(program)) else {
            continue;
        };
        let Some(environ) = strings(&entry.path().join("environ")) else {
            continue;
        };
        let var = |name: &str| {
            environ
                .iter()
                .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
                .map(str::to_owned)
        };
        let Some(pane) = var("HERDR_PANE_ID").filter(|pane| !pane.is_empty()) else {
            continue;
        };
        // The pane's agent is its oldest such process: a CLI may start helpers of its own name.
        if found.get(&pane).is_some_and(|(first, _)| *first < pid) {
            continue;
        }
        let home = var("HOME").map(PathBuf::from);
        let model = arg_model(&argv).or_else(|| match kind {
            Cli::Claude => {
                let dir = var("CLAUDE_CONFIG_DIR")
                    .map(PathBuf::from)
                    .or_else(|| home.as_ref().map(|home| home.join(".claude")))?;
                claude_model(&dir, pid)
            }
            Cli::Codex => {
                let dir = var("CODEX_HOME")
                    .map(PathBuf::from)
                    .or_else(|| home.as_ref().map(|home| home.join(".codex")))?;
                config_model(&fs::read_to_string(dir.join("config.toml")).ok()?)
            }
            Cli::Other => None,
        });
        if let Some(model) = model {
            found.insert(pane, (pid, label(&model)));
        }
    }
    found
        .into_iter()
        .map(|(pane, (_, model))| (pane, model))
        .collect()
}

/// The agent CLIs whose model the node can find.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cli {
    Claude,
    Codex,
    Other,
}

/// The CLI a program is, by its file name; `None` for anything that is not an agent.
fn cli(program: &str) -> Option<Cli> {
    let name = Path::new(program).file_name()?.to_str()?;
    match name {
        "claude" => Some(Cli::Claude),
        "codex" => Some(Cli::Codex),
        "kimi" | "kimi-code" | "dsh" | "dsh-tui" | "deepseek" => Some(Cli::Other),
        _ => None,
    }
}

/// A `/proc` file of NUL-separated strings.
fn strings(path: &Path) -> Option<Vec<String>> {
    let bytes = fs::read(path).ok()?;
    let list: Vec<String> = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();
    (!list.is_empty()).then_some(list)
}

/// The model a command line names: `--model opus`, `--model=opus` or `-m opus`.
fn arg_model(argv: &[String]) -> Option<String> {
    let mut args = argv.iter().skip(1);
    while let Some(arg) = args.next() {
        if let Some(model) = arg.strip_prefix("--model=") {
            return Some(model.to_owned()).filter(|model| !model.is_empty());
        }
        if arg == "--model" || arg == "-m" {
            return args.next().cloned().filter(|model| !model.is_empty());
        }
    }
    None
}

/// The model of the Claude session process `pid` runs: its session file names the session, and
/// the session's transcript names the model of each answer.
fn claude_model(dir: &Path, pid: u32) -> Option<String> {
    let session: Value =
        serde_json::from_str(&fs::read_to_string(dir.join(format!("sessions/{pid}.json"))).ok()?)
            .ok()?;
    let id = session["sessionId"].as_str()?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let projects = fs::read_dir(dir.join("projects")).ok()?;
    let transcript = projects
        .flatten()
        .map(|project| project.path().join(format!("{id}.jsonl")))
        .find(|path| path.is_file())?;
    transcript_model(&tail(&transcript)?)
}

/// The end of a file, from a line start.
fn tail(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let from = len.saturating_sub(TAIL);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if from == 0 {
        return Some(text);
    }
    // The first line is cut: start after it.
    text.split_once('\n').map(|(_, rest)| rest.to_owned())
}

/// The model of the last answer in a Claude transcript; sub-agents' answers don't count.
fn transcript_model(text: &str) -> Option<String> {
    text.lines().rev().find_map(|line| {
        if !line.contains("\"model\"") {
            return None;
        }
        let entry: Value = serde_json::from_str(line).ok()?;
        if entry["type"] != "assistant" || entry["isSidechain"] == true {
            return None;
        }
        let model = entry["message"]["model"].as_str()?;
        (!model.is_empty() && !model.starts_with('<')).then(|| model.to_owned())
    })
}

/// The top-level `model = "…"` of a Codex config.
fn config_model(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            // Tables follow the top-level keys.
            return None;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "model" {
            let value = value.split('#').next().unwrap_or(value).trim();
            let value = value.trim_matches(|c| c == '"' || c == '\'');
            return (!value.is_empty()).then(|| value.to_owned());
        }
    }
    None
}

/// A model id as a row shows it: `claude-opus-5-5` is `Opus 5.5`, the alias `opus` is `Opus`,
/// `gpt-6.1-sol` is `GPT-6.1 Sol`; anything else stays as the CLI wrote it.
fn label(model: &str) -> String {
    let model = model.split('[').next().unwrap_or(model).trim();
    let title = |word: &str| {
        let mut chars = word.chars();
        chars
            .next()
            .map(|first| first.to_uppercase().chain(chars).collect::<String>())
            .unwrap_or_default()
    };
    if let Some(rest) = model.strip_prefix("claude-") {
        let mut parts: Vec<&str> = rest.split('-').collect();
        // A dated release (`…-20261001`) is the same model.
        if parts
            .last()
            .is_some_and(|part| part.len() == 8 && part.chars().all(|c| c.is_ascii_digit()))
        {
            parts.pop();
        }
        let (words, numbers): (Vec<&str>, Vec<&str>) = parts
            .iter()
            .partition(|part| !part.chars().all(|c| c.is_ascii_digit()));
        let name = words
            .iter()
            .map(|word| title(word))
            .collect::<Vec<_>>()
            .join(" ");
        if !name.is_empty() {
            return [name, numbers.join(".")]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
        }
    }
    if let Some(rest) = model.strip_prefix("gpt-") {
        let mut parts = rest.split('-');
        let version = parts.next().unwrap_or_default();
        let rest: Vec<String> = parts.map(title).collect();
        return [format!("GPT-{version}"), rest.join(" ")]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    if !model.is_empty() && model.chars().all(|c| c.is_ascii_lowercase()) {
        return title(model);
    }
    model.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_id_reads_as_a_name() {
        assert_eq!(label("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(label("claude-fable-5-2[1m]"), "Fable 5.2");
        assert_eq!(label("claude-haiku-5-5-20261001"), "Haiku 5.5");
        assert_eq!(label("opus"), "Opus");
        assert_eq!(label("gpt-6.1-sol"), "GPT-6.1 Sol");
        assert_eq!(label("gpt-6.1"), "GPT-6.1");
        assert_eq!(label("kimi-k2-0905"), "kimi-k2-0905");
    }

    #[test]
    fn the_command_line_names_the_model_first() {
        let argv = |line: &str| line.split(' ').map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(
            arg_model(&argv("claude --model opus")).as_deref(),
            Some("opus")
        );
        assert_eq!(
            arg_model(&argv("codex -m gpt-6.1 --yolo")).as_deref(),
            Some("gpt-6.1")
        );
        assert_eq!(
            arg_model(&argv("claude --model=sonnet")).as_deref(),
            Some("sonnet")
        );
        assert_eq!(arg_model(&argv("claude --continue")), None);
        assert_eq!(arg_model(&argv("claude --model")), None);
        assert_eq!(cli("/usr/local/bin/claude"), Some(Cli::Claude));
        assert_eq!(cli("bash"), None);
    }

    #[test]
    fn a_transcript_names_its_last_answers_model() {
        let text = concat!(
            r#"{"type":"assistant","message":{"model":"claude-opus-5-5"}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"say \"model\""}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-fable-5-2"}}"#,
            "\n",
            r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-5-5"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"<synthetic>"}}"#,
            "\n",
            "not json \"model\"\n",
        );
        assert_eq!(transcript_model(text).as_deref(), Some("claude-fable-5-2"));
        assert_eq!(transcript_model(""), None);
    }

    #[test]
    fn a_claude_process_is_followed_to_its_transcript() {
        let dir = std::env::temp_dir().join(format!("afterburn-models-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sessions")).unwrap();
        fs::create_dir_all(dir.join("projects/-root-code")).unwrap();
        fs::write(
            dir.join("sessions/42.json"),
            r#"{"pid":42,"sessionId":"abc-1"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("projects/-root-code/abc-1.jsonl"),
            r#"{"type":"assistant","message":{"model":"claude-opus-5-5"}}"#,
        )
        .unwrap();
        assert_eq!(claude_model(&dir, 42).as_deref(), Some("claude-opus-5-5"));
        assert_eq!(claude_model(&dir, 43), None, "no session file");
        fs::write(dir.join("sessions/44.json"), r#"{"sessionId":"../../etc"}"#).unwrap();
        assert_eq!(claude_model(&dir, 44), None, "a session id is never a path");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_names_its_model_in_its_config() {
        assert_eq!(
            config_model("# mine\nmodel = \"gpt-6.1-sol\" # fast\n[profiles.x]\nmodel = \"other\"")
                .as_deref(),
            Some("gpt-6.1-sol")
        );
        assert_eq!(config_model("[profiles.x]\nmodel = \"other\""), None);
        assert_eq!(config_model("model_provider = \"x\""), None);
    }
}
