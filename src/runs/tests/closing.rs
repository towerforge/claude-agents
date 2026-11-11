//! Tests del cierre de una ejecución leído del final de su transcripción.

use super::*;

/// Líneas literales (recortadas) del final de transcripciones de subagente.
const HANDBACK_CALL: &str = r#"{"type":"assistant","timestamp":"2026-11-05T10:02:00.000Z","message":{"role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","id":"toolu_01Hb","name":"SubagentHandback","input":{"message":"hecho"}}]}}"#;
const HANDBACK_RESULT: &str = r#"{"type":"user","timestamp":"2026-11-05T10:02:00.200Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01Hb","content":"ok"}]}}"#;
const STRUCTURED_CALL: &str = r#"{"type":"assistant","timestamp":"2026-11-05T10:02:00.000Z","message":{"role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","id":"toolu_01So","name":"StructuredOutput","input":{"ok":true}}]}}"#;
const STRUCTURED_RESULT: &str = r#"{"type":"user","timestamp":"2026-11-05T10:02:00.200Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01So","content":"Structured output provided successfully"}]}}"#;
const STRUCTURED_ERROR: &str = r#"{"type":"user","timestamp":"2026-11-05T10:02:00.200Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01So","is_error":true,"content":"no cumple el schema"}]}}"#;
const END_TURN: &str = r#"{"type":"assistant","timestamp":"2026-11-05T10:02:00.000Z","message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"Listo."}]}}"#;
const STOP_SEQUENCE: &str = r#"{"type":"assistant","timestamp":"2026-11-05T10:02:00.000Z","message":{"role":"assistant","stop_reason":"stop_sequence","content":[{"type":"text","text":"Listo."}]}}"#;
const READ_CALL: &str = r#"{"type":"assistant","timestamp":"2026-11-05T10:01:00.000Z","message":{"role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","id":"toolu_01Rd","name":"Read","input":{"file_path":"/p/a.rs"}}]}}"#;
const READ_RESULT: &str = r#"{"type":"user","timestamp":"2026-11-05T10:01:00.300Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01Rd","content":"fn main() {}"}]}}"#;
const USER_TEXT: &str = r#"{"type":"user","timestamp":"2026-11-05T10:00:00.000Z","message":{"role":"user","content":"Revisa el README"}}"#;
const ATTACHMENT: &str = r#"{"type":"attachment","timestamp":"2026-11-05T10:02:01.000Z","attachment":{"type":"todo_reminder"}}"#;
/// Un `end_turn` que además lleva un `tool_use`: no es final.
const END_TURN_WITH_TOOL: &str = r#"{"type":"assistant","timestamp":"2026-11-05T10:02:00.000Z","message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"sigo"},{"type":"tool_use","id":"toolu_01Rd","name":"Read","input":{}}]}}"#;

fn closed_after(lines: &[&str]) -> bool {
    let mut t = Transcript::default();
    for l in lines {
        t.add(parse_run_line(l).unwrap());
    }
    t.ending.closed()
}

#[test]
fn a_handback_or_structured_output_result_closes_the_run() {
    assert!(closed_after(&[USER_TEXT, HANDBACK_CALL, HANDBACK_RESULT]));
    assert!(closed_after(&[
        USER_TEXT,
        STRUCTURED_CALL,
        STRUCTURED_RESULT
    ]));
    // La llamada sola, sin su resultado, aún no.
    assert!(!closed_after(&[USER_TEXT, HANDBACK_CALL]));
    // Un resultado con error: el agente tiene que reintentar.
    assert!(!closed_after(&[
        USER_TEXT,
        STRUCTURED_CALL,
        STRUCTURED_ERROR
    ]));
}

#[test]
fn a_final_assistant_text_closes_the_run() {
    assert!(closed_after(&[USER_TEXT, READ_CALL, READ_RESULT, END_TURN]));
    assert!(closed_after(&[USER_TEXT, STOP_SEQUENCE]));
    assert!(!closed_after(&[USER_TEXT, END_TURN_WITH_TOOL]));
}

#[test]
fn interrupted_endings_are_not_closed() {
    assert!(!closed_after(&[]));
    assert!(!closed_after(&[USER_TEXT]));
    assert!(!closed_after(&[USER_TEXT, READ_CALL]));
    assert!(!closed_after(&[USER_TEXT, READ_CALL, READ_RESULT]));
    // El resultado de otra herramienta no cierra aunque antes hubiera un
    // handback ya contestado.
    assert!(!closed_after(&[
        HANDBACK_CALL,
        HANDBACK_RESULT,
        READ_CALL,
        READ_RESULT
    ]));
}

#[test]
fn lines_of_other_types_do_not_change_the_ending() {
    assert!(closed_after(&[HANDBACK_CALL, HANDBACK_RESULT, ATTACHMENT]));
    assert!(closed_after(&[END_TURN, ATTACHMENT]));
    assert!(!closed_after(&[READ_CALL, ATTACHMENT]));
    // Ni las líneas sin `type` (formas viejas) ni las que no son JSON.
    assert!(closed_after(&[END_TURN, &line("2026-11-05T10:03:00Z", 0)]));
}

#[test]
fn a_closed_run_reopens_when_its_transcript_grows() {
    assert!(!closed_after(&[END_TURN, USER_TEXT]));
    assert!(!closed_after(&[HANDBACK_CALL, HANDBACK_RESULT, READ_CALL]));
}

#[test]
fn a_closed_run_without_finished_is_not_in_progress() {
    let now = at("2026-11-05T10:20:00Z");
    let recent = "2026-11-05T10:19:30Z";
    let closed = |finished| Run {
        closed: true,
        ..run(finished, recent, recent)
    };
    // Workflow (sin `finished`): recién escrita pero cerrada, ya no está en curso…
    assert!(run(None, recent, recent).in_progress(now));
    assert!(!closed(None).in_progress(now));
    // …ni suma la cola hasta ahora.
    assert_eq!(closed(None).worked_until(now), TimeDelta::minutes(3));
    // Con `finished: false` manda el meta, que Claude Code pone a `true` en el
    // mismo segundo que la última línea.
    assert!(closed(Some(false)).in_progress(now));
    assert!(!closed(Some(true)).in_progress(now));
}

#[test]
fn the_run_carries_the_ending_and_follows_incremental_reads() {
    let tmp = TempDir::new("closing");
    let path = tmp
        .0
        .join("proj/s1/subagents/workflows/wf_1/agent-a1.jsonl");
    append(&path, &format!("{USER_TEXT}\n{STRUCTURED_CALL}\n"));
    std::fs::write(meta_path(&path), r#"{"agentType":"workflow-subagent"}"#).unwrap();
    let mut runs = Runs::default();
    let closed = |runs: &mut Runs| {
        runs.refresh(&tmp.0).unwrap();
        runs.runs()[0].closed
    };
    assert!(!closed(&mut runs));
    // El resultado llega en otra pasada: la llamada pendiente se recuerda.
    append(&path, &format!("{STRUCTURED_RESULT}\n"));
    assert!(closed(&mut runs));
    // Sigue escribiendo: deja de estar cerrada.
    append(&path, &format!("{READ_CALL}\n"));
    assert!(!closed(&mut runs));
}
