//! Historial guardado de ejecuciones terminadas.
//!
//! Claude Code borra las transcripciones viejas (`cleanupPeriodDays`, 30 días
//! por defecto). Para que las ejecuciones terminadas no desaparezcan con
//! ellas, se guardan en `runs.jsonl` (un objeto JSON por ejecución) bajo
//! `$XDG_CACHE_HOME/claude-agents/` o `~/.cache/claude-agents/`. Es lo único
//! que la herramienta escribe en disco.
//!
//! - Al leer, manda la transcripción: el historial solo añade las ejecuciones
//!   que ya no tienen (`from_cache`), sin pasos.
//! - Se reescribe entero y de forma atómica (temporal + `rename`), y antes se
//!   vuelve a leer y se une con lo que haya, por si otra instancia escribió.
//! - Nada de esto falla: si no se puede leer o escribir, se sigue sin él. Si
//!   existe pero no se puede leer, no se escribe (no se pisa lo que hay).

use super::usage::{Tokens, Usage};
use super::Run;
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Versión del formato de cada línea; una línea de otra versión se ignora.
const VERSION: u64 = 1;

/// `$XDG_CACHE_HOME/claude-agents/runs.jsonl` si la variable es una ruta
/// absoluta; si no, `~/.cache/claude-agents/runs.jsonl`.
pub fn default_path() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    path_from(xdg, home)
}

fn path_from(xdg: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let base = xdg
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".cache")))?;
    Some(base.join("claude-agents").join("runs.jsonl"))
}

/// El historial en memoria y en disco.
#[derive(Debug)]
pub(super) struct Kept {
    path: PathBuf,
    loaded: bool,
    /// `false` si el fichero existe pero no se pudo leer.
    writable: bool,
    runs: HashMap<String, Run>,
    /// Huella de cada ejecución la última vez que se miró, para no
    /// compararla entera en cada pasada.
    seen: HashMap<String, Fingerprint>,
}

type Fingerprint = (
    DateTime<Utc>,
    usize,
    Option<bool>,
    usize,
    Option<PathBuf>,
    Option<PathBuf>,
);

fn fingerprint(r: &Run) -> Fingerprint {
    (
        r.modified,
        r.tools,
        r.finished,
        r.children.len(),
        r.cwd.clone(),
        r.launch_dir.clone(),
    )
}

impl Kept {
    pub(super) fn new(path: PathBuf) -> Self {
        Kept {
            path,
            loaded: false,
            writable: true,
            runs: HashMap::new(),
            seen: HashMap::new(),
        }
    }

    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        match read(&self.path) {
            Ok(runs) => self.runs = runs,
            Err(_) => self.writable = false,
        }
    }

    /// Guarda las ejecuciones de `runs` que no están en curso a fecha de
    /// `now` y son nuevas o han cambiado; reescribe el fichero si hay alguna.
    pub(super) fn keep(&mut self, runs: &[Run], now: DateTime<Utc>) {
        self.load();
        let mut changed = false;
        for r in runs {
            let print = fingerprint(r);
            if self.seen.get(&r.id) == Some(&print) {
                continue;
            }
            if r.in_progress(now) || r.start.is_none() || r.id.is_empty() {
                continue;
            }
            self.seen.insert(r.id.clone(), print);
            let stored = stored(r);
            if self.runs.get(&r.id) != Some(&stored) {
                self.runs.insert(r.id.clone(), stored);
                changed = true;
            }
        }
        if changed && self.writable {
            // Lo que otra instancia haya guardado y aquí no esté, se conserva.
            if let Ok(on_disk) = read(&self.path) {
                for (id, r) in on_disk {
                    self.runs.entry(id).or_insert(r);
                }
            }
            let _ = write(&self.path, &self.runs);
        }
    }

    /// Las guardadas cuyo id no está en `present`.
    pub(super) fn cached_runs<'a>(
        &'a self,
        present: &'a HashSet<String>,
    ) -> impl Iterator<Item = Run> + 'a {
        self.runs
            .values()
            .filter(|r| !present.contains(&r.id))
            .cloned()
    }

    pub(super) fn only_cached(&self, present: &HashSet<String>) -> usize {
        self.runs.keys().filter(|id| !present.contains(*id)).count()
    }
}

/// La ejecución tal como queda guardada: sin pasos ni nada de la lectura en
/// curso, y marcada como del historial.
fn stored(r: &Run) -> Run {
    Run {
        temp_cwd: None,
        last_tool: None,
        skill: None,
        steps: Arc::from([]),
        from_cache: true,
        ..r.clone()
    }
}

/// Lee el historial. Que no exista no es error (vacío); las líneas rotas o de
/// otra versión se saltan; con un `id` repetido gana la última línea.
fn read(path: &Path) -> std::io::Result<HashMap<String, Run>> {
    let text = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(e),
    };
    Ok(text
        .lines()
        .filter_map(from_line)
        .map(|r| (r.id.clone(), r))
        .collect())
}

/// Reescribe el historial entero, ordenado por inicio, en un temporal al lado
/// que luego se renombra encima: o queda el viejo o el nuevo, nunca a medias.
fn write(path: &Path, runs: &HashMap<String, Run>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut sorted: Vec<_> = runs.values().collect();
    sorted.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    let mut text = String::new();
    for r in sorted {
        text.push_str(&to_line(r));
        text.push('\n');
    }
    let tmp = path.with_extension(format!("jsonl.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn ms(d: TimeDelta) -> i64 {
    d.num_milliseconds()
}

fn stamp(t: &DateTime<Utc>) -> String {
    t.to_rfc3339()
}

/// Una ejecución como línea del historial.
fn to_line(r: &Run) -> String {
    let t = &r.usage.tokens;
    json!({
        "v": VERSION,
        "id": r.id,
        "agent_type": r.agent_type,
        "description": r.description,
        "tool_use_id": r.tool_use_id,
        "session_id": r.session_id,
        "session_prompt": r.session_prompt,
        "parent_run": r.parent_run,
        "children": r.children,
        "cwd": r.cwd,
        "launch_dir": r.launch_dir,
        "start": r.start.as_ref().map(stamp),
        "last": r.last.as_ref().map(stamp),
        "modified": stamp(&r.modified),
        "finished": r.finished,
        "worked_ms": ms(r.worked),
        "worked_slots": r.worked_slots.iter().map(|(s, d)| json!([stamp(s), ms(*d)])).collect::<Vec<_>>(),
        "tools": r.tools,
        "tool_counts": r.tool_counts.iter().map(|(n, c)| json!([n, c])).collect::<Vec<_>>(),
        "task": r.task,
        "task_full": r.task_full,
        "tokens": {
            "input": t.input,
            "output": t.output,
            "cache_read": t.cache_read,
            "cache_creation": t.cache_creation,
        },
        "model": r.usage.model,
        "context": r.usage.context,
        "context_window": r.usage.context_window,
    })
    .to_string()
}

/// Una línea del historial como ejecución; `None` si no es de esta versión o
/// le falta `id`, `agent_type` o `start`.
fn from_line(line: &str) -> Option<Run> {
    let doc: Map<String, Value> = serde_json::from_str(line).ok()?;
    if doc.get("v").and_then(Value::as_u64) != Some(VERSION) {
        return None;
    }
    let text = |k: &str| doc.get(k).and_then(Value::as_str).map(str::to_string);
    let time = |v: &Value| v.as_str().and_then(|s| s.parse::<DateTime<Utc>>().ok());
    let at = |k: &str| doc.get(k).and_then(time);
    let num = |v: Option<&Value>| v.and_then(Value::as_u64).unwrap_or(0);
    let delta = |v: Option<&Value>| TimeDelta::milliseconds(v.and_then(Value::as_i64).unwrap_or(0));
    let pairs = |k: &str| {
        doc.get(k)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| Some((p.get(0)?, p.get(1)?)))
    };
    let tokens = doc.get("tokens");
    let token = |k: &str| num(tokens.and_then(|t| t.get(k)));
    Some(Run {
        id: text("id")?,
        agent_type: text("agent_type")?,
        description: text("description"),
        tool_use_id: text("tool_use_id"),
        session_id: text("session_id"),
        session_prompt: text("session_prompt"),
        parent_run: text("parent_run"),
        children: doc
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_str().map(str::to_string))
            .collect(),
        cwd: text("cwd").map(PathBuf::from),
        // Las entradas anteriores a este campo no lo traen: `None`.
        launch_dir: text("launch_dir").map(PathBuf::from),
        start: Some(at("start")?),
        last: at("last"),
        modified: at("modified").unwrap_or_default(),
        finished: doc.get("finished").and_then(Value::as_bool),
        worked: delta(doc.get("worked_ms")),
        worked_slots: pairs("worked_slots")
            .filter_map(|(s, d)| Some((time(s)?, delta(Some(d)))))
            .collect::<BTreeMap<_, _>>(),
        tools: num(doc.get("tools")) as usize,
        tool_counts: pairs("tool_counts")
            .filter_map(|(n, c)| Some((n.as_str()?.to_string(), c.as_u64()? as usize)))
            .collect::<Vec<_>>()
            .into(),
        task: text("task"),
        task_full: text("task_full"),
        usage: Usage {
            tokens: Tokens {
                input: token("input"),
                output: token("output"),
                cache_read: token("cache_read"),
                cache_creation: token("cache_creation"),
            },
            model: text("model"),
            context: doc.get("context").and_then(Value::as_u64),
            context_window: num(doc.get("context_window")),
        },
        from_cache: true,
        ..Run::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::{Runs, ToolStep};

    fn at(t: &str) -> DateTime<Utc> {
        t.parse().unwrap()
    }

    /// Directorio temporal propio de cada test; se borra al soltarlo.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "claude-agents-history-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn done(id: &str) -> Run {
        let start = at("2026-11-05T10:00:00Z");
        Run {
            id: id.into(),
            agent_type: "docs".into(),
            description: Some("Docs: README".into()),
            tool_use_id: Some(format!("toolu_{id}")),
            session_id: Some("s1".into()),
            session_prompt: Some("mejora el README".into()),
            parent_run: Some("p0".into()),
            children: vec!["k1".into()],
            cwd: Some(PathBuf::from("/Users/x/app/src")),
            launch_dir: Some(PathBuf::from("/Users/x/app")),
            start: Some(start),
            last: Some(at("2026-11-05T10:03:00Z")),
            modified: at("2026-11-05T10:03:00Z"),
            finished: Some(true),
            worked: TimeDelta::milliseconds(180_500),
            worked_slots: BTreeMap::from([(start, TimeDelta::milliseconds(180_500))]),
            tools: 3,
            tool_counts: vec![("Read".to_string(), 2), ("Bash".to_string(), 1)].into(),
            task: Some("README".into()),
            task_full: Some("README\ncon detalle".into()),
            usage: Usage {
                tokens: Tokens {
                    input: 1,
                    output: 2,
                    cache_read: 3,
                    cache_creation: 4,
                },
                model: Some("claude-opus-5-5".into()),
                context: Some(8),
                context_window: 200_000,
            },
            ..Run::default()
        }
    }

    #[test]
    fn the_path_follows_xdg_or_home() {
        let p = |x: Option<&str>, h: Option<&str>| {
            path_from(x.map(PathBuf::from), h.map(PathBuf::from))
        };
        assert_eq!(
            p(Some("/c"), Some("/h")),
            Some(PathBuf::from("/c/claude-agents/runs.jsonl"))
        );
        // Relativa no vale (lo dice la especificación XDG).
        assert_eq!(
            p(Some("rel"), Some("/h")),
            Some(PathBuf::from("/h/.cache/claude-agents/runs.jsonl"))
        );
        assert_eq!(p(None, None), None);
    }

    #[test]
    fn a_run_survives_the_round_trip_without_its_steps() {
        let r = Run {
            steps: vec![ToolStep {
                at: at("2026-11-05T10:01:00Z"),
                name: "Read".into(),
                arg: None,
            }]
            .into(),
            last_tool: Some("Read".into()),
            skill: Some("superpowers:brainstorming".into()),
            temp_cwd: Some(PathBuf::from("/tmp/claude-501/x")),
            ..done("a1")
        };
        let back = from_line(&to_line(&r)).unwrap();
        assert_eq!(back, stored(&r));
        assert!(back.from_cache && back.steps.is_empty() && back.last_tool.is_none());
        assert_eq!(back.skill, None);
    }

    #[test]
    fn an_old_line_without_launch_dir_reads_as_none() {
        let old = r#"{"v":1,"id":"a1","agent_type":"docs","cwd":"/Users/x/app/src","start":"2026-11-05T10:00:00Z"}"#;
        let r = from_line(old).unwrap();
        assert_eq!(r.cwd, Some(PathBuf::from("/Users/x/app/src")));
        assert_eq!(r.launch_dir, None);
        let back = from_line(&to_line(&done("a1"))).unwrap();
        assert_eq!(back.launch_dir, Some(PathBuf::from("/Users/x/app")));
    }

    #[test]
    fn a_run_is_kept_again_when_its_launch_dir_appears() {
        let tmp = TempDir::new("launch");
        let path = tmp.0.join("runs.jsonl");
        let now = at("2026-11-06T00:00:00Z");
        let mut kept = Kept::new(path.clone());
        let unresolved = Run {
            launch_dir: None,
            ..done("a1")
        };
        kept.keep(&[unresolved], now);
        assert_eq!(read(&path).unwrap()["a1"].launch_dir, None);
        kept.keep(&[done("a1")], now);
        assert_eq!(
            read(&path).unwrap()["a1"].launch_dir,
            Some(PathBuf::from("/Users/x/app"))
        );
    }

    #[test]
    fn broken_or_foreign_lines_are_skipped_and_the_last_line_wins() {
        let tmp = TempDir::new("read");
        let path = tmp.0.join("runs.jsonl");
        let newer = Run {
            tools: 9,
            ..done("a1")
        };
        let text = [
            to_line(&done("a1")),
            "no es json".to_string(),
            r#"{"v":99,"id":"x","agent_type":"docs","start":"2026-11-05T10:00:00Z"}"#.into(),
            r#"{"v":1,"id":"sin-inicio","agent_type":"docs"}"#.into(),
            to_line(&newer),
            to_line(&done("a2")),
        ]
        .join("\n");
        std::fs::write(&path, [text.as_bytes(), b"\n\xff\xfe\n"].concat()).unwrap();
        let runs = read(&path).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs["a1"].tools, 9);
        assert!(read(&tmp.0.join("no-existe.jsonl")).unwrap().is_empty());
    }

    #[test]
    fn only_finished_runs_are_kept_and_written_atomically() {
        let tmp = TempDir::new("keep");
        let path = tmp.0.join("sub/dir/runs.jsonl");
        let now = at("2026-11-05T10:05:00Z");
        let running = Run {
            finished: Some(false),
            modified: at("2026-11-05T10:04:00Z"),
            ..done("busy")
        };
        let mut kept = Kept::new(path.clone());
        kept.keep(&[done("a1"), running.clone()], now);
        let on_disk = read(&path).unwrap();
        assert_eq!(on_disk.keys().collect::<Vec<_>>(), vec!["a1"]);
        // Sin temporales sueltos al lado.
        let files = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(files, 1);
        // Cuando termina, entra.
        kept.keep(
            &[Run {
                finished: Some(true),
                ..running
            }],
            now,
        );
        assert_eq!(read(&path).unwrap().len(), 2);
    }

    #[test]
    fn another_instance_entries_are_not_lost() {
        let tmp = TempDir::new("merge");
        let path = tmp.0.join("runs.jsonl");
        let now = at("2026-11-06T00:00:00Z");
        let mut mine = Kept::new(path.clone());
        mine.keep(&[], now); // carga vacío
        let mut other = Kept::new(path.clone());
        other.keep(&[done("theirs")], now);
        mine.keep(&[done("mine")], now);
        let ids: HashSet<_> = read(&path).unwrap().into_keys().collect();
        assert_eq!(
            ids,
            HashSet::from(["mine".to_string(), "theirs".to_string()])
        );
    }

    #[test]
    fn an_unreadable_cache_is_ignored_and_never_overwritten() {
        let tmp = TempDir::new("bad");
        // Un directorio donde va el fichero: no se puede leer ni escribir.
        let path = tmp.0.join("runs.jsonl");
        std::fs::create_dir_all(&path).unwrap();
        let mut kept = Kept::new(path.clone());
        kept.keep(&[done("a1")], at("2026-11-06T00:00:00Z"));
        assert!(path.is_dir());
        assert_eq!(kept.cached_runs(&HashSet::new()).count(), 1);
    }

    fn line(ts: &str) -> String {
        format!(r#"{{"cwd":"/p","sessionId":"s1","timestamp":"{ts}","message":{{"content":[]}}}}"#)
    }

    #[test]
    fn runs_gone_from_disk_come_back_from_the_cache() {
        let tmp = TempDir::new("runs");
        let projects = tmp.0.join("projects");
        let cache = tmp.0.join("cache/runs.jsonl");
        let path = projects.join("proj/s1/subagents/agent-a1.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path.with_extension("meta.json"),
            r#"{"agentType":"docs","finished":true}"#,
        )
        .unwrap();
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n",
                line("2026-11-05T10:00:00Z"),
                line("2026-11-05T10:02:00Z")
            ),
        )
        .unwrap();
        let mut runs = Runs::with_history(Some(cache.clone()));
        runs.refresh(&projects).unwrap();
        assert_eq!(runs.cached_only(), 0);
        assert!(!runs.runs()[0].from_cache);
        // Claude Code borra la transcripción: vuelve del historial, sin pasos.
        std::fs::remove_file(&path).unwrap();
        let mut later = Runs::with_history(Some(cache));
        later.refresh(&projects).unwrap();
        let all = later.runs();
        assert_eq!(all.len(), 1);
        assert_eq!(later.cached_only(), 1);
        let r = &all[0];
        assert!(r.from_cache && r.steps.is_empty());
        assert_eq!((r.id.as_str(), r.worked), ("a1", TimeDelta::minutes(2)));
        // Sin historial no se escribe nada.
        let mut plain = Runs::default();
        plain.refresh(&projects).unwrap();
        assert!(plain.runs().is_empty());
    }
}
