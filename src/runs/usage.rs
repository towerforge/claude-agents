//! Tokens de una ejecución, sacados de `message.usage` y `message.model` de
//! sus líneas de asistente.
//!
//! Claude Code escribe una línea por bloque de contenido (`thinking`,
//! `text`, `tool_use`…) y todas repiten el mismo `message.id` con su `usage`;
//! mientras se escribe, `output_tokens` crece de una línea a la siguiente
//! (visto en disco: 9 → 9 → 305 en el mismo mensaje). Por eso se cuenta un
//! mensaje una sola vez, con el último `usage` visto de su `id`.

use serde_json::Value;
use std::collections::HashMap;

/// Ventana de contexto por defecto de los modelos de Claude.
pub const DEFAULT_WINDOW: u64 = 200_000;
/// Ventana larga (beta de 1M de tokens).
pub const LARGE_WINDOW: u64 = 1_000_000;

/// Ventana de contexto de una ejecución. No hay campo que la diga, así que:
/// 1M si el id del modelo lleva `[1m]` (así lo escribe Claude Code cuando se
/// elige la variante de contexto largo) o si algún mensaje de la ejecución
/// pasó de 200k de contexto (es imposible con la ventana normal: en disco hay
/// ejecuciones de opus y fable con más de 900k y el id sin `[1m]`); si no, 200k.
pub fn context_window(model: Option<&str>, max_context: u64) -> u64 {
    if model.is_some_and(|m| m.contains("[1m]")) || max_context > DEFAULT_WINDOW {
        LARGE_WINDOW
    } else {
        DEFAULT_WINDOW
    }
}

/// Tokens de un mensaje o de una ejecución.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    /// `input_tokens`: entrada sin caché.
    pub input: u64,
    pub output: u64,
    /// `cache_read_input_tokens`.
    pub cache_read: u64,
    /// `cache_creation_input_tokens`.
    pub cache_creation: u64,
}

impl Tokens {
    /// Los cuatro sumados.
    // Sin uso hasta que frontend/visual lo muestren; quitar este allow entonces.
    #[allow(dead_code)]
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_creation
    }

    /// Contexto que ocupa el mensaje: todo lo que entró (con caché o sin ella).
    pub fn context(&self) -> u64 {
        self.input + self.cache_read + self.cache_creation
    }

    fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_creation += o.cache_creation;
    }

    fn sub(&mut self, o: &Tokens) {
        self.input = self.input.saturating_sub(o.input);
        self.output = self.output.saturating_sub(o.output);
        self.cache_read = self.cache_read.saturating_sub(o.cache_read);
        self.cache_creation = self.cache_creation.saturating_sub(o.cache_creation);
    }
}

/// Consumo de una ejecución tal como lo ve la pantalla.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    /// Suma de todos sus mensajes, cada uno una vez.
    pub tokens: Tokens,
    /// `message.model` del último mensaje con uso (`claude-opus-5-5`…).
    pub model: Option<String>,
    /// Contexto del último mensaje: `input + cache_read + cache_creation`.
    /// `None` si aún no hay ninguno.
    pub context: Option<u64>,
    /// Ventana de contexto según `context_window`.
    pub context_window: u64,
}

/// El uso de una línea de asistente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LineUsage {
    /// `message.id`; sin él se usa el `uuid` de la línea y, sin ninguno, la
    /// línea cuenta sola.
    pub(super) key: Option<String>,
    pub(super) model: Option<String>,
    pub(super) tokens: Tokens,
}

/// `message.usage` de una línea de asistente. Las de `<synthetic>` (mensajes
/// que Claude Code se inventa, con uso 0) no cuentan.
pub(super) fn usage_of(doc: &Value) -> Option<LineUsage> {
    if doc.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let message = doc.get("message")?;
    let model = message.get("model").and_then(Value::as_str);
    if model == Some("<synthetic>") {
        return None;
    }
    let usage = message.get("usage")?.as_object()?;
    let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    let key = message
        .get("id")
        .or_else(|| doc.get("uuid"))
        .and_then(Value::as_str);
    Some(LineUsage {
        key: key.map(str::to_string),
        model: model.map(str::to_string),
        tokens: Tokens {
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_read: n("cache_read_input_tokens"),
            cache_creation: n("cache_creation_input_tokens"),
        },
    })
}

/// Lo acumulado del uso de una transcripción, al día con cada línea.
#[derive(Debug, Default)]
pub(super) struct UsageAcc {
    /// Último uso visto de cada mensaje: lo que hay que restar si vuelve.
    by_message: HashMap<String, Tokens>,
    total: Tokens,
    model: Option<String>,
    last_context: Option<u64>,
    max_context: u64,
}

impl UsageAcc {
    pub(super) fn add(&mut self, line: LineUsage) {
        if let Some(key) = line.key {
            if let Some(old) = self.by_message.insert(key, line.tokens) {
                self.total.sub(&old);
            }
        }
        self.total.add(&line.tokens);
        let context = line.tokens.context();
        self.last_context = Some(context);
        self.max_context = self.max_context.max(context);
        if line.model.is_some() {
            self.model = line.model;
        }
    }

    pub(super) fn usage(&self) -> Usage {
        Usage {
            tokens: self.total,
            model: self.model.clone(),
            context: self.last_context,
            context_window: context_window(self.model.as_deref(), self.max_context),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Línea literal de una transcripción real (recortada): un bloque
    /// `thinking` de un mensaje que sigue en otra línea con su `tool_use`.
    const THINKING: &str = r#"{"parentUuid":"p","isSidechain":true,"agentId":"a1","message":{"model":"claude-sonnet-5-5","id":"msg_011CgjkRpBLEFNVEBTioBM1A","type":"message","role":"assistant","content":[{"type":"thinking","thinking":"..."}],"usage":{"input_tokens":2,"cache_creation_input_tokens":7046,"cache_read_input_tokens":17316,"output_tokens":9,"output_tokens_details":{"thinking_tokens":263},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":0,"ephemeral_5m_input_tokens":7046}}},"type":"assistant","uuid":"u1","timestamp":"2026-11-05T10:00:00.000Z"}"#;
    const TOOL_USE: &str = r#"{"parentUuid":"u1","isSidechain":true,"agentId":"a1","message":{"model":"claude-sonnet-5-5","id":"msg_011CgjkRpBLEFNVEBTioBM1A","type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{}}],"usage":{"input_tokens":2,"cache_creation_input_tokens":7046,"cache_read_input_tokens":17316,"output_tokens":470}},"type":"assistant","uuid":"u2","timestamp":"2026-11-05T10:00:01.000Z"}"#;
    const NEXT: &str = r#"{"message":{"model":"claude-sonnet-5-5","id":"msg_2","role":"assistant","content":[{"type":"text","text":"fin"}],"usage":{"input_tokens":4,"cache_creation_input_tokens":100,"cache_read_input_tokens":24362,"output_tokens":50}},"type":"assistant","uuid":"u3"}"#;

    fn parse(line: &str) -> Option<LineUsage> {
        usage_of(&serde_json::from_str(line).unwrap())
    }

    #[test]
    fn reads_the_four_counters_and_the_model() {
        let u = parse(TOOL_USE).unwrap();
        assert_eq!(u.key.as_deref(), Some("msg_011CgjkRpBLEFNVEBTioBM1A"));
        assert_eq!(u.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(
            u.tokens,
            Tokens {
                input: 2,
                output: 470,
                cache_read: 17316,
                cache_creation: 7046
            }
        );
        assert_eq!(u.tokens.total(), 2 + 470 + 17316 + 7046);
        assert_eq!(u.tokens.context(), 2 + 17316 + 7046);
    }

    #[test]
    fn users_synthetic_and_lines_without_usage_do_not_count() {
        assert!(parse(r#"{"type":"user","message":{"usage":{"input_tokens":5}}}"#).is_none());
        assert!(parse(r#"{"type":"assistant","message":{"model":"<synthetic>","id":"x","usage":{"input_tokens":0,"output_tokens":0}}}"#).is_none());
        assert!(parse(r#"{"type":"assistant","message":{"content":[]}}"#).is_none());
        // Campos que faltan valen 0; sin `id` vale el `uuid` de la línea.
        let u =
            parse(r#"{"type":"assistant","uuid":"u9","message":{"usage":{"output_tokens":3}}}"#)
                .unwrap();
        assert_eq!(u.key.as_deref(), Some("u9"));
        assert_eq!(u.tokens.total(), 3);
    }

    #[test]
    fn a_message_split_over_several_lines_counts_once_with_its_last_usage() {
        let mut acc = UsageAcc::default();
        for l in [THINKING, TOOL_USE, TOOL_USE, NEXT] {
            acc.add(parse(l).unwrap());
        }
        let u = acc.usage();
        assert_eq!(
            u.tokens,
            Tokens {
                input: 2 + 4,
                output: 470 + 50,
                cache_read: 17316 + 24362,
                cache_creation: 7046 + 100
            }
        );
        assert_eq!(u.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(u.context, Some(4 + 24362 + 100));
        assert_eq!(u.context_window, DEFAULT_WINDOW);
        assert_eq!(UsageAcc::default().usage().context, None);
    }

    #[test]
    fn the_window_is_1m_with_the_suffix_or_over_200k_of_context() {
        assert_eq!(
            context_window(Some("claude-opus-5-5"), 150_000),
            DEFAULT_WINDOW
        );
        assert_eq!(
            context_window(Some("claude-opus-5-5[1m]"), 10),
            LARGE_WINDOW
        );
        assert_eq!(
            context_window(Some("claude-opus-5-5"), 200_001),
            LARGE_WINDOW
        );
        assert_eq!(context_window(None, 200_000), DEFAULT_WINDOW);
        // Una vez pasado de 200k, sigue en 1M aunque el contexto baje (compactado).
        let mut acc = UsageAcc::default();
        let line = |id: &str, read: u64| LineUsage {
            key: Some(id.into()),
            model: Some("claude-opus-5-5".into()),
            tokens: Tokens {
                cache_read: read,
                ..Tokens::default()
            },
        };
        acc.add(line("m1", 250_000));
        acc.add(line("m2", 30_000));
        assert_eq!(acc.usage().context, Some(30_000));
        assert_eq!(acc.usage().context_window, LARGE_WINDOW);
    }
}
