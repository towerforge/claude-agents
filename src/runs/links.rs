//! Quién lanzó a quién entre ejecuciones.
//!
//! Una ejecución lleva en su meta el `toolUseId` de la llamada `Agent` (antes
//! `Task`) que la lanzó. Si esa llamada está en la transcripción de otra
//! ejecución, esa es su madre; si no, la lanzó la sesión. Un fork empieza con
//! una copia del mensaje del padre que lo lanzó, con su propia llamada dentro:
//! esa no cuenta (se descarta al leer, ver `Transcript::add`).
//!
//! Comprobado en disco (noviembre de 2026): todas las metas tienen
//! `spawnDepth: 1` y ninguna transcripción de subagente lanza a otro, así que
//! el caso anidado solo está probado con fixtures.

use super::Run;
use serde_json::Value;
use std::collections::HashMap;

/// Ids de los bloques `tool_use` que lanzan un subagente (`Agent` o `Task`).
pub(super) fn launch_ids(doc: &Value) -> Vec<String> {
    doc.pointer("/message/content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                .filter(|b| {
                    matches!(
                        b.get("name").and_then(Value::as_str),
                        Some("Agent" | "Task")
                    )
                })
                .filter_map(|b| b.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Rellena `parent_run` y `children` de `runs`. `launched` va de cada llamada
/// lanzadora (id del `tool_use`) al id de la ejecución en cuya transcripción
/// está.
pub(super) fn link(runs: &mut [Run], launched: &HashMap<String, String>) {
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for r in runs.iter_mut() {
        r.parent_run = r
            .tool_use_id
            .as_ref()
            .and_then(|call| launched.get(call))
            .filter(|parent| **parent != r.id)
            .cloned();
        if let Some(parent) = &r.parent_run {
            children
                .entry(parent.clone())
                .or_default()
                .push(r.id.clone());
        }
    }
    for r in runs.iter_mut() {
        let mut kids = children.remove(&r.id).unwrap_or_default();
        kids.sort();
        r.children = kids;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_ids_are_the_agent_and_task_calls() {
        let doc: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"content":[
                {"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"docs"}},
                {"type":"tool_use","id":"t2","name":"Task","input":{}},
                {"type":"tool_use","id":"t3","name":"Read","input":{}},
                {"type":"text","text":"Agent"}]}}"#,
        )
        .unwrap();
        assert_eq!(launch_ids(&doc), vec!["t1".to_string(), "t2".to_string()]);
        assert!(launch_ids(&serde_json::json!({"message":{"content":"x"}})).is_empty());
    }

    fn run(id: &str, call: Option<&str>) -> Run {
        Run {
            id: id.into(),
            tool_use_id: call.map(String::from),
            ..Run::default()
        }
    }

    #[test]
    fn link_sets_parents_and_sorted_children() {
        let mut runs = [
            run("p", Some("top")),
            run("c2", Some("k2")),
            run("c1", Some("k1")),
            run("lonely", None),
        ];
        let launched = HashMap::from([
            ("k1".to_string(), "p".to_string()),
            ("k2".to_string(), "p".to_string()),
            // una ejecución no es madre de sí misma
            ("top".to_string(), "p".to_string()),
        ]);
        link(&mut runs, &launched);
        assert_eq!(runs[0].parent_run, None);
        assert_eq!(runs[0].children, vec!["c1".to_string(), "c2".to_string()]);
        assert_eq!(runs[1].parent_run.as_deref(), Some("p"));
        assert!(runs[3].parent_run.is_none() && runs[3].children.is_empty());
    }
}
