//! Directorio real de una ejecución cuyo `cwd` es la carpeta temporal de Claude.
//!
//! Algunas ejecuciones arrancan con `cwd` en `/private/tmp/claude-<uid>/<proyecto>/<sesión>`.
//! El directorio de trabajo de verdad es el `cwd` de la transcripción de la sesión
//! padre (`<proyecto>/<sesión>.jsonl`), de la que solo se leen las primeras líneas.
//! De esas mismas líneas sale el primer prompt de la sesión («lanzada por»).

use super::text::{command_of, session_prompt_of};
use super::Runs;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};

/// Líneas y bytes máximos que se miran del padre hasta el primer `cwd` válido.
const MAX_LINES: usize = 200;
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// `/tmp/claude-<uid>/…` o `/private/tmp/claude-<uid>/…`, con `<uid>` numérico.
pub fn is_claude_temp(path: &Path) -> bool {
    let mut parts = path.components().filter_map(|c| match c {
        Component::Normal(s) => s.to_str(),
        _ => None,
    });
    let mut first = parts.next();
    if first == Some("private") {
        first = parts.next();
    }
    if first != Some("tmp") {
        return false;
    }
    parts
        .next()
        .and_then(|s| s.strip_prefix("claude-"))
        .is_some_and(|uid| !uid.is_empty() && uid.bytes().all(|b| b.is_ascii_digit()))
}

/// Primer `cwd` no temporal de las primeras líneas de `parent`.
pub fn first_real_cwd(parent: &Path) -> Option<PathBuf> {
    let file = File::open(parent).ok()?;
    let mut reader = BufReader::new(file.take(MAX_BYTES));
    let mut line = Vec::new();
    for _ in 0..MAX_LINES {
        line.clear();
        if reader.read_until(b'\n', &mut line).ok()? == 0 {
            return None;
        }
        let text = String::from_utf8_lossy(&line);
        if !text.contains("\"cwd\"") {
            continue;
        }
        let Ok(doc) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if let Some(cwd) = doc.get("cwd").and_then(Value::as_str) {
            let cwd = PathBuf::from(cwd);
            if !is_claude_temp(&cwd) {
                return Some(cwd);
            }
        }
    }
    None
}

/// Lo que interesa del principio de la transcripción de una sesión.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct SessionHead {
    /// Primer `cwd` no temporal.
    cwd: Option<PathBuf>,
    /// Primer prompt de verdad (`session_prompt_of`) o, si no hay, el primer
    /// comando de barra (`/review 123`).
    prompt: Option<String>,
    command: Option<String>,
}

impl SessionHead {
    fn complete(&self) -> bool {
        self.cwd.is_some() && self.prompt.is_some()
    }
}

/// Las primeras líneas de `parent` (`MAX_LINES`, `MAX_BYTES`), hasta tener
/// `cwd` y prompt. Solo se parsean las que pueden traer alguno de los dos.
fn read_head(parent: &Path) -> Option<SessionHead> {
    let file = File::open(parent).ok()?;
    let mut reader = BufReader::new(file.take(MAX_BYTES));
    let mut head = SessionHead::default();
    let mut line = Vec::new();
    for _ in 0..MAX_LINES {
        line.clear();
        if reader.read_until(b'\n', &mut line).ok()? == 0 {
            break;
        }
        let text = String::from_utf8_lossy(&line);
        let want_cwd = head.cwd.is_none() && text.contains("\"cwd\"");
        let want_prompt = head.prompt.is_none() && text.contains("\"user\"");
        if !want_cwd && !want_prompt {
            continue;
        }
        let Ok(doc) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if want_cwd {
            head.cwd = doc
                .get("cwd")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .filter(|c| !is_claude_temp(c));
        }
        if want_prompt {
            head.prompt = session_prompt_of(&doc);
            if head.command.is_none() {
                head.command = command_of(&doc);
            }
        }
        if head.complete() {
            break;
        }
    }
    Some(head)
}

/// Directorio de la sesión de una ejecución: el que contiene el `subagents`
/// más alto de `…/<proyecto>/<sesión>/subagents/…/agent-*.jsonl`.
fn session_dir(run: &Path) -> Option<&Path> {
    run.ancestors()
        .filter(|a| a.file_name().is_some_and(|n| n == "subagents"))
        .last()?
        .parent()
}

/// Transcripción principal de la sesión a la que pertenece `path`: la de
/// `<proyecto>/<sesión>/subagents/**/agent-*.jsonl` es `<proyecto>/<sesión>.jsonl`;
/// cualquier otra es ella misma.
pub fn session_transcript(path: &Path) -> PathBuf {
    let main = session_dir(path).and_then(|dir| {
        let name = dir.file_name()?.to_str()?;
        Some(dir.with_file_name(format!("{name}.jsonl")))
    });
    main.unwrap_or_else(|| path.to_path_buf())
}

/// Transcripción padre de una ejecución: `<proyecto>/<sesión>.jsonl`.
fn parent_transcript(run: &Path, session_id: &str) -> Option<PathBuf> {
    let project = session_dir(run)?.parent()?;
    Some(project.join(format!("{session_id}.jsonl")))
}

/// Cabecera de cada sesión, con el tamaño de su transcripción cuando se leyó:
/// completa no se vuelve a leer; incompleta, solo si el fichero ha crecido.
#[derive(Debug, Default)]
pub struct SessionCwds(HashMap<String, (u64, SessionHead)>);

impl SessionCwds {
    fn head(&mut self, run: &Path, session: &str) -> SessionHead {
        if let Some((_, head)) = self.0.get(session).filter(|(_, h)| h.complete()) {
            return head.clone();
        }
        let Some(parent) = parent_transcript(run, session) else {
            return SessionHead::default();
        };
        let old = self.0.get(session);
        // Lo ya encontrado no se pierde aunque la sesión desaparezca o cambie.
        let Ok(len) = std::fs::metadata(&parent).map(|m| m.len()) else {
            return old.map(|(_, h)| h.clone()).unwrap_or_default();
        };
        if let Some((_, head)) = old.filter(|(seen, _)| *seen == len) {
            return head.clone();
        }
        let mut head = read_head(&parent).unwrap_or_default();
        if let Some((_, prev)) = old {
            head.cwd = head.cwd.or_else(|| prev.cwd.clone());
            head.prompt = head.prompt.or_else(|| prev.prompt.clone());
            head.command = head.command.or_else(|| prev.command.clone());
        }
        self.0.insert(session.to_string(), (len, head.clone()));
        head
    }
}

impl Runs {
    /// Con la cabecera de la sesión padre: apunta su primer `cwd` real como
    /// carpeta de lanzamiento (`launch_dir`), cambia el `cwd` temporal de Claude
    /// por el real (si aún no se puede leer, queda el temporal y se reintenta
    /// en la próxima pasada) y apunta el primer prompt de la sesión.
    pub(super) fn resolve_sessions(&mut self) {
        for (path, state) in self.files.iter_mut() {
            let t = &mut state.transcript;
            let dir_name = session_dir(path)
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str());
            let Some(session) = t.session_id.as_deref().or(dir_name) else {
                continue;
            };
            let head = self.session_cwds.head(path, session);
            t.launch_dir = head.cwd.clone();
            t.session_prompt = head.prompt.or(head.command);
            if let Some(cwd) = t.cwd.take() {
                if t.temp_cwd.is_none() && is_claude_temp(&cwd) {
                    t.temp_cwd = Some(cwd.clone());
                }
                t.cwd = Some(match head.cwd {
                    Some(real) if is_claude_temp(&cwd) => real,
                    _ => cwd,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_claude_temp_dirs_for_any_uid() {
        for p in [
            "/private/tmp/claude-501/-Users-x-app/sess1",
            "/tmp/claude-1000/-Users-x-app/sess1",
            "/tmp/claude-0",
        ] {
            assert!(is_claude_temp(Path::new(p)), "{p}");
        }
        for p in [
            "/Users/x/app",
            "/tmp/other",
            "/tmp/claude-abc/x",
            "/tmp/claude-/x",
            "/Users/x/tmp/claude-501",
            "/private/var/claude-501",
        ] {
            assert!(!is_claude_temp(Path::new(p)), "{p}");
        }
    }

    const PARENT: &str = r#"{"type":"queue-operation","sessionId":"sess1"}
{"cwd":"/Users/x/app","sessionId":"sess1","type":"user"}
"#;

    fn run_line(cwd: &str) -> String {
        format!(
            r#"{{"cwd":"{cwd}","sessionId":"sess1","timestamp":"2026-11-05T10:00:00Z","message":{{"content":[]}}}}"#
        ) + "\n"
    }

    /// Proyecto con una ejecución en `sess1`; devuelve (raíz, ejecución).
    fn fixture(name: &str, run_cwd: &str, parent: Option<&str>) -> (PathBuf, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("claude-agents-cwd-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let run = root.join("proj/sess1/subagents/agent-a1.jsonl");
        std::fs::create_dir_all(run.parent().unwrap()).unwrap();
        std::fs::write(&run, run_line(run_cwd)).unwrap();
        std::fs::write(run.with_extension("meta.json"), r#"{"agentType":"kiosk"}"#).unwrap();
        if let Some(text) = parent {
            std::fs::write(root.join("proj/sess1.jsonl"), text).unwrap();
        }
        (root, run)
    }

    fn cwd_of(runs: &Runs) -> Option<PathBuf> {
        runs.runs().remove(0).cwd
    }

    #[test]
    fn temp_cwd_is_replaced_by_the_parent_session_cwd() {
        for temp in [
            "/private/tmp/claude-501/-Users-x-app/sess1",
            "/tmp/claude-1000/-Users-x-app/sess1",
        ] {
            let (root, _) = fixture("temp", temp, Some(PARENT));
            let mut runs = Runs::default();
            runs.refresh(&root).unwrap();
            assert_eq!(cwd_of(&runs), Some(PathBuf::from("/Users/x/app")), "{temp}");
            assert_eq!(runs.runs()[0].temp_cwd, Some(PathBuf::from(temp)));
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn a_normal_cwd_is_left_alone() {
        let (root, _) = fixture("normal", "/Users/x/other", Some(PARENT));
        let mut runs = Runs::default();
        runs.refresh(&root).unwrap();
        assert_eq!(cwd_of(&runs), Some(PathBuf::from("/Users/x/other")));
        assert_eq!(runs.runs()[0].temp_cwd, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn without_a_readable_parent_the_temp_cwd_stays_and_is_retried() {
        let temp = "/private/tmp/claude-501/-Users-x-app/sess1";
        let (root, _) = fixture("noparent", temp, None);
        let mut runs = Runs::default();
        runs.refresh(&root).unwrap();
        assert_eq!(cwd_of(&runs), Some(PathBuf::from(temp)));
        assert_eq!(runs.runs()[0].temp_cwd, Some(PathBuf::from(temp)));
        // Un padre con solo cwd temporal tampoco sirve.
        std::fs::write(root.join("proj/sess1.jsonl"), run_line(temp)).unwrap();
        runs.refresh(&root).unwrap();
        assert_eq!(cwd_of(&runs), Some(PathBuf::from(temp)));
        // Cuando el padre ya tiene cwd real, la siguiente pasada lo recoge.
        std::fs::write(root.join("proj/sess1.jsonl"), PARENT).unwrap();
        runs.refresh(&root).unwrap();
        assert_eq!(cwd_of(&runs), Some(PathBuf::from("/Users/x/app")));
        assert_eq!(runs.runs()[0].temp_cwd, Some(PathBuf::from(temp)));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn launch_of(runs: &Runs, id: &str) -> Option<PathBuf> {
        runs.runs().into_iter().find(|r| r.id == id)?.launch_dir
    }

    #[test]
    fn the_launch_dir_is_the_first_cwd_of_the_main_session() {
        // El principal hizo `cd` a src/css antes de lanzarlo: el cwd del
        // subagente se conserva, la carpeta de lanzamiento es la del principal.
        let parent = r#"{"type":"queue-operation","sessionId":"sess1"}
{"cwd":"/private/tmp/claude-501/-Users-x-app/sess1","sessionId":"sess1","type":"user"}
{"cwd":"/Users/x/app","sessionId":"sess1","type":"user"}
{"cwd":"/Users/x/app/src/css","sessionId":"sess1","type":"assistant"}
"#;
        let (root, _) = fixture("launch", "/Users/x/app/src/css", Some(parent));
        let mut runs = Runs::default();
        runs.refresh(&root).unwrap();
        assert_eq!(cwd_of(&runs), Some(PathBuf::from("/Users/x/app/src/css")));
        assert_eq!(launch_of(&runs, "a1"), Some(PathBuf::from("/Users/x/app")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nested_and_workflow_runs_share_the_launch_dir_of_their_session() {
        let (root, _) = fixture("nested", "/Users/x/app/src", Some(PARENT));
        // Lanzada por a1 (misma carpeta `subagents`) y un paso de workflow.
        let nested = root.join("proj/sess1/subagents/agent-b1.jsonl");
        let wf = root.join("proj/sess1/subagents/workflows/wf_1/agent-c1.jsonl");
        std::fs::create_dir_all(wf.parent().unwrap()).unwrap();
        for (path, cwd) in [(&nested, "/tmp/other"), (&wf, "/Users/x/app/docs")] {
            std::fs::write(path, run_line(cwd)).unwrap();
            std::fs::write(path.with_extension("meta.json"), r#"{"agentType":"docs"}"#).unwrap();
        }
        let mut runs = Runs::default();
        runs.refresh(&root).unwrap();
        for id in ["a1", "b1", "c1"] {
            assert_eq!(
                launch_of(&runs, id),
                Some(PathBuf::from("/Users/x/app")),
                "{id}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn without_a_readable_parent_there_is_no_launch_dir_until_it_appears() {
        let (root, _) = fixture("nolaunch", "/Users/x/app/src", None);
        let mut runs = Runs::default();
        runs.refresh(&root).unwrap();
        assert_eq!(launch_of(&runs, "a1"), None);
        assert_eq!(cwd_of(&runs), Some(PathBuf::from("/Users/x/app/src")));
        std::fs::write(root.join("proj/sess1.jsonl"), PARENT).unwrap();
        runs.refresh(&root).unwrap();
        assert_eq!(launch_of(&runs, "a1"), Some(PathBuf::from("/Users/x/app")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_session_cwd_is_cached() {
        let temp = "/private/tmp/claude-501/-Users-x-app/sess1";
        let (root, _) = fixture("cache", temp, Some(PARENT));
        let mut runs = Runs::default();
        runs.refresh(&root).unwrap();
        // Sin padre y con una ejecución nueva de la misma sesión: sale de la caché.
        std::fs::remove_file(root.join("proj/sess1.jsonl")).unwrap();
        let second = root.join("proj/sess1/subagents/agent-a2.jsonl");
        std::fs::write(&second, run_line(temp)).unwrap();
        std::fs::write(
            second.with_extension("meta.json"),
            r#"{"agentType":"docs"}"#,
        )
        .unwrap();
        runs.refresh(&root).unwrap();
        for r in runs.runs() {
            assert_eq!(
                r.cwd,
                Some(PathBuf::from("/Users/x/app")),
                "{}",
                r.agent_type
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
