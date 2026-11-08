//! Los pasos de una ejecución: cada bloque `tool_use` con su hora, y el
//! recuento por herramienta de toda la ejecución.

use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// Cuántos pasos se guardan por ejecución (los más recientes). El recuento por
/// herramienta no se ve afectado: cuenta todos.
pub const MAX_STEPS: usize = 200;

/// Un bloque `tool_use` de la transcripción.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolStep {
    /// `timestamp` de la línea del bloque (la última conocida si no lo trae).
    pub at: DateTime<Utc>,
    pub name: String,
    /// Argumento corto: el comando de `Bash`, el fichero de `Edit`…
    pub arg: Option<String>,
}

impl ToolStep {
    /// `Nombre · argumento`, o solo el nombre si no hay argumento.
    pub fn label(&self) -> String {
        match &self.arg {
            Some(a) => format!("{} · {a}", self.name),
            None => self.name.clone(),
        }
    }
}

/// Lo acumulado de los `tool_use` de una transcripción.
#[derive(Debug, Default)]
pub(super) struct Steps {
    recent: VecDeque<ToolStep>,
    counts: HashMap<String, usize>,
    /// Copias compartidas de lo anterior, rehechas por `seal` al acabar cada
    /// lectura: `Runs::runs()` solo clona el `Arc`, no los pasos.
    recent_arc: Arc<[ToolStep]>,
    counts_arc: Arc<[(String, usize)]>,
}

impl Steps {
    pub(super) fn push(&mut self, step: ToolStep) {
        *self.counts.entry(step.name.clone()).or_default() += 1;
        if self.recent.len() == MAX_STEPS {
            self.recent.pop_front();
        }
        self.recent.push_back(step);
    }

    /// Rehace las copias compartidas; se llama tras cada lectura del fichero.
    pub(super) fn seal(&mut self) {
        self.recent_arc = self.recent().into();
        self.counts_arc = self.sorted_counts().into();
    }

    pub(super) fn recent_arc(&self) -> Arc<[ToolStep]> {
        Arc::clone(&self.recent_arc)
    }

    pub(super) fn counts_arc(&self) -> Arc<[(String, usize)]> {
        Arc::clone(&self.counts_arc)
    }

    /// `Nombre · argumento` del último paso.
    pub(super) fn last_label(&self) -> Option<String> {
        self.recent.back().map(ToolStep::label)
    }

    /// Los pasos guardados, del más antiguo al más reciente.
    pub(super) fn recent(&self) -> Vec<ToolStep> {
        self.recent.iter().cloned().collect()
    }

    /// Usos por herramienta, de más a menos y, a igualdad, por nombre.
    pub(super) fn sorted_counts(&self) -> Vec<(String, usize)> {
        let mut v: Vec<_> = self.counts.iter().map(|(k, n)| (k.clone(), *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::{parse_run_line, Transcript};

    fn step(secs: i64, name: &str, arg: Option<&str>) -> ToolStep {
        ToolStep {
            at: DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap(),
            name: name.into(),
            arg: arg.map(String::from),
        }
    }

    #[test]
    fn keeps_only_the_last_steps_but_counts_all_of_them() {
        let mut s = Steps::default();
        for i in 0..(MAX_STEPS as i64 + 50) {
            s.push(step(i, if i % 2 == 0 { "Bash" } else { "Read" }, None));
        }
        let kept = s.recent();
        assert_eq!(kept.len(), MAX_STEPS);
        assert_eq!(kept[0].at, step(50, "", None).at);
        assert_eq!(
            kept[MAX_STEPS - 1].at,
            step(MAX_STEPS as i64 + 49, "", None).at
        );
        let counts = s.sorted_counts();
        assert_eq!(counts.iter().map(|c| c.1).sum::<usize>(), MAX_STEPS + 50);
    }

    #[test]
    fn sealed_copies_are_shared_until_the_next_seal() {
        let mut s = Steps::default();
        s.push(step(0, "Read", None));
        s.seal();
        let (a, b) = (s.recent_arc(), s.recent_arc());
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(s.counts_arc().as_ref(), &[("Read".to_string(), 1)]);
        s.push(step(1, "Bash", None));
        assert_eq!(s.recent_arc().len(), 1); // aún sin rehacer
        s.seal();
        assert_eq!(s.recent_arc().len(), 2);
    }

    #[test]
    fn counts_are_sorted_by_use_then_name_and_last_label_follows_the_last_step() {
        let mut s = Steps::default();
        assert_eq!(s.last_label(), None);
        s.push(step(0, "Read", Some("a.rs")));
        s.push(step(1, "Edit", Some("a.rs")));
        s.push(step(2, "Bash", Some("ls")));
        s.push(step(3, "Edit", None));
        assert_eq!(
            s.sorted_counts(),
            vec![("Edit".into(), 2), ("Bash".into(), 1), ("Read".into(), 1)]
        );
        assert_eq!(s.last_label().as_deref(), Some("Edit"));
        s.push(step(4, "Bash", Some("git diff")));
        assert_eq!(s.last_label().as_deref(), Some("Bash · git diff"));
    }

    fn line(ts: &str, blocks: &str) -> String {
        format!(r#"{{"timestamp":"{ts}","cwd":"/p","message":{{"content":[{blocks}]}}}}"#)
    }

    #[test]
    fn steps_carry_their_line_time_and_counts_add_up_to_tools() {
        let mut t = Transcript::default();
        for l in [
            line(
                "2026-11-05T10:00:30.500Z",
                r#"{"type":"tool_use","name":"Read","input":{"file_path":"/p/a.rs"}},{"type":"tool_use","name":"Bash","input":{}}"#,
            ),
            line(
                "2026-11-05T10:00:40Z",
                r#"{"type":"tool_use","name":"Bash","input":{"command":"ls"}}"#,
            ),
            // sin timestamp y sin nombre: cuenta como `?` con la hora anterior
            r#"{"message":{"content":[{"type":"tool_use"}]}}"#.to_string(),
        ] {
            t.add(parse_run_line(&l).unwrap());
        }
        let steps = t.steps.recent();
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[0].label(), "Read · a.rs");
        assert_eq!(
            steps[0].at,
            "2026-11-05T10:00:30.500Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(steps[2].label(), "Bash · ls");
        assert_eq!((steps[3].name.as_str(), steps[3].at), ("?", steps[2].at));
        let counts = t.steps.sorted_counts();
        assert_eq!(counts[0], ("Bash".to_string(), 2));
        assert_eq!(counts.iter().map(|c| c.1).sum::<usize>(), t.tools);
    }
}
