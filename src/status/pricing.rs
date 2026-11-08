//! Coste estimado de una ejecución en dólares.
//!
//! Es una ESTIMACIÓN a precios de lista de la API (Anthropic, octubre de
//! 2026): con una suscripción no se paga por token. La escritura en caché se
//! cuenta a la tarifa de 5 minutos (1,25× la entrada); Claude Code puede usar
//! la caché de 1 hora, cuya escritura cuesta 2× la entrada, así que la cifra
//! puede quedarse corta. Un modelo que no está en la tabla no tiene coste:
//! nunca se adivina.

use crate::runs::usage::Usage;

/// Dólares por millón de tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rates {
    input: f64,
    output: f64,
    /// Escritura en caché (5 minutos).
    cache_write: f64,
    cache_read: f64,
}

const fn rates(input: f64, output: f64, cache_write: f64, cache_read: f64) -> Rates {
    Rates {
        input,
        output,
        cache_write,
        cache_read,
    }
}

/// Precio por id de modelo. Cada clave encaja solo como modelo entero (ver
/// `matches`), así que `claude-opus-5` no encaja con `claude-opus-5-5`.
const TABLE: &[(&str, Rates)] = &[
    ("claude-fable-5-1", rates(10.0, 50.0, 12.5, 0.25)),
    ("claude-fable-5", rates(10.0, 50.0, 12.5, 1.00)),
    ("claude-mythos-5-1", rates(10.0, 50.0, 12.5, 0.25)),
    ("claude-opus-5-5", rates(4.0, 20.0, 5.0, 0.20)),
    ("claude-opus-5", rates(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-8", rates(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-7", rates(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-6", rates(5.0, 25.0, 6.25, 0.50)),
    ("claude-sonnet-5-5", rates(2.0, 10.0, 2.5, 0.20)),
    ("claude-sonnet-5", rates(2.0, 10.0, 2.5, 0.20)),
    ("claude-sonnet-4-6", rates(3.0, 15.0, 3.75, 0.30)),
    // Hasta 100K de prompt; por encima cuesta 0,50/2,50. Se ignora el tramo:
    // una ejecución con prompts largos sale más barata de lo que es.
    ("claude-haiku-5-5", rates(0.10, 0.50, 0.125, 0.01)),
    ("claude-haiku-4-5", rates(1.0, 5.0, 1.25, 0.10)),
];

/// `key` aparece en `model` como modelo entero: delante, el principio o algo
/// que no es letra, número ni guion (`us.anthropic.`); detrás, nada o, tras
/// una fecha opcional (`-20261101`), un sufijo de variante (`[1m]`, `@…`,
/// `:0`, `-v1`). Así `claude-opus-5-1` no pasa por `claude-opus-5`.
fn matches(model: &str, key: &str) -> bool {
    model.match_indices(key).any(|(at, _)| {
        let before_ok = model[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-'));
        let mut rest = &model[at + key.len()..];
        if let Some(date) = rest
            .strip_prefix('-')
            .filter(|d| d.len() >= 8 && d.as_bytes()[..8].iter().all(u8::is_ascii_digit))
        {
            rest = &date[8..];
        }
        let after_ok =
            rest.is_empty() || rest.starts_with(['[', '@', ':']) || rest.starts_with("-v");
        before_ok && after_ok
    })
}

fn rates_of(model: &str) -> Option<Rates> {
    let model = model.to_ascii_lowercase();
    TABLE
        .iter()
        .find(|(key, _)| matches(&model, key))
        .map(|(_, r)| *r)
}

/// Coste estimado en USD de `usage` a las tarifas de su modelo; `None` sin
/// modelo o con uno que no está en la tabla.
pub fn cost(usage: &Usage) -> Option<f64> {
    let r = rates_of(usage.model.as_deref()?)?;
    let t = &usage.tokens;
    let per_m = |n: u64, rate: f64| n as f64 * rate / 1_000_000.0;
    Some(
        per_m(t.input, r.input)
            + per_m(t.output, r.output)
            + per_m(t.cache_creation, r.cache_write)
            + per_m(t.cache_read, r.cache_read),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::usage::Tokens;

    fn input_rate(model: &str) -> Option<f64> {
        rates_of(model).map(|r| r.input)
    }

    #[test]
    fn the_longer_id_never_falls_into_the_shorter_one() {
        assert_eq!(input_rate("claude-opus-5-5"), Some(4.0));
        assert_eq!(input_rate("claude-opus-5"), Some(5.0));
        assert_eq!(rates_of("claude-fable-5-1").unwrap().cache_read, 0.25);
        assert_eq!(rates_of("claude-fable-5").unwrap().cache_read, 1.00);
        assert_eq!(input_rate("claude-sonnet-4-6"), Some(3.0));
        assert_eq!(input_rate("claude-sonnet-5"), Some(2.0));
        assert_eq!(input_rate("claude-haiku-5-5"), Some(0.10));
        assert_eq!(input_rate("claude-haiku-4-5"), Some(1.0));
        assert_eq!(input_rate("claude-opus-4-7"), Some(5.0));
        assert_eq!(input_rate("claude-mythos-5-1"), Some(10.0));
    }

    #[test]
    fn suffixes_and_prefixes_are_ignored() {
        assert_eq!(input_rate("claude-opus-5-5[1m]"), Some(4.0));
        assert_eq!(input_rate("claude-haiku-4-5-20251001"), Some(1.0));
        assert_eq!(input_rate("claude-opus-5-20260301"), Some(5.0));
        assert_eq!(input_rate("us.anthropic.claude-sonnet-4-6-v1:0"), Some(3.0));
        assert_eq!(input_rate("claude-opus-5-5@20261101"), Some(4.0));
        assert_eq!(input_rate("Claude-Opus-5-5"), Some(4.0));
    }

    #[test]
    fn an_unknown_model_has_no_rates() {
        // ni por prefijo: una versión que no está en la tabla no se adivina
        assert_eq!(rates_of("claude-opus-5-1"), None);
        assert_eq!(rates_of("claude-sonnet-4-5"), None);
        assert_eq!(rates_of("xclaude-opus-5"), None);
        assert_eq!(rates_of("<synthetic>"), None);
        assert_eq!(rates_of(""), None);
    }

    #[test]
    fn cost_adds_the_four_counters_at_their_rates() {
        let usage = Usage {
            tokens: Tokens {
                input: 1_000_000,
                output: 100_000,
                cache_read: 2_000_000,
                cache_creation: 400_000,
            },
            model: Some("claude-opus-5-5".into()),
            context: None,
            context_window: 200_000,
        };
        // 4 + 2 + 0,40 + 2
        let got = cost(&usage).unwrap();
        assert!((got - 8.40).abs() < 1e-9, "{got}");
        assert_eq!(
            cost(&Usage {
                model: None,
                ..usage.clone()
            }),
            None
        );
        assert_eq!(
            cost(&Usage {
                model: Some("claude-opus-9".into()),
                ..usage
            }),
            None
        );
        assert_eq!(
            cost(&Usage {
                model: Some("claude-haiku-4-5".into()),
                ..Usage::default()
            }),
            Some(0.0)
        );
    }
}
