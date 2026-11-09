//! Formato de salida: tiempos relativos, fechas cortas y rutas abreviadas.

use crate::status::Period;
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, Utc};
use std::path::Path;

/// "20 s ago", "3 min ago", "2 h 30 min ago" por debajo de un día; "4 d ago"
/// hasta los 29 días; a partir de 30, la fecha local US ("10/05/2026"). Un instante
/// futuro (relojes desajustados) cuenta como ahora.
pub fn relative(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - at).num_seconds().max(0);
    let days = secs / 86_400;
    if days >= 30 {
        return at.with_timezone(&Local).format("%m/%d/%Y").to_string();
    }
    if days >= 1 {
        return format!("{days} d ago");
    }
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    match (h, m) {
        (0, 0) => format!("{secs} s ago"),
        (0, m) => format!("{m} min ago"),
        (h, m) => format!("{h} h {m} min ago"),
    }
}

/// La última llamada de siempre, o "never" si el agente no se ha usado.
pub fn last_seen(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    match at {
        Some(at) => relative(at, now),
        None => "never".to_string(),
    }
}

/// Tiempo trabajado: "<1 min", "N min" por debajo de una hora y "N h M min"
/// a partir de ahí.
pub fn worked(d: chrono::TimeDelta) -> String {
    let mins = d.num_minutes().max(0);
    match (mins / 60, mins % 60) {
        (0, 0) => "<1 min".to_string(),
        (0, m) => format!("{m} min"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// Lo que lleva una espera: "1.2 s" con una décima (truncada, nunca
/// "60.0 s") por debajo del minuto; desde ahí, como `worked` ("2 min").
pub fn elapsed(d: chrono::TimeDelta) -> String {
    let tenths = d.num_milliseconds().max(0) / 100;
    if tenths < 600 {
        format!("{}.{} s", tenths / 10, tenths % 10)
    } else {
        worked(d)
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn month(m: u32) -> &'static str {
    MONTHS[(m as usize).saturating_sub(1) % 12]
}

/// Fecha y hora local corta de una ejecución: "Nov 4 18:12".
pub fn day_time(at: NaiveDateTime) -> String {
    format!("{} {} {}", month(at.month()), at.day(), at.format("%H:%M"))
}

/// "Oct → Nov" si los días de `from` a `to` cruzan de mes; si no, nada.
pub fn month_span(from: NaiveDate, to: NaiveDate) -> Option<String> {
    (from.month() != to.month() || from.year() != to.year())
        .then(|| format!("{} → {}", month(from.month()), month(to.month())))
}

/// Nombre del periodo del ranking: "7 days", "30 days" o "all time".
pub fn period(p: Period) -> &'static str {
    match p {
        Period::Week => "7 days",
        Period::Month => "30 days",
        Period::Ever => "all time",
    }
}

/// Una décima sin el `.0` final: "1.5", "1".
fn tenths(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

/// Tokens en corto: tal cual por debajo de mil; "1.5k" hasta 10k, "412k" hasta
/// el millón, "1.1M"/"31.4M" hasta 100M y "142M" a partir de ahí. Sin `.0`.
pub fn tokens(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let k = n as f64 / 1e3;
    if (k * 10.0).round() < 100.0 {
        return format!("{}k", tenths(k));
    }
    if k.round() < 1_000.0 {
        return format!("{}k", k.round());
    }
    let m = n as f64 / 1e6;
    if (m * 10.0).round() < 1_000.0 {
        format!("{}M", tenths(m))
    } else {
        format!("{}M", m.round())
    }
}

/// Nombre del modelo sin el prefijo `claude-` ni la fecha final
/// (`claude-haiku-4-5-20251001` → `haiku-4-5`).
pub fn model(name: &str) -> String {
    let name = name.strip_prefix("claude-").unwrap_or(name);
    match name.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => {
            head.to_string()
        }
        _ => name.to_string(),
    }
}

/// Sustituye el prefijo del home por `~` para que las rutas quepan en la tabla.
pub fn shorten(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Una ruta de dentro del proyecto, relativa a su raíz ("./front/src"); las
/// de fuera, con `~` como `shorten`.
pub fn within(path: &Path, root: Option<&Path>, home: &Path) -> String {
    match root.and_then(|r| path.strip_prefix(r).ok()) {
        Some(rest) if !rest.as_os_str().is_empty() => format!("./{}", rest.display()),
        _ => shorten(path, home),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn relative_time_in_seconds_minutes_and_hours() {
        let now = t("2026-11-05T12:00:00Z");
        assert_eq!(relative(t("2026-11-05T11:59:40Z"), now), "20 s ago");
        assert_eq!(relative(t("2026-11-05T11:57:00Z"), now), "3 min ago");
        assert_eq!(relative(t("2026-11-05T09:30:00Z"), now), "2 h 30 min ago");
    }

    #[test]
    fn relative_time_in_days_from_one_to_twenty_nine() {
        let now = t("2026-11-05T12:00:00Z");
        assert_eq!(relative(t("2026-11-04T12:00:01Z"), now), "23 h 59 min ago");
        assert_eq!(relative(t("2026-11-04T12:00:00Z"), now), "1 d ago");
        assert_eq!(relative(t("2026-11-03T02:00:00Z"), now), "2 d ago");
        assert_eq!(relative(t("2026-10-06T12:00:01Z"), now), "29 d ago");
    }

    #[test]
    fn thirty_days_or_more_is_the_local_date() {
        let now = t("2026-11-05T12:00:00Z");
        // a mediodía UTC la fecha local es la misma en casi cualquier huso
        assert_eq!(relative(t("2026-10-06T12:00:00Z"), now), "10/06/2026");
        assert_eq!(relative(t("2025-03-01T12:00:00Z"), now), "03/01/2025");
    }

    #[test]
    fn last_call_ever_or_never() {
        let now = t("2026-11-05T12:00:00Z");
        assert_eq!(last_seen(Some(t("2026-11-05T11:57:00Z")), now), "3 min ago");
        assert_eq!(last_seen(None, now), "never");
    }

    #[test]
    fn elapsed_in_tenths_of_a_second_then_like_worked() {
        let ms = chrono::TimeDelta::milliseconds;
        assert_eq!(elapsed(ms(0)), "0.0 s");
        assert_eq!(elapsed(ms(1_200)), "1.2 s");
        // se trunca: nunca sale «60.0 s»
        assert_eq!(elapsed(ms(59_999)), "59.9 s");
        assert_eq!(elapsed(ms(60_000)), "1 min");
        assert_eq!(elapsed(ms((3 * 60 + 5) * 60_000)), "3 h 5 min");
        assert_eq!(elapsed(ms(-500)), "0.0 s");
    }

    #[test]
    fn future_timestamps_count_as_now() {
        let now = t("2026-11-05T12:00:00Z");
        assert_eq!(relative(t("2026-11-05T12:00:05Z"), now), "0 s ago");
    }

    #[test]
    fn worked_time_in_minutes_or_hours() {
        let m = chrono::TimeDelta::minutes;
        assert_eq!(worked(chrono::TimeDelta::seconds(59)), "<1 min");
        assert_eq!(worked(m(0)), "<1 min");
        assert_eq!(worked(m(1)), "1 min");
        assert_eq!(worked(m(59) + chrono::TimeDelta::seconds(59)), "59 min");
        assert_eq!(worked(m(60)), "1 h 0 min");
        assert_eq!(worked(m(65)), "1 h 5 min");
        assert_eq!(worked(m(72)), "1 h 12 min");
        assert_eq!(worked(m(26 * 60 + 3)), "26 h 3 min");
    }

    #[test]
    fn day_and_time_in_us_style_without_padding_the_day() {
        let at = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        assert_eq!(day_time(at("2026-11-04 18:12")), "Nov 4 18:12");
        assert_eq!(day_time(at("2026-10-23 09:05")), "Oct 23 09:05");
        assert_eq!(day_time(at("2026-01-01 00:00")), "Jan 1 00:00");
    }

    #[test]
    fn month_span_only_when_the_days_cross_a_month() {
        let d = |s: &str| s.parse::<chrono::NaiveDate>().unwrap();
        assert_eq!(
            month_span(d("2026-10-23"), d("2026-11-05")),
            Some("Oct → Nov".to_string())
        );
        assert_eq!(month_span(d("2026-11-01"), d("2026-11-14")), None);
        assert_eq!(
            month_span(d("2025-12-20"), d("2026-01-02")),
            Some("Dec → Jan".to_string())
        );
    }

    #[test]
    fn period_labels() {
        use crate::status::Period;
        assert_eq!(period(Period::Week), "7 days");
        assert_eq!(period(Period::Month), "30 days");
        assert_eq!(period(Period::Ever), "all time");
    }

    #[test]
    fn tokens_in_short_form() {
        assert_eq!(tokens(0), "0");
        assert_eq!(tokens(999), "999");
        assert_eq!(tokens(1_000), "1k");
        assert_eq!(tokens(1_540), "1.5k");
        assert_eq!(tokens(9_949), "9.9k");
        assert_eq!(tokens(9_960), "10k");
        assert_eq!(tokens(86_300), "86k");
        assert_eq!(tokens(412_000), "412k");
        assert_eq!(tokens(999_400), "999k");
        assert_eq!(tokens(999_600), "1M");
        assert_eq!(tokens(1_000_000), "1M");
        assert_eq!(tokens(1_120_000), "1.1M");
        assert_eq!(tokens(31_400_000), "31.4M");
        assert_eq!(tokens(99_960_000), "100M");
        assert_eq!(tokens(142_300_000), "142M");
    }

    #[test]
    fn model_names_without_the_claude_prefix_nor_the_date() {
        assert_eq!(model("claude-sonnet-5-5"), "sonnet-5-5");
        assert_eq!(model("claude-haiku-4-5-20251001"), "haiku-4-5");
        assert_eq!(model("gpt-x"), "gpt-x");
        assert_eq!(model("claude-opus-5-5[1m]"), "opus-5-5[1m]");
    }

    #[test]
    fn paths_inside_the_project_are_relative_to_it() {
        let home = Path::new("/Users/x");
        let root = Path::new("/Users/x/Offing/station");
        assert_eq!(
            within(
                Path::new("/Users/x/Offing/station/front/src"),
                Some(root),
                home
            ),
            "./front/src"
        );
        assert_eq!(
            within(Path::new("/Users/x/Offing/otro"), Some(root), home),
            "~/Offing/otro"
        );
        assert_eq!(within(Path::new("/Users/x/p"), None, home), "~/p");
    }

    #[test]
    fn home_prefix_is_shortened_to_tilde() {
        let home = Path::new("/Users/x");
        assert_eq!(
            shorten(Path::new("/Users/x/Offing/app/.claude/agents/a.md"), home),
            "~/Offing/app/.claude/agents/a.md"
        );
        assert_eq!(shorten(Path::new("/opt/app/a.md"), home), "/opt/app/a.md");
    }
}
