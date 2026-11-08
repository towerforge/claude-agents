//! Hilo que relee el disco y manda los resultados a la pantalla: primero las
//! definiciones (de vez en cuando o cuando se pide con `r`) y después la
//! actividad (todo el historial de transcripciones, las ejecuciones de
//! subagentes y las sesiones) cada pocos segundos.

use crate::tui::app::Msg;
use crate::{agents, runs, sessions, transcripts};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

pub struct Sources {
    pub roots: Vec<PathBuf>,
    pub home: PathBuf,
    pub projects: PathBuf,
    /// Historial guardado de ejecuciones (`runs::history::default_path()`);
    /// `None` no lee ni escribe nada (los tests).
    pub history: Option<PathBuf>,
}

/// El historial y las ejecuciones se ponen al día a este ritmo. La primera
/// pasada lo lee todo (~1 s el historial y ~0,7 s las ejecuciones con 1 GB);
/// las siguientes solo lo añadido, en ms.
const ACTIVITY_EVERY: Duration = Duration::from_secs(2);
/// Recorrer el home entero cuesta segundos: solo de vez en cuando.
const AGENTS_EVERY: Duration = Duration::from_secs(30);

/// Arranca el hilo. Termina solo cuando la pantalla deja de escuchar.
pub fn spawn(src: Sources, rescan: Receiver<()>) -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut agents_at: Option<Instant> = None;
        // Vive con el hilo: así cada vuelta solo lee lo escrito desde la anterior.
        let mut history = transcripts::History::default();
        let mut runs = runs::Runs::with_history(src.history.clone());
        loop {
            if agents_at.is_none_or(|t| t.elapsed() >= AGENTS_EVERY) {
                let msg = match agents::find_agents(&src.roots, &src.home) {
                    Ok(found) => Msg::Agents(found),
                    Err(e) => Msg::Error(format!("{e:#}")),
                };
                if tx.send(msg).is_err() {
                    return;
                }
                agents_at = Some(Instant::now());
            }
            // Va el historial completo: la pantalla decide qué cae en la ventana.
            // Las ejecuciones, al mismo ritmo, también incrementales y a la vez
            // que el historial: en serie la primera pasada se va a ~2 s.
            let read = thread::scope(|s| {
                let reading = s.spawn(|| runs.refresh(&src.projects));
                let read = history.refresh(&src.projects);
                read.and(
                    reading
                        .join()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("reader thread panicked"))),
                )
            });
            let msg = match read {
                Ok(()) => Msg::Activity {
                    calls: history.invocations().cloned().collect(),
                    runs: runs.runs(),
                    sessions: sessions::open_sessions().map(|s| s.len()).unwrap_or(0),
                },
                Err(e) => Msg::Error(format!("{e:#}")),
            };
            if tx.send(msg).is_err() {
                return;
            }
            match rescan.recv_timeout(ACTIVITY_EVERY) {
                Ok(()) => agents_at = None,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn activity_carries_the_runs_read_from_disk() {
        let base = std::env::temp_dir().join(format!(
            "claude-agents-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dir = base.join("projects/-p/sesion/subagents");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("agent-a1.jsonl"),
            concat!(
                r#"{"timestamp":"2026-11-05T10:00:00Z","cwd":"/p","sessionId":"s"}"#,
                "\n",
                r#"{"timestamp":"2026-11-05T10:03:00Z","cwd":"/p","sessionId":"s"}"#,
                "\n"
            ),
        )
        .unwrap();
        fs::write(
            dir.join("agent-a1.meta.json"),
            r#"{"agentType":"docs","toolUseId":"toolu_1","finished":true}"#,
        )
        .unwrap();

        let (_rescan_tx, rescan_rx) = mpsc::channel();
        let rx = spawn(
            Sources {
                roots: vec![],
                home: base.clone(),
                projects: base.join("projects"),
                history: None,
            },
            rescan_rx,
        );
        let runs = loop {
            match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
                Msg::Activity { runs, .. } => break runs,
                Msg::Agents(_) => {}
                Msg::Error(e) => panic!("{e}"),
            }
        };
        fs::remove_dir_all(&base).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].agent_type, "docs");
        assert_eq!(runs[0].worked, chrono::TimeDelta::minutes(3));
    }
}
