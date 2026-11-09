//! Transcripciones de Claude Code (`~/.claude/projects/**/*.jsonl`) y las
//! invocaciones de subagentes que contienen.

use chrono::{DateTime, Utc};
use std::path::PathBuf;

/// Una llamada a la herramienta `Agent` encontrada en una transcripción.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub agent: String,
    pub cwd: PathBuf,
    pub at: DateTime<Utc>,
    /// Carpeta donde se lanzó la sesión principal a la que pertenece el
    /// fichero de la llamada (la del principal también si la llamada sale de
    /// un subagente): primer `cwd` no temporal de `<proyecto>/<sesión>.jsonl`.
    /// La pone `History`; `None` mientras no se pueda leer (se reintenta).
    pub launch_dir: Option<PathBuf>,
    /// `id` del bloque `tool_use` (`toolu_…`): coincide con el `toolUseId` del
    /// `.meta.json` de la ejecución que lanzó (ver `runs`).
    pub id: Option<String>,
    /// `description` del input de la llamada.
    pub description: Option<String>,
    /// Primera línea con contenido del `prompt`, limpia (sin `#` ni la
    /// cabecera `ENCARGO (de …):`), en una línea y sin recortar.
    pub prompt_line: Option<String>,
}

use crate::runs::cwd::{first_real_cwd, is_claude_temp, session_transcript};
use crate::runs::first_task_line;
use anyhow::Result;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;
use walkdir::WalkDir;

/// Saca las llamadas a `Agent` de una línea de transcripción. Una línea que no
/// es JSON o que no contiene ninguna devuelve un vector vacío.
pub fn parse_line(line: &str) -> Vec<Invocation> {
    let Ok(doc) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let (Some(cwd), Some(at)) = (
        doc.get("cwd").and_then(Value::as_str),
        doc.get("timestamp")
            .and_then(Value::as_str)
            .and_then(|t| t.parse::<DateTime<Utc>>().ok()),
    ) else {
        return Vec::new();
    };
    let Some(content) = doc.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter(|b| b.get("name").and_then(Value::as_str) == Some("Agent"))
        .filter_map(|b| {
            let agent = b.pointer("/input/subagent_type").and_then(Value::as_str)?;
            Some(Invocation {
                agent: agent.to_string(),
                cwd: PathBuf::from(cwd),
                at,
                id: b.get("id").and_then(Value::as_str).map(str::to_string),
                description: b
                    .pointer("/input/description")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .filter(|d| !d.trim().is_empty()),
                prompt_line: b
                    .pointer("/input/prompt")
                    .and_then(Value::as_str)
                    .and_then(first_task_line),
                launch_dir: None,
            })
        })
        .collect()
}

/// Filtro barato antes de parsear JSON: la mayoría de líneas no interesan.
fn mentions_subagent(line: &[u8]) -> bool {
    line.windows(14).any(|w| w == b"subagent_type\"")
}

/// Lo leído de una transcripción: hasta qué byte y qué llamadas salieron.
#[derive(Debug, Default)]
struct FileState {
    offset: u64,
    calls: Vec<Invocation>,
}

/// Todas las invocaciones del historial de transcripciones, leído de forma
/// incremental: la primera pasada lee cada fichero entero y las siguientes solo
/// los bytes añadidos desde la anterior (los `.jsonl` solo crecen). Un fichero
/// que encoge se relee desde el principio; uno que desaparece se olvida.
#[derive(Debug, Default)]
pub struct History {
    files: HashMap<PathBuf, FileState>,
    /// Carpeta de lanzamiento de cada transcripción principal, con su tamaño
    /// cuando se miró: encontrada no se vuelve a leer; sin encontrar, solo si
    /// el fichero ha crecido.
    launch_dirs: HashMap<PathBuf, (u64, Option<PathBuf>)>,
}

impl History {
    /// Pone al día el historial con lo escrito en `projects_dir` desde la
    /// última llamada. Un fichero que no se puede abrir se salta y se vuelve a
    /// intentar en la siguiente.
    pub fn refresh(&mut self, projects_dir: &Path) -> Result<()> {
        let mut seen = HashSet::new();
        for entry in WalkDir::new(projects_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !entry.file_type().is_file() || path.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            seen.insert(path.to_path_buf());
            let Ok(len) = entry.metadata().map(|m| m.len()) else {
                continue;
            };
            let state = self.files.entry(path.to_path_buf()).or_default();
            if len == state.offset {
                continue;
            }
            if len < state.offset {
                *state = FileState::default();
            }
            // Si falla a medias, lo ya leído queda y se sigue desde ahí.
            let _ = read_from(path, state);
            fix_temp_cwds(path, state);
        }
        self.files.retain(|p, _| seen.contains(p));
        self.launch_dirs.retain(|p, _| seen.contains(p));
        // Después del recorrido: la principal puede haber cambiado sin que lo
        // haga el fichero de la llamada (un subagente ya terminado).
        for (path, state) in &mut self.files {
            if state.calls.iter().all(|c| c.launch_dir.is_some()) {
                continue;
            }
            let main = session_transcript(path);
            if let Some(dir) = launch_dir_of(&mut self.launch_dirs, &main) {
                for c in state.calls.iter_mut() {
                    c.launch_dir = Some(dir.clone());
                }
            }
        }
        Ok(())
    }

    /// Todas las invocaciones leídas hasta ahora, sin orden fijo. Una llamada
    /// que la transcripción guardó en varias líneas (mismo `id`) sale una vez.
    pub fn invocations(&self) -> impl Iterator<Item = &Invocation> {
        let mut seen: HashSet<&str> = HashSet::new();
        self.files
            .values()
            .flat_map(|f| f.calls.iter())
            .filter(move |c| c.id.as_deref().is_none_or(|id| seen.insert(id)))
    }
}

/// Primer `cwd` no temporal de la transcripción principal `main`, con
/// `cache`. Si `main` no existe o aún no lo tiene, `None`.
fn launch_dir_of(
    cache: &mut HashMap<PathBuf, (u64, Option<PathBuf>)>,
    main: &Path,
) -> Option<PathBuf> {
    if let Some((_, Some(dir))) = cache.get(main) {
        return Some(dir.clone());
    }
    let len = std::fs::metadata(main).ok()?.len();
    if cache.get(main).is_some_and(|(seen, _)| *seen == len) {
        return None;
    }
    let dir = first_real_cwd(main);
    cache.insert(main.to_path_buf(), (len, dir.clone()));
    dir
}

/// Una llamada hecha desde la carpeta temporal de Claude toma el `cwd` real de
/// la propia transcripción (su primera línea con uno no temporal). Si no lo
/// hay (p. ej. una ejecución de subagente), queda el temporal.
fn fix_temp_cwds(path: &Path, state: &mut FileState) {
    if !state.calls.iter().any(|c| is_claude_temp(&c.cwd)) {
        return;
    }
    if let Some(real) = first_real_cwd(path) {
        for c in state.calls.iter_mut().filter(|c| is_claude_temp(&c.cwd)) {
            c.cwd = real.clone();
        }
    }
}

/// Lee `path` desde `state.offset` hasta la última línea completa. Una línea
/// sin `\n` final está a medio escribir: no se consume y se relee en la
/// siguiente pasada.
fn read_from(path: &Path, state: &mut FileState) -> std::io::Result<()> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(state.offset))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 || line.last() != Some(&b'\n') {
            return Ok(());
        }
        state.offset += n as u64;
        if mentions_subagent(&line) {
            state
                .calls
                .extend(parse_line(&String::from_utf8_lossy(&line)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = r#"{"cwd":"/Users/x/Offing/app","timestamp":"2026-11-05T11:16:17.298Z","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"voy"},{"type":"tool_use","id":"t1","name":"Agent","input":{"description":"revisar","prompt":"...","subagent_type":"docs"}}]}}"#;

    #[test]
    fn extracts_agent_cwd_and_timestamp() {
        let found = parse_line(LINE);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].agent, "docs");
        assert_eq!(found[0].cwd, PathBuf::from("/Users/x/Offing/app"));
        assert_eq!(found[0].at.to_rfc3339(), "2026-11-05T11:16:17.298+00:00");
    }

    #[test]
    fn keeps_the_description_and_the_first_clean_line_of_the_prompt() {
        let found = parse_line(LINE);
        assert_eq!(found[0].description.as_deref(), Some("revisar"));
        assert_eq!(found[0].prompt_line.as_deref(), Some("..."));
        let line = r##"{"cwd":"/p","timestamp":"2026-11-05T11:16:17Z","message":{"content":[{"type":"tool_use","name":"Agent","input":{"description":"x","prompt":"\n# ENCARGO (de Jordi, 05-11-2026):  revisa   el README\n\nmás detalle","subagent_type":"docs"}}]}}"##;
        assert_eq!(
            parse_line(line)[0].prompt_line.as_deref(),
            Some("revisa el README")
        );
        // sin description ni prompt (o vacíos) no hay nada
        let bare = r#"{"cwd":"/p","timestamp":"2026-11-05T11:16:17Z","message":{"content":[{"type":"tool_use","name":"Agent","input":{"prompt":"  ","subagent_type":"a"}}]}}"#;
        let c = &parse_line(bare)[0];
        assert_eq!(
            (c.description.as_deref(), c.prompt_line.as_deref()),
            (None, None)
        );
    }

    #[test]
    fn keeps_the_tool_use_id_to_link_the_run() {
        assert_eq!(parse_line(LINE)[0].id.as_deref(), Some("t1"));
        // Sin "id" sigue saliendo la llamada.
        let line = r#"{"cwd":"/p","timestamp":"2026-11-05T11:16:17Z","message":{"content":[{"type":"tool_use","name":"Agent","input":{"subagent_type":"a"}}]}}"#;
        let found = parse_line(line);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, None);
    }

    #[test]
    fn ignores_tool_definitions_that_mention_the_field() {
        let line = r#"{"cwd":"/p","timestamp":"2026-11-05T11:16:17Z","message":{"content":[{"type":"text","text":"\"subagent_type\":{\"description\":\"x\"}"}]}}"#;
        assert!(parse_line(line).is_empty());
    }

    #[test]
    fn ignores_other_tools_and_garbage() {
        let other = r#"{"cwd":"/p","timestamp":"2026-11-05T11:16:17Z","message":{"content":[{"type":"tool_use","name":"Bash","input":{"subagent_type":"docs"}}]}}"#;
        assert!(parse_line(other).is_empty());
        assert!(parse_line("esto no es json").is_empty());
        assert!(parse_line("").is_empty());
    }

    #[test]
    fn reports_every_agent_call_in_the_line() {
        let line = r#"{"cwd":"/p","timestamp":"2026-11-05T11:16:17Z","message":{"content":[{"type":"tool_use","name":"Agent","input":{"subagent_type":"a"}},{"type":"tool_use","name":"Agent","input":{"subagent_type":"b"}}]}}"#;
        let names: Vec<_> = parse_line(line).into_iter().map(|i| i.agent).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    /// Directorio temporal propio de cada test; se borra al soltarlo.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("claude-agents-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("proj")).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn call(agent: &str, ts: &str) -> String {
        format!(
            r#"{{"cwd":"/p","timestamp":"{ts}","message":{{"content":[{{"type":"tool_use","name":"Agent","input":{{"subagent_type":"{agent}"}}}}]}}}}"#
        )
    }

    fn append(path: &Path, text: &str) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn names(h: &History) -> Vec<String> {
        let mut v: Vec<_> = h.invocations().map(|i| i.agent.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn history_first_pass_reads_every_file_whatever_its_age() {
        let tmp = TempDir::new("first");
        let a = tmp.0.join("proj/a.jsonl");
        append(
            &a,
            &format!(
                "{}\nbasura\n{}\n",
                call("docs", "2020-01-01T00:00:00Z"),
                call("data", "2026-11-05T10:00:00Z")
            ),
        );
        append(
            &tmp.0.join("proj/notas.txt"),
            &format!("{}\n", call("otro", "2026-11-05T10:00:00Z")),
        );
        let mut h = History::default();
        h.refresh(&tmp.0).unwrap();
        assert_eq!(names(&h), vec!["data", "docs"]);
    }

    #[test]
    fn history_reads_only_what_was_appended() {
        let tmp = TempDir::new("append");
        let a = tmp.0.join("proj/a.jsonl");
        append(&a, &format!("{}\n", call("docs", "2026-11-05T10:00:00Z")));
        let mut h = History::default();
        h.refresh(&tmp.0).unwrap();
        append(&a, &format!("{}\n", call("data", "2026-11-05T10:01:00Z")));
        h.refresh(&tmp.0).unwrap();
        // Sin duplicar lo ya leído.
        assert_eq!(names(&h), vec!["data", "docs"]);
        h.refresh(&tmp.0).unwrap();
        assert_eq!(names(&h), vec!["data", "docs"]);
    }

    #[test]
    fn history_counts_a_call_repeated_with_the_same_id_once() {
        let tmp = TempDir::new("dupid");
        let a = tmp.0.join("proj/a.jsonl");
        // La transcripción guarda a veces la misma llamada en dos líneas.
        append(&a, &format!("{LINE}\n{LINE}\n"));
        let mut h = History::default();
        h.refresh(&tmp.0).unwrap();
        assert_eq!(h.invocations().count(), 1);
    }

    #[test]
    fn history_rereads_a_file_that_shrank() {
        let tmp = TempDir::new("shrink");
        let a = tmp.0.join("proj/a.jsonl");
        append(
            &a,
            &format!(
                "{}\n{}\n",
                call("docs", "2026-11-05T10:00:00Z"),
                call("data", "2026-11-05T10:01:00Z")
            ),
        );
        let mut h = History::default();
        h.refresh(&tmp.0).unwrap();
        std::fs::write(&a, format!("{}\n", call("backend", "2026-11-05T10:02:00Z"))).unwrap();
        h.refresh(&tmp.0).unwrap();
        assert_eq!(names(&h), vec!["backend"]);
    }

    #[test]
    fn history_waits_for_a_half_written_line() {
        let tmp = TempDir::new("half");
        let a = tmp.0.join("proj/a.jsonl");
        let line = call("docs", "2026-11-05T10:00:00Z");
        let (head, tail) = line.split_at(line.len() / 2);
        append(
            &a,
            &format!("{}\n{head}", call("data", "2026-11-05T09:00:00Z")),
        );
        let mut h = History::default();
        h.refresh(&tmp.0).unwrap();
        assert_eq!(names(&h), vec!["data"]);
        append(&a, tail);
        h.refresh(&tmp.0).unwrap();
        assert_eq!(names(&h), vec!["data"]);
        append(&a, "\n");
        h.refresh(&tmp.0).unwrap();
        assert_eq!(names(&h), vec!["data", "docs"]);
    }

    #[test]
    fn history_picks_up_new_files_and_forgets_deleted_ones() {
        let tmp = TempDir::new("new");
        let a = tmp.0.join("proj/a.jsonl");
        append(&a, &format!("{}\n", call("docs", "2026-11-05T10:00:00Z")));
        let mut h = History::default();
        h.refresh(&tmp.0).unwrap();
        std::fs::create_dir_all(tmp.0.join("otro/sub")).unwrap();
        append(
            &tmp.0.join("otro/sub/b.jsonl"),
            &format!("{}\n", call("data", "2026-11-05T10:01:00Z")),
        );
        h.refresh(&tmp.0).unwrap();
        assert_eq!(names(&h), vec!["data", "docs"]);
        std::fs::remove_file(&a).unwrap();
        h.refresh(&tmp.0).unwrap();
        assert_eq!(names(&h), vec!["data"]);
    }

    fn launches(h: &History) -> Vec<(String, PathBuf, Option<PathBuf>)> {
        let mut v: Vec<_> = h
            .invocations()
            .map(|i| (i.agent.clone(), i.cwd.clone(), i.launch_dir.clone()))
            .collect();
        v.sort();
        v
    }

    fn call_at(agent: &str, cwd: &str) -> String {
        call(agent, "2026-11-05T10:00:00Z").replace("\"/p\"", &format!("\"{cwd}\""))
    }

    #[test]
    fn a_call_keeps_its_cwd_and_takes_the_launch_dir_of_its_main_session() {
        let tmp = TempDir::new("launch");
        // El principal arranca en la temporal de Claude, pasa a /Users/x/app y
        // luego hace `cd` a src/css antes de lanzar.
        let main = tmp.0.join("proj/s1.jsonl");
        let first = r#"{"cwd":"/private/tmp/claude-501/-Users-x-app/s1","timestamp":"2026-11-05T09:00:00Z"}"#;
        let real = r#"{"cwd":"/Users/x/app","timestamp":"2026-11-05T09:00:01Z"}"#;
        append(
            &main,
            &format!(
                "{first}\n{real}\n{}\n",
                call_at("docs", "/Users/x/app/src/css")
            ),
        );
        // Un subagente de esa sesión (y un paso de workflow) que lanza otro:
        // carpeta del principal, no el primer cwd del propio subagente.
        let sub = tmp.0.join("proj/s1/subagents/agent-a1.jsonl");
        let wf = tmp
            .0
            .join("proj/s1/subagents/workflows/wf_1/agent-b1.jsonl");
        std::fs::create_dir_all(wf.parent().unwrap()).unwrap();
        append(&sub, &format!("{}\n", call_at("data", "/Users/x/app/src")));
        append(&wf, &format!("{}\n", call_at("visual", "/tmp")));
        let mut h = History::default();
        h.refresh(&tmp.0).unwrap();
        let app = Some(PathBuf::from("/Users/x/app"));
        assert_eq!(
            launches(&h),
            vec![
                (
                    "data".into(),
                    PathBuf::from("/Users/x/app/src"),
                    app.clone()
                ),
                (
                    "docs".into(),
                    PathBuf::from("/Users/x/app/src/css"),
                    app.clone()
                ),
                ("visual".into(), PathBuf::from("/tmp"), app),
            ]
        );
    }

    #[test]
    fn without_its_main_session_a_call_has_no_launch_dir_until_it_appears() {
        let tmp = TempDir::new("nolaunch");
        let sub = tmp.0.join("proj/s1/subagents/agent-a1.jsonl");
        std::fs::create_dir_all(sub.parent().unwrap()).unwrap();
        append(&sub, &format!("{}\n", call_at("data", "/Users/x/app/src")));
        let mut h = History::default();
        h.refresh(&tmp.0).unwrap();
        assert_eq!(launches(&h)[0].2, None);
        // Solo con cwd temporal tampoco vale.
        let main = tmp.0.join("proj/s1.jsonl");
        append(
            &main,
            "{\"cwd\":\"/private/tmp/claude-501/-Users-x-app/s1\"}\n",
        );
        h.refresh(&tmp.0).unwrap();
        assert_eq!(launches(&h)[0].2, None);
        // Aparece el cwd real en el principal: se recoge aunque el fichero del
        // subagente no haya cambiado.
        append(&main, "{\"cwd\":\"/Users/x/app\"}\n");
        h.refresh(&tmp.0).unwrap();
        assert_eq!(launches(&h)[0].2, Some(PathBuf::from("/Users/x/app")));
    }

    #[test]
    fn parse_line_leaves_the_launch_dir_to_the_history() {
        assert_eq!(parse_line(LINE)[0].launch_dir, None);
    }

    #[test]
    fn a_call_made_from_claude_temp_takes_the_real_cwd_of_its_session() {
        let tmp = TempDir::new("tempcwd");
        let a = tmp.0.join("proj/a.jsonl");
        let temp_call = call("docs", "2026-11-05T10:00:00Z")
            .replace("\"/p\"", "\"/private/tmp/claude-501/-Users-x-app/s1\"");
        let first = r#"{"cwd":"/Users/x/app","timestamp":"2026-11-05T09:00:00Z"}"#;
        append(&a, &format!("{first}\n{temp_call}\n"));
        // Sin cwd real en el fichero, queda el temporal.
        let b = tmp.0.join("proj/b.jsonl");
        append(&b, &format!("{temp_call}\n"));
        let mut h = History::default();
        h.refresh(&tmp.0).unwrap();
        let mut cwds: Vec<_> = h.invocations().map(|i| i.cwd.clone()).collect();
        cwds.sort();
        assert_eq!(
            cwds,
            vec![
                PathBuf::from("/Users/x/app"),
                PathBuf::from("/private/tmp/claude-501/-Users-x-app/s1")
            ]
        );
    }
}
