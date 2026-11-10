//! Tests de `runs.rs`: lectura de líneas, metas, tiempo trabajado y recorrido.

use super::*;
use chrono::FixedOffset;

/// Línea literal de una transcripción de subagente, con dos `tool_use`.
const TOOL_LINE: &str = r#"{"parentUuid":"p","isSidechain":true,"agentId":"a1","message":{"role":"assistant","content":[{"type":"text","text":"\"type\":\"tool_use\""},{"type":"tool_use","id":"toolu_1","name":"Read","input":{}},{"type":"tool_use","id":"toolu_2","name":"Bash","input":{}}]},"type":"assistant","uuid":"u","timestamp":"2026-11-05T10:00:30.500Z","userType":"external","cwd":"/Users/x/app","sessionId":"s1","version":"2.1.0"}"#;

const META: &str = r#"{"agentType":"docs","isBuiltIn":false,"launchedWithDefaults":true,"description":"Docs: F4 y menú tras fallo","toolUseId":"toolu_01QfJPQ","spawnDepth":1,"requestShape":"background","requestNonInteractive":true,"pickedEffortModel":"claude-opus-5-5","finished":true}"#;

#[test]
fn reads_timestamp_cwd_session_and_counts_tool_uses() {
    let info = parse_run_line(TOOL_LINE).unwrap();
    assert_eq!(info.tools, 2);
    assert_eq!(info.cwd.as_deref(), Some("/Users/x/app"));
    assert_eq!(info.session_id.as_deref(), Some("s1"));
    assert_eq!(
        info.at.unwrap().to_rfc3339(),
        "2026-11-05T10:00:30.500+00:00"
    );
    assert!(parse_run_line("no es json").is_none());
}

#[test]
fn reads_the_full_meta() {
    let m = parse_meta(META).unwrap();
    assert_eq!(m.agent_type, "docs");
    assert_eq!(m.description.as_deref(), Some("Docs: F4 y menú tras fallo"));
    assert_eq!(m.tool_use_id.as_deref(), Some("toolu_01QfJPQ"));
    assert_eq!(m.finished, Some(true));
}

#[test]
fn meta_only_needs_agent_type() {
    let m = parse_meta(r#"{"agentType":"data"}"#).unwrap();
    assert_eq!(
        m,
        Meta {
            agent_type: "data".into(),
            description: None,
            tool_use_id: None,
            finished: None,
        }
    );
    assert!(parse_meta(r#"{"description":"x"}"#).is_none());
    assert!(parse_meta("roto").is_none());
}

/// Primera línea de una transcripción de subagente: el encargo (prompt).
const PROMPT_LINE: &str = r##"{"parentUuid":null,"isSidechain":true,"agentId":"a1","type":"user","message":{"role":"user","content":"# ENCARGO (de Jordi, 05-11-2026): copia de seguridad de TODO\n\nDetalle largo\nde varias líneas"},"uuid":"u0","timestamp":"2026-11-05T10:00:00.000Z","cwd":"/Users/x/app","sessionId":"s1"}"##;

fn tool_line(name: &str, input: &str) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"hola"}},{{"type":"tool_use","id":"t","name":"{name}","input":{input}}}]}},"timestamp":"2026-11-05T10:00:10.000Z","cwd":"/Users/x/app"}}"#
    )
}

#[test]
fn the_task_is_the_first_content_line_without_the_boilerplate() {
    let info = parse_run_line(PROMPT_LINE).unwrap();
    assert_eq!(info.task.as_deref(), Some("copia de seguridad de TODO"));
    assert!(parse_run_line(TOOL_LINE).unwrap().task.is_none());
}

#[test]
fn task_cleaning_handles_headers_blocks_and_empty_prompts() {
    let task = |content: &str| {
        let line = serde_json::json!({"type":"user","message":{"role":"user","content":content}});
        parse_run_line(&line.to_string()).unwrap().task
    };
    assert_eq!(
        task("\n\n## Revisa   el README\notra"),
        Some("Revisa el README".into())
    );
    // la cabecera sola no es encargo: manda la siguiente línea con contenido
    assert_eq!(
        task("# ENCARGO (de Jordi, 05-11-2026):\n\nHaz esto"),
        Some("Haz esto".into())
    );
    assert_eq!(task("   \n  "), None);
    // content como bloques de texto
    let blocks = r##"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"# x"},{"type":"text","text":"Hola mundo"}]}}"##;
    assert_eq!(parse_run_line(blocks).unwrap().task.as_deref(), Some("x"));
    // un tool_result no es un prompt
    let result = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}}"#;
    assert!(parse_run_line(result).unwrap().task.is_none());
}

#[test]
fn last_tool_shows_name_and_a_short_argument() {
    let tool = |name: &str, input: &str| {
        parse_run_line(&tool_line(name, input))
            .unwrap()
            .tool_uses
            .pop()
            .map(|(n, a)| {
                ToolStep {
                    at: DateTime::default(),
                    name: n,
                    arg: a,
                }
                .label()
            })
    };
    assert_eq!(
        tool("Bash", r#"{"command":"git diff\n  --stat"}"#),
        Some("Bash · git diff --stat".into())
    );
    assert_eq!(
        tool("Edit", r#"{"file_path":"/Users/x/app/README.md"}"#),
        Some("Edit · README.md".into())
    );
    assert_eq!(
        tool("Read", r#"{"file_path":"/Users/x/app/src/main.rs"}"#),
        Some("Read · src/main.rs".into())
    );
    assert_eq!(
        tool("Read", r#"{"file_path":"/otro/sitio/a.rs"}"#),
        Some("Read · a.rs".into())
    );
    assert_eq!(
        tool(
            "Write",
            r#"{"file_path":"/Users/x/app/b.rs","content":"x"}"#
        ),
        Some("Write · b.rs".into())
    );
    assert_eq!(
        tool("Grep", r#"{"pattern":"fn main"}"#),
        Some("Grep · fn main".into())
    );
    assert_eq!(
        tool("Glob", r#"{"pattern":"**/*.rs"}"#),
        Some("Glob · **/*.rs".into())
    );
    assert_eq!(tool("Foo", r#"{"x":1}"#), Some("Foo".into()));
    assert_eq!(tool("Bash", "{}"), Some("Bash".into()));
    // sin tool_use no hay herramienta
    assert!(parse_run_line(PROMPT_LINE).unwrap().tool_uses.is_empty());
}

#[test]
fn task_is_fixed_by_the_first_read_and_last_tool_follows_new_lines() {
    let mut t = Transcript::default();
    t.add(parse_run_line(PROMPT_LINE).unwrap());
    t.add(parse_run_line(&tool_line("Read", r#"{"file_path":"/Users/x/app/a.rs"}"#)).unwrap());
    t.add(parse_run_line(r#"{"type":"user","message":{"role":"user","content":"otro"}}"#).unwrap());
    t.add(parse_run_line(&tool_line("Bash", r#"{"command":"ls"}"#)).unwrap());
    t.add(
        parse_run_line(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"fin"}]}}"#,
        )
        .unwrap(),
    );
    assert_eq!(t.task.as_deref(), Some("copia de seguridad de TODO"));
    assert_eq!(t.steps.last_label().as_deref(), Some("Bash · ls"));
    assert_eq!(
        t.task_full.as_deref(),
        Some("copia de seguridad de TODO\n\nDetalle largo\nde varias líneas")
    );
}

fn at(t: &str) -> DateTime<Utc> {
    t.parse().unwrap()
}

fn line(ts: &str, tools: usize) -> String {
    let blocks: Vec<String> = (0..tools)
        .map(|i| format!(r#"{{"type":"tool_use","id":"t{i}","name":"Bash","input":{{}}}}"#))
        .collect();
    format!(
        r#"{{"cwd":"/p","sessionId":"s1","timestamp":"{ts}","message":{{"content":[{}]}}}}"#,
        blocks.join(",")
    )
}

#[test]
fn gaps_over_five_minutes_do_not_count() {
    let mut t = Transcript::default();
    for (ts, tools) in [
        ("2026-11-05T10:00:00Z", 0),
        ("2026-11-05T10:02:00Z", 1),
        ("2026-11-05T10:07:00Z", 2), // 5 min justos: cuenta
        ("2026-11-05T13:00:00Z", 0), // retomada horas después: no cuenta
        ("2026-11-05T13:01:30Z", 1),
    ] {
        t.add(parse_run_line(&line(ts, tools)).unwrap());
    }
    assert_eq!(t.worked, TimeDelta::seconds(2 * 60 + 5 * 60 + 90));
    assert_eq!(t.tools, 4);
    assert_eq!(t.start, Some(at("2026-11-05T10:00:00Z")));
    assert_eq!(t.last, Some(at("2026-11-05T13:01:30Z")));
    assert_eq!(t.cwd, Some(PathBuf::from("/p")));
}

#[test]
fn lines_without_timestamp_still_count_tools() {
    let mut t = Transcript::default();
    t.add(parse_run_line(&line("2026-11-05T10:00:00Z", 1)).unwrap());
    t.add(parse_run_line(r#"{"type":"x","message":{"content":[{"type":"tool_use"}]}}"#).unwrap());
    t.add(parse_run_line(&line("2026-11-05T10:01:00Z", 0)).unwrap());
    assert_eq!(t.tools, 2);
    assert_eq!(t.worked, TimeDelta::minutes(1));
}

fn run(finished: Option<bool>, modified: &str, last: &str) -> Run {
    Run {
        agent_type: "docs".into(),
        description: None,
        tool_use_id: None,
        session_id: None,
        cwd: None,
        temp_cwd: None,
        start: Some(at("2026-11-05T10:00:00Z")),
        last: Some(at(last)),
        worked: TimeDelta::minutes(3),
        worked_slots: BTreeMap::new(),
        tools: 0,
        task: None,
        task_full: None,
        last_tool: None,
        steps: Default::default(),
        tool_counts: Default::default(),
        finished,
        modified: at(modified),
        ..Run::default()
    }
}

#[test]
fn in_progress_only_if_unfinished_and_recently_touched() {
    let now = at("2026-11-05T10:20:00Z");
    assert!(run(Some(false), "2026-11-05T10:15:00Z", "2026-11-05T10:15:00Z").in_progress(now));
    // Parada más de 10 min: interrumpida.
    assert!(!run(Some(false), "2026-11-05T10:05:00Z", "2026-11-05T10:05:00Z").in_progress(now));
    assert!(!run(Some(true), "2026-11-05T10:19:00Z", "2026-11-05T10:19:00Z").in_progress(now));
    // Las de workflow no traen `finished`: en curso solo mientras se escriben.
    assert!(run(None, "2026-11-05T10:19:00Z", "2026-11-05T10:19:00Z").in_progress(now));
    assert!(!run(None, "2026-11-05T10:17:00Z", "2026-11-05T10:17:00Z").in_progress(now));
}

#[test]
fn worked_until_adds_the_tail_only_while_in_progress_and_short() {
    let now = at("2026-11-05T10:20:00Z");
    let r = run(Some(false), "2026-11-05T10:18:00Z", "2026-11-05T10:18:00Z");
    assert_eq!(r.worked_until(now), TimeDelta::minutes(5));
    // Última línea hace 7 min (fichero tocado hace 2): la cola no cuenta.
    let r = run(Some(false), "2026-11-05T10:18:00Z", "2026-11-05T10:13:00Z");
    assert_eq!(r.worked_until(now), TimeDelta::minutes(3));
    let r = run(Some(true), "2026-11-05T10:18:00Z", "2026-11-05T10:18:00Z");
    assert_eq!(r.worked_until(now), TimeDelta::minutes(3));
}

fn tz(hours: i32) -> FixedOffset {
    FixedOffset::east_opt(hours * 3600).unwrap()
}

fn day(d: &str) -> NaiveDate {
    d.parse().unwrap()
}

fn transcript_of(stamps: &[&str]) -> Transcript {
    let mut t = Transcript::default();
    for ts in stamps {
        t.add(parse_run_line(&line(ts, 0)).unwrap());
    }
    t
}

fn total(by_day: &BTreeMap<NaiveDate, TimeDelta>) -> TimeDelta {
    by_day.values().copied().sum()
}

#[test]
fn work_in_a_single_local_day_goes_to_that_day() {
    let t = transcript_of(&[
        "2026-11-05T10:00:00Z",
        "2026-11-05T10:02:00Z",
        "2026-11-05T10:07:00Z",
        "2026-11-05T13:00:00Z", // hueco largo: fuera
        "2026-11-05T13:01:30Z",
    ]);
    let r = Run {
        worked: t.worked,
        worked_slots: t.slots.clone(),
        ..run(Some(true), "2026-11-05T13:01:30Z", "2026-11-05T13:01:30Z")
    };
    let by_day = r.worked_by_day(&tz(1));
    assert_eq!(
        by_day,
        BTreeMap::from([(day("2026-11-05"), TimeDelta::seconds(8 * 60 + 30))])
    );
    assert_eq!(total(&by_day), r.worked);
}

#[test]
fn a_gap_goes_to_the_local_day_where_it_starts() {
    // En +01:00: 23:56, 23:59, 00:03, 00:04 locales. El hueco 23:59→00:03
    // empieza el día 4 y entero va al 4.
    let t = transcript_of(&[
        "2026-11-04T22:56:00Z",
        "2026-11-04T22:59:00Z",
        "2026-11-04T23:03:00Z",
        "2026-11-04T23:04:00Z",
    ]);
    let r = Run {
        worked: t.worked,
        worked_slots: t.slots.clone(),
        ..run(Some(true), "2026-11-04T23:04:00Z", "2026-11-04T23:04:00Z")
    };
    let by_day = r.worked_by_day(&tz(1));
    assert_eq!(
        by_day,
        BTreeMap::from([
            (day("2026-11-04"), TimeDelta::minutes(7)),
            (day("2026-11-05"), TimeDelta::minutes(1)),
        ])
    );
    assert_eq!(total(&by_day), r.worked);
    // En UTC todo cae el 4; en -05:00 también, a otra hora.
    assert_eq!(
        r.worked_by_day(&Utc),
        BTreeMap::from([(day("2026-11-04"), TimeDelta::minutes(8))])
    );
    // Con media hora de desfase (India) la frontera sigue siendo exacta.
    let ist = FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap();
    let t = transcript_of(&["2026-11-04T18:28:00Z", "2026-11-04T18:31:00Z"]);
    let r = Run {
        worked: t.worked,
        worked_slots: t.slots.clone(),
        ..r
    };
    assert_eq!(
        r.worked_by_day(&ist),
        BTreeMap::from([(day("2026-11-04"), TimeDelta::minutes(3))])
    );
    let t = transcript_of(&["2026-11-04T18:30:00Z", "2026-11-04T18:31:00Z"]);
    let r = Run {
        worked: t.worked,
        worked_slots: t.slots.clone(),
        ..r
    };
    assert_eq!(
        r.worked_by_day(&ist),
        BTreeMap::from([(day("2026-11-05"), TimeDelta::minutes(1))])
    );
}

#[test]
fn long_gaps_never_reach_any_day() {
    let t = transcript_of(&[
        "2026-11-03T10:00:00Z",
        "2026-11-03T10:01:00Z",
        "2026-11-05T09:00:00Z", // dos días después: no cuenta en ninguno
        "2026-11-05T09:02:00Z",
    ]);
    assert_eq!(t.worked, TimeDelta::minutes(3));
    let r = Run {
        worked: t.worked,
        worked_slots: t.slots.clone(),
        ..run(Some(true), "2026-11-05T09:02:00Z", "2026-11-05T09:02:00Z")
    };
    assert_eq!(
        r.worked_by_day(&tz(1)),
        BTreeMap::from([
            (day("2026-11-03"), TimeDelta::minutes(1)),
            (day("2026-11-05"), TimeDelta::minutes(2)),
        ])
    );
}

#[test]
fn worked_by_day_until_adds_the_running_tail_to_its_day() {
    // En curso, última línea 23:58 local del día 4, ahora 00:01 del 5:
    // la cola empieza el 4 y va al 4; la suma iguala worked_until.
    let r = Run {
        start: Some(at("2026-11-04T22:50:00Z")),
        worked: TimeDelta::minutes(3),
        worked_slots: BTreeMap::from([(at("2026-11-04T22:45:00Z"), TimeDelta::minutes(3))]),
        ..run(Some(false), "2026-11-04T22:58:00Z", "2026-11-04T22:58:00Z")
    };
    let now = at("2026-11-04T23:01:00Z");
    let by_day = r.worked_by_day_until(now, &tz(1));
    assert_eq!(
        by_day,
        BTreeMap::from([(day("2026-11-04"), TimeDelta::minutes(6))])
    );
    assert_eq!(total(&by_day), r.worked_until(now));
    // Terminada: solo lo de la transcripción.
    let done = Run {
        finished: Some(true),
        ..r.clone()
    };
    assert_eq!(total(&done.worked_by_day_until(now, &tz(1))), done.worked);
}

#[test]
fn start_in_local_time() {
    let r = run(Some(true), "2026-11-05T10:00:00Z", "2026-11-05T10:00:00Z");
    assert_eq!(
        r.start_local(&tz(1)).unwrap().to_rfc3339(),
        "2026-11-05T11:00:00+01:00"
    );
    let none = Run { start: None, ..r };
    assert!(none.start_local(&tz(1)).is_none());
}

#[test]
fn the_daily_split_survives_incremental_reads_and_sums_to_worked() {
    let tmp = TempDir::new("byday");
    let path = tmp.0.join("proj/s1/subagents/agent-a1.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(meta_path(&path), r#"{"agentType":"data"}"#).unwrap();
    append(
        &path,
        &format!(
            "{}\n{}\n",
            line("2026-11-04T22:58:00Z", 0),
            line("2026-11-04T23:00:00Z", 0)
        ),
    );
    let mut runs = Runs::default();
    runs.refresh(&tmp.0).unwrap();
    append(&path, &format!("{}\n", line("2026-11-04T23:04:00Z", 0)));
    runs.refresh(&tmp.0).unwrap();
    let r = &runs.runs()[0];
    assert_eq!(r.worked, TimeDelta::minutes(6));
    assert_eq!(
        r.worked_by_day(&tz(1)),
        BTreeMap::from([
            (day("2026-11-04"), TimeDelta::minutes(2)),
            (day("2026-11-05"), TimeDelta::minutes(4)),
        ])
    );
    assert_eq!(total(&r.worked_by_day(&tz(-8))), r.worked);
}

/// Directorio temporal propio de cada test; se borra al soltarlo.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("claude-agents-runs-{}-{name}", std::process::id()));
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

fn append(path: &Path, text: &str) {
    use std::io::Write;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn by_type(runs: &Runs) -> Vec<(String, usize, i64)> {
    let mut v: Vec<_> = runs
        .runs()
        .into_iter()
        .map(|r| (r.agent_type, r.tools, r.worked.num_seconds()))
        .collect();
    v.sort();
    v
}

#[test]
fn finds_runs_in_subagents_and_workflows_with_their_meta() {
    let tmp = TempDir::new("find");
    let direct = tmp.0.join("proj/s1/subagents/agent-a1.jsonl");
    append(
        &direct,
        &format!(
            "{}\n{}\n",
            line("2026-11-05T10:00:00Z", 1),
            line("2026-11-05T10:01:00Z", 2)
        ),
    );
    std::fs::write(meta_path(&direct), META).unwrap();
    let wf = tmp
        .0
        .join("proj/s1/subagents/workflows/wf_1/agent-a2.jsonl");
    append(&wf, &format!("{}\n", line("2026-11-05T11:00:00Z", 1)));
    std::fs::write(meta_path(&wf), r#"{"agentType":"workflow-subagent"}"#).unwrap();
    // Ni la transcripción padre, ni un journal, ni una ejecución sin meta.
    append(
        &tmp.0.join("proj/s1.jsonl"),
        &format!("{}\n", line("2026-11-05T10:00:00Z", 5)),
    );
    append(
        &tmp.0.join("proj/s1/subagents/workflows/wf_1/journal.jsonl"),
        &format!("{}\n", line("2026-11-05T10:00:00Z", 5)),
    );
    append(
        &tmp.0.join("proj/s1/subagents/agent-a3.jsonl"),
        &format!("{}\n", line("2026-11-05T10:00:00Z", 5)),
    );

    let mut runs = Runs::default();
    runs.refresh(&tmp.0).unwrap();
    assert_eq!(
        by_type(&runs),
        vec![("docs".into(), 3, 60), ("workflow-subagent".into(), 1, 0)]
    );
    let docs = runs
        .runs()
        .into_iter()
        .find(|r| r.agent_type == "docs")
        .unwrap();
    assert_eq!(docs.tool_use_id.as_deref(), Some("toolu_01QfJPQ"));
    assert_eq!(docs.session_id.as_deref(), Some("s1"));
    assert_eq!(docs.finished, Some(true));
    assert!(docs.modified > at("2020-01-01T00:00:00Z"));
}

#[test]
fn refresh_is_incremental_and_rereads_a_changed_meta() {
    let tmp = TempDir::new("incr");
    let path = tmp.0.join("proj/s1/subagents/agent-a1.jsonl");
    append(&path, &format!("{}\n", line("2026-11-05T10:00:00Z", 1)));
    let mut runs = Runs::default();
    runs.refresh(&tmp.0).unwrap();
    // Sin meta todavía: no sale.
    assert!(runs.runs().is_empty());
    std::fs::write(meta_path(&path), r#"{"agentType":"data","finished":false}"#).unwrap();
    let tail = line("2026-11-05T10:02:00Z", 2);
    let (head, rest) = tail.split_at(tail.len() / 2);
    append(&path, head);
    runs.refresh(&tmp.0).unwrap();
    assert_eq!(by_type(&runs), vec![("data".into(), 1, 0)]);
    assert_eq!(runs.runs()[0].finished, Some(false));
    append(&path, &format!("{rest}\n"));
    runs.refresh(&tmp.0).unwrap();
    runs.refresh(&tmp.0).unwrap();
    // Sin duplicar lo ya leído.
    assert_eq!(by_type(&runs), vec![("data".into(), 3, 120)]);
    std::fs::write(
        meta_path(&path),
        r#"{"agentType":"data","finished":true,"description":"hecha"}"#,
    )
    .unwrap();
    runs.refresh(&tmp.0).unwrap();
    assert_eq!(runs.runs()[0].finished, Some(true));
    assert_eq!(runs.runs()[0].description.as_deref(), Some("hecha"));
    std::fs::remove_file(&path).unwrap();
    runs.refresh(&tmp.0).unwrap();
    assert!(runs.runs().is_empty());
}

#[cfg(unix)]
#[test]
fn a_symlinked_transcript_is_not_counted_twice() {
    // Claude Code enlaza la transcripción de una ejecución desde otra
    // sesión (pasa en el disco real): es la misma ejecución.
    let tmp = TempDir::new("link");
    let real = tmp.0.join("proj/s1/subagents/agent-a1.jsonl");
    append(&real, &format!("{}\n", line("2026-11-05T10:00:00Z", 1)));
    std::fs::write(meta_path(&real), META).unwrap();
    let link = tmp.0.join("proj/s2/subagents/agent-a1.jsonl");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    std::fs::write(meta_path(&link), META).unwrap();
    let mut runs = Runs::default();
    runs.refresh(&tmp.0).unwrap();
    assert_eq!(runs.runs().len(), 1);
}

/// Línea de asistente con uso: `id` del mensaje, salida y lectura de caché.
fn usage_line(msg: &str, output: u64, read: u64, blocks: &str) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"s1","cwd":"/p","timestamp":"2026-11-05T10:00:00Z","message":{{"model":"claude-opus-5-5","id":"{msg}","role":"assistant","content":[{blocks}],"usage":{{"input_tokens":2,"output_tokens":{output},"cache_read_input_tokens":{read},"cache_creation_input_tokens":10}}}}}}"#
    )
}

fn agent_call(id: &str) -> String {
    format!(
        r#"{{"type":"tool_use","id":"{id}","name":"Agent","input":{{"subagent_type":"docs","prompt":"x"}}}}"#
    )
}

fn find<'a>(runs: &'a [Run], id: &str) -> &'a Run {
    runs.iter().find(|r| r.id == id).unwrap()
}

#[test]
fn a_run_has_its_file_id_and_deduplicated_tokens() {
    let tmp = TempDir::new("usage");
    let path = tmp.0.join("proj/s1/subagents/agent-a1.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(meta_path(&path), r#"{"agentType":"docs","finished":true}"#).unwrap();
    let think = r#"{"type":"thinking","thinking":"x"}"#;
    let read = r#"{"type":"tool_use","id":"t1","name":"Read","input":{}}"#;
    append(
        &path,
        &format!(
            "{}\n{}\n{}\n",
            usage_line("m1", 9, 1000, think),
            usage_line("m1", 300, 1000, read),
            usage_line("m2", 50, 5000, think),
        ),
    );
    let mut runs = Runs::default();
    runs.refresh(&tmp.0).unwrap();
    let r = &runs.runs()[0];
    assert_eq!(r.id, "a1");
    assert_eq!(r.usage.tokens.output, 350);
    assert_eq!(r.usage.tokens.cache_read, 6000);
    assert_eq!(r.usage.tokens.total(), 4 + 350 + 6000 + 20);
    assert_eq!(r.usage.context, Some(2 + 5000 + 10));
    assert_eq!(r.usage.model.as_deref(), Some("claude-opus-5-5"));
    assert!(!r.from_cache);
}

#[test]
fn a_fork_does_not_count_the_copied_parent_call_as_its_tokens_or_child() {
    // Un fork empieza con una copia del mensaje del padre que lo lanzó
    // (visto en disco): su uso es del padre y su `Agent` es su propia llamada.
    let tmp = TempDir::new("fork");
    let path = tmp.0.join("proj/s1/subagents/agent-f1.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        meta_path(&path),
        r#"{"agentType":"fork","isFork":true,"toolUseId":"toolu_own","finished":true}"#,
    )
    .unwrap();
    append(
        &path,
        &format!(
            "{}\n{}\n{}\n",
            r#"{"type":"fork-context-ref","agentId":"f1","parentSessionId":"s1"}"#,
            usage_line("m_parent", 4000, 800000, &agent_call("toolu_own")),
            usage_line("m_own", 100, 801000, r#"{"type":"text","text":"x"}"#),
        ),
    );
    let mut runs = Runs::default();
    runs.refresh(&tmp.0).unwrap();
    let r = &runs.runs()[0];
    assert_eq!(r.usage.tokens.output, 100);
    assert!(r.children.is_empty());
    assert_eq!(r.parent_run, None);
}

#[test]
fn a_run_launched_from_another_run_is_its_child() {
    let tmp = TempDir::new("nested");
    let dir = tmp.0.join("proj/s1/subagents");
    let parent = dir.join("agent-p1.jsonl");
    let child = dir.join("agent-c1.jsonl");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        meta_path(&parent),
        r#"{"agentType":"backend","toolUseId":"toolu_top"}"#,
    )
    .unwrap();
    std::fs::write(
        meta_path(&child),
        r#"{"agentType":"docs","toolUseId":"toolu_nested","spawnDepth":2}"#,
    )
    .unwrap();
    append(
        &parent,
        &format!("{}\n", usage_line("m1", 1, 1, &agent_call("toolu_nested"))),
    );
    append(&child, &format!("{}\n", line("2026-11-05T10:01:00Z", 0)));
    let mut runs = Runs::default();
    runs.refresh(&tmp.0).unwrap();
    let all = runs.runs();
    assert_eq!(find(&all, "c1").parent_run.as_deref(), Some("p1"));
    assert_eq!(find(&all, "p1").children, vec!["c1".to_string()]);
    assert_eq!(find(&all, "p1").parent_run, None);
}

#[test]
fn a_run_knows_the_first_real_prompt_of_its_session() {
    let tmp = TempDir::new("prompt");
    let path = tmp.0.join("proj/s1/subagents/agent-a1.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(meta_path(&path), r#"{"agentType":"docs"}"#).unwrap();
    append(&path, &format!("{}\n", line("2026-11-05T10:00:00Z", 0)));
    // Ni el comando, ni el `isMeta`, ni los avisos `<…>`: el primer prompt de verdad.
    std::fs::write(
        tmp.0.join("proj/s1.jsonl"),
        concat!(
            r#"{"type":"user","cwd":"/p","sessionId":"s1","message":{"role":"user","content":"<command-message>review</command-message>\n<command-name>/review</command-name>"}}"#,
            "\n",
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"Base directory for this skill"}]}}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<ide_opened_file>x</ide_opened_file>"},{"type":"text","text":"mejora el README\ny más"}]}}"#,
            "\n"
        ),
    )
    .unwrap();
    let mut runs = Runs::default();
    runs.refresh(&tmp.0).unwrap();
    assert_eq!(
        runs.runs()[0].session_prompt.as_deref(),
        Some("mejora el README")
    );
}

/// Línea literal de una llamada a una skill dentro de una transcripción de
/// subagente (forma vista en disco).
const SKILL_LINE: &str = r#"{"parentUuid":"p","isSidechain":true,"agentId":"a1","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_01Jgc7f24","name":"Skill","input":{"skill":"superpowers:brainstorming","args":"una idea"}}]},"type":"assistant","uuid":"u","timestamp":"2026-11-05T10:00:20.000Z","cwd":"/Users/x/app","sessionId":"s1"}"#;

#[test]
fn a_skill_call_shows_the_skill_as_its_argument() {
    let info = parse_run_line(SKILL_LINE).unwrap();
    assert_eq!(
        info.tool_uses,
        vec![("Skill".into(), Some("superpowers:brainstorming".into()))]
    );
    // Sin `input.skill` no hay argumento.
    let bare = parse_run_line(&tool_line("Skill", "{}")).unwrap();
    assert_eq!(bare.tool_uses, vec![("Skill".into(), None)]);
}

#[test]
fn the_skill_is_the_last_one_invoked_and_stays_until_another_is_loaded() {
    let mut t = Transcript::default();
    t.add(parse_run_line(PROMPT_LINE).unwrap());
    t.add(parse_run_line(&tool_line("Read", r#"{"file_path":"/Users/x/app/a.rs"}"#)).unwrap());
    assert_eq!(t.steps.skill(), None);
    t.add(parse_run_line(SKILL_LINE).unwrap());
    t.add(parse_run_line(&tool_line("Bash", r#"{"command":"ls"}"#)).unwrap());
    // Una skill cargada sigue mandando aunque luego se usen otras herramientas.
    assert_eq!(t.steps.skill(), Some("superpowers:brainstorming"));
    // Una llamada sin nombre de skill no la cambia.
    t.add(parse_run_line(&tool_line("Skill", "{}")).unwrap());
    assert_eq!(t.steps.skill(), Some("superpowers:brainstorming"));
    t.add(
        parse_run_line(&tool_line(
            "Skill",
            r#"{"skill":"superpowers:test-driven-development"}"#,
        ))
        .unwrap(),
    );
    assert_eq!(t.steps.skill(), Some("superpowers:test-driven-development"));
}

#[test]
fn the_skill_survives_dropping_old_steps_and_reaches_the_run() {
    let tmp = TempDir::new("skill");
    let path = tmp.0.join("proj/s1/subagents/agent-a1.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(meta_path(&path), r#"{"agentType":"docs","finished":false}"#).unwrap();
    let mut text = format!("{SKILL_LINE}\n");
    // Más pasos de los que se guardan: la llamada a la skill ya no está en `steps`.
    for _ in 0..steps::MAX_STEPS {
        text.push_str(&tool_line("Bash", r#"{"command":"ls"}"#));
        text.push('\n');
    }
    append(&path, &text);
    let mut runs = Runs::default();
    runs.refresh(&tmp.0).unwrap();
    let r = &runs.runs()[0];
    assert!(r.steps.iter().all(|s| s.name != "Skill"));
    assert_eq!(r.skill.as_deref(), Some("superpowers:brainstorming"));

    // Sin ninguna skill invocada, no hay.
    let other = tmp.0.join("proj/s1/subagents/agent-a2.jsonl");
    std::fs::write(meta_path(&other), r#"{"agentType":"data"}"#).unwrap();
    append(&other, &format!("{TOOL_LINE}\n"));
    runs.refresh(&tmp.0).unwrap();
    assert_eq!(find(&runs.runs(), "a2").skill, None);
}
