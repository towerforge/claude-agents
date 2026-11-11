//! Tests de la lista de skills invocadas por una ejecución (`Run::skills`).

use super::*;
use steps::{SkillUse, MAX_SKILLS};

/// Línea literal de una llamada a una skill, con la hora y el nombre dados.
fn skill_line(ts: &str, skill: &str) -> String {
    format!(
        r#"{{"parentUuid":"p","isSidechain":true,"agentId":"a1","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"toolu_01Jgc7f24","name":"Skill","input":{{"skill":"{skill}","args":"una idea"}}}}]}},"type":"assistant","uuid":"u","timestamp":"{ts}","cwd":"/Users/x/app","sessionId":"s1"}}"#
    )
}

fn names(skills: &[SkillUse]) -> Vec<(&str, DateTime<Utc>)> {
    skills.iter().map(|s| (s.name.as_str(), s.at)).collect()
}

#[test]
fn every_skill_call_is_kept_with_its_time_in_file_order() {
    let mut t = Transcript::default();
    t.add(parse_run_line(SKILL_LINE).unwrap());
    t.add(parse_run_line(&tool_line("Bash", r#"{"command":"ls"}"#)).unwrap());
    // Sin `input.skill` no es una invocación: no entra en la lista.
    t.add(parse_run_line(&tool_line("Skill", "{}")).unwrap());
    t.add(parse_run_line(&skill_line("2026-11-05T10:03:00.000Z", "frontend-design")).unwrap());
    // La misma skill otra vez es otra invocación.
    t.add(
        parse_run_line(&skill_line(
            "2026-11-05T10:04:00.000Z",
            "superpowers:brainstorming",
        ))
        .unwrap(),
    );
    assert_eq!(
        names(&t.steps.skills()),
        vec![
            ("superpowers:brainstorming", at("2026-11-05T10:00:20Z")),
            ("frontend-design", at("2026-11-05T10:03:00Z")),
            ("superpowers:brainstorming", at("2026-11-05T10:04:00Z")),
        ]
    );
    // La que manda sigue siendo la última.
    assert_eq!(t.steps.skill(), Some("superpowers:brainstorming"));
}

#[test]
fn only_the_last_skill_calls_are_kept_and_the_last_one_still_rules() {
    let mut t = Transcript::default();
    for i in 0..(MAX_SKILLS + 3) {
        t.add(
            parse_run_line(&skill_line(
                "2026-11-05T10:00:00.000Z",
                &format!("skill-{i}"),
            ))
            .unwrap(),
        );
    }
    let skills = t.steps.skills();
    assert_eq!(skills.len(), MAX_SKILLS);
    assert_eq!(skills[0].name, "skill-3");
    let last = format!("skill-{}", MAX_SKILLS + 2);
    assert_eq!(skills[MAX_SKILLS - 1].name, last);
    assert_eq!(t.steps.skill(), Some(last.as_str()));
}

#[test]
fn the_skills_reach_the_run_even_after_their_steps_are_dropped() {
    let tmp = TempDir::new("skills");
    let path = tmp.0.join("proj/s1/subagents/agent-a1.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(meta_path(&path), r#"{"agentType":"docs","finished":false}"#).unwrap();
    let mut text = format!(
        "{SKILL_LINE}\n{}\n",
        skill_line("2026-11-05T10:00:25.000Z", "frontend-design")
    );
    // Más pasos de los que se guardan: las llamadas ya no están en `steps`.
    for _ in 0..steps::MAX_STEPS {
        text.push_str(&tool_line("Bash", r#"{"command":"ls"}"#));
        text.push('\n');
    }
    append(&path, &text);
    let mut runs = Runs::default();
    runs.refresh(&tmp.0).unwrap();
    let r = &runs.runs()[0];
    assert!(r.steps.iter().all(|s| s.name != "Skill"));
    assert_eq!(
        names(&r.skills),
        vec![
            ("superpowers:brainstorming", at("2026-11-05T10:00:20Z")),
            ("frontend-design", at("2026-11-05T10:00:25Z")),
        ]
    );
    assert_eq!(r.skill.as_deref(), Some("frontend-design"));

    // Lo añadido en una lectura posterior se suma a la lista.
    append(
        &path,
        &format!("{}\n", skill_line("2026-11-05T10:01:00.000Z", "simplify")),
    );
    runs.refresh(&tmp.0).unwrap();
    let r = &runs.runs()[0];
    assert_eq!(r.skills.len(), 3);
    assert_eq!(r.skills[2].name, "simplify");
    assert_eq!(r.skill.as_deref(), Some("simplify"));

    // Sin ninguna skill invocada, lista vacía.
    let other = tmp.0.join("proj/s1/subagents/agent-a2.jsonl");
    std::fs::write(meta_path(&other), r#"{"agentType":"data"}"#).unwrap();
    append(&other, &format!("{TOOL_LINE}\n"));
    runs.refresh(&tmp.0).unwrap();
    assert!(find(&runs.runs(), "a2").skills.is_empty());
}

/// Línea literal `api-request-blob` (Claude Code 2.1.319+): un trozo del cuerpo
/// de la siguiente petición a la API, que repite los `tool_use` ya escritos en
/// sus líneas `assistant` (mismos ids), sin `timestamp` ni `cwd`.
const REQUEST_BLOB: &str = r#"{"type":"api-request-blob","sessionId":"s1","agentId":"a1","hash":"8581baeced0d63cd","message":{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"Bash","input":{"command":"ls"},"caller":{"type":"direct"}},{"type":"tool_use","id":"toolu_01Jgc7f24","name":"Skill","input":{"skill":"superpowers:brainstorming"},"caller":{"type":"direct"}},{"type":"tool_use","id":"toolu_01Launch","name":"Agent","input":{"subagent_type":"docs"},"caller":{"type":"direct"}}]}}"#;

/// Línea literal `api-request`: la petición en sí, con `timestamp` y sin mensaje.
const REQUEST: &str = r#"{"type":"api-request","sessionId":"s1","agentId":"a1","id":"2fcf6118","timestamp":"2026-11-05T10:00:25.000Z","version":"2.1.319","querySource":"agent:custom:visual","shapeHash":"ab12","params":{},"keep":0,"tail":[],"cacheControl":[],"requestDigest":"cd34"}"#;

#[test]
fn request_records_do_not_repeat_the_calls_of_the_run() {
    let mut t = Transcript::default();
    t.add(parse_run_line(&tool_line("Bash", r#"{"command":"ls"}"#)).unwrap());
    t.add(parse_run_line(SKILL_LINE).unwrap());
    t.add(parse_run_line(REQUEST).unwrap());
    t.add(parse_run_line(REQUEST_BLOB).unwrap());
    assert_eq!(
        names(&t.steps.skills()),
        vec![("superpowers:brainstorming", at("2026-11-05T10:00:20Z"))]
    );
    assert_eq!(
        t.steps.sorted_counts(),
        vec![("Bash".to_string(), 1), ("Skill".to_string(), 1)]
    );
    assert_eq!((t.tools, t.steps.recent().len()), (2, 2));
    // La llamada `Agent` que solo está en el trozo no es un lanzamiento suyo.
    assert!(t.launches.is_empty());
    // La hora de la petición sí cuenta como actividad.
    assert_eq!(t.last, Some(at("2026-11-05T10:00:25Z")));
}
