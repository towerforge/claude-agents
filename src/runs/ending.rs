//! Si una ejecución está cerrada, leído del final de su transcripción.
//!
//! El meta de las ejecuciones de workflow no trae `finished`, pero el final de
//! la transcripción sí dice si terminó. Está cerrada cuando su último mensaje
//! `assistant`/`user` es:
//!
//! - el `tool_result` sin error de un `tool_use` `SubagentHandback` o
//!   `StructuredOutput` (la entrega del informe o de la salida con schema); o
//! - un `assistant` con `stop_reason` `end_turn` o `stop_sequence` y ningún
//!   `tool_use`.
//!
//! Cualquier otro final (texto de usuario, el resultado de otra herramienta,
//! una llamada sin contestar) es una ejecución en marcha o interrumpida. Las
//! líneas de otros tipos (`attachment`, `system`…) no cuentan.

use serde_json::Value;

/// Herramientas cuyo resultado es lo último que escribe una ejecución.
const HANDOFFS: [&str; 2] = ["SubagentHandback", "StructuredOutput"];
/// `stop_reason` de un mensaje con el que el asistente da el turno por acabado.
const FINAL_STOPS: [&str; 2] = ["end_turn", "stop_sequence"];

/// Lo que una línea `assistant` o `user` dice sobre el final.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Turn {
    Assistant {
        /// Ids de sus `tool_use` de entrega (`HANDOFFS`).
        handoffs: Vec<String>,
        /// `stop_reason` final y ningún `tool_use`.
        ended: bool,
    },
    /// Ids de los `tool_use` que contesta sin error.
    User { results: Vec<String> },
}

/// El `Turn` de una línea; `None` si no es `assistant` ni `user`.
pub(super) fn turn_of(doc: &Value) -> Option<Turn> {
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let is = |b: &Value, kind: &str| b.get("type").and_then(Value::as_str) == Some(kind);
    let blocks = || {
        doc.pointer("/message/content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    };
    match doc.get("type").and_then(Value::as_str)? {
        "assistant" => {
            let stop = doc.pointer("/message/stop_reason").and_then(Value::as_str);
            Some(Turn::Assistant {
                handoffs: blocks()
                    .filter(|b| is(b, "tool_use"))
                    .filter(|b| {
                        b.get("name")
                            .and_then(Value::as_str)
                            .is_some_and(|n| HANDOFFS.contains(&n))
                    })
                    .filter_map(|b| text(b, "id"))
                    .collect(),
                ended: stop.is_some_and(|s| FINAL_STOPS.contains(&s))
                    && !blocks().any(|b| is(b, "tool_use")),
            })
        }
        "user" => Some(Turn::User {
            results: blocks()
                .filter(|b| is(b, "tool_result"))
                .filter(|b| b.get("is_error").and_then(Value::as_bool) != Some(true))
                .filter_map(|b| text(b, "tool_use_id"))
                .collect(),
        }),
        _ => None,
    }
}

/// El final de una transcripción, mensaje a mensaje.
#[derive(Debug, Default)]
pub(super) struct Ending {
    /// Entregas del último mensaje `assistant` que las llevaba, a la espera de
    /// su resultado (que puede llegar en otra pasada de lectura).
    pending: Vec<String>,
    closed: bool,
}

impl Ending {
    /// Manda el último mensaje: si la transcripción sigue creciendo después de
    /// cerrarse, deja de estar cerrada.
    pub(super) fn add(&mut self, turn: Turn) {
        match turn {
            Turn::Assistant { handoffs, ended } => {
                if !handoffs.is_empty() {
                    self.pending = handoffs;
                }
                self.closed = ended;
            }
            Turn::User { results } => {
                self.closed = results.iter().any(|id| self.pending.contains(id));
            }
        }
    }

    pub(super) fn closed(&self) -> bool {
        self.closed
    }
}
