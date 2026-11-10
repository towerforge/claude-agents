//! Pintado de la pantalla: cabecera con pestañas, el monitor de activos (bloques
//! de tres o cuatro líneas por proyecto, en curso y terminados en la ventana), la lista
//! por grupos de todos, ranking, ficha y pie. Misma gramática que las listas de moon:
//! `❯` en la fila del cursor, número en cada fila seleccionable, reglas
//! ` título ─── info ` y pie con las teclas. Aquí no se decide nada: se pinta
//! lo que da `App`.

use crate::report;
use crate::status::{
    Chain, Context, LaunchedBy, Period, PeriodStats, RankKey, RunDetail, RunKind, RunLink, RunState,
};
use crate::tui::app::{
    App, Hits, Item, LastUse, Mode, NowLine, Order, Row, RowHit, RunLine, TabHit, DAYS, MAX_DAYS,
};
use crate::tui::theme::Theme;
use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const LIST_KEYS: &[(&str, &str)] = &[
    ("↑↓", "move"),
    ("enter", "details"),
    ("tab", "mode"),
    ("/", "filter"),
    ("o", "sort"),
    ("q", "quit"),
];

/// Activos: sin orden, que el monitor va siempre del más reciente al más
/// antiguo.
const ACTIVE_KEYS: &[(&str, &str)] = &[
    ("↑↓", "move"),
    ("enter", "details"),
    ("tab", "mode"),
    ("/", "filter"),
    ("q", "quit"),
];

const RANKING_KEYS: &[(&str, &str)] = &[
    ("↑↓", "move"),
    ("enter", "details"),
    ("tab", "mode"),
    ("p", "period"),
    ("s", "by"),
    ("q", "quit"),
];

/// Las teclas del pie que se quitan, en este orden, cuando no cabe el contador.
const MINOR_KEYS: &[&str] = &["o", "/", "enter"];

/// Pie mientras carga: sin lista, mover, abrir, filtrar o cambiar de modo aún
/// no muestran nada.
const LOADING_KEYS: &[(&str, &str)] = &[("q", "quit")];
/// Columnas mínimas del texto de la carga tras la estrella (una letra y `…`);
/// con menos, solo la estrella.
const LOADING_TEXT_MIN: usize = 2;
/// Columnas mínimas de la ruta de la carga; con menos, solo el tiempo.
const LOADING_PATH_MIN: usize = 4;

const DETAIL_KEYS: &[(&str, &str)] = &[("esc", "back"), ("↑↓", "other agent"), ("q", "quit")];

/// Pie de la ficha de ejecución. Cada una sale solo si vale (`run_footer`).
const RUN_KEYS: &[(&str, &str)] = &[
    ("esc", "back"),
    // ←→ solo sale si el agente tiene más de una ejecución (`run_nav`).
    ("↑↓", "other agent"),
    ("←→", "other run"),
    ("tab", "launched"),
    ("enter", "open"),
    ("pgup/pgdn", "steps"),
    ("p", "parent"),
    ("a", "agent"),
    ("q", "quit"),
];
/// Las teclas de la ficha de ejecución que se quitan, en este orden, si el
/// pie no cabe (esc y q nunca).
const RUN_MINOR_KEYS: &[&str] = &["←→", "a", "pgup/pgdn", "↑↓", "p", "tab", "enter"];
/// Ancho mínimo de la ruta temporal en el aviso; con menos, no sale.
const NOTICE_PATH_MIN: usize = 8;
/// Líneas como mucho del encargo y de las herramientas en la ficha de ejecución.
const RUN_TASK_MAX: usize = 8;
const RUN_TOOLS_MAX: usize = 3;
/// Hijas como mucho en «launched»; con más, una ventana que sigue a la elegida.
const RUN_LAUNCHED_MAX: usize = 6;
/// Ancho de la etiqueta de las filas de uso («cache write» y dos de hueco).
const USAGE_LABEL_W: usize = 13;
/// La barra de contexto de la ficha: elástica entre estos dos anchos; con
/// menos sitio, no sale.
const USAGE_GAUGE_MIN: usize = 8;
const USAGE_GAUGE_MAX: usize = 40;
/// Pasos que se reservan antes de dar sitio al encargo.
const RUN_STEPS_MIN: usize = 3;
/// Desde este alto el cuerpo de la ficha deja una línea en blanco tras la cabecera.
const RUN_SPACER_MIN_H: usize = 14;
/// Desde este ancho sale «lleva N min» en la cabecera de la ficha de ejecución.
const RUN_WORKED_MIN_W: usize = 70;
/// Desde este ancho sale la etiqueta del tipo en la cabecera de la ficha.
const RUN_TAG_MIN_W: usize = 50;

/// Por debajo de este ancho la lista no lleva barrita de días.
const SPARK_MIN_W: usize = 100;
/// Por debajo de este ancho la lista no lleva la columna de ejecuciones.
const RUNS_MIN_W: usize = 80;
/// Desde este ancho de terminal salen las columnas «ctx» y «tokens» de
/// activos; por debajo solo queda «time».
const LIVE_FIGURES_MIN_W: usize = 70;
/// Desde este ancho de terminal sale la etiqueta tras el nombre del bloque
/// en curso («[propio]», «[claude]», «vía workflow»).
const LIVE_TAG_MIN_W: usize = 50;
/// Ancho de la etiqueta (`skill`, `now`) de las sublíneas de una fila en curso.
const LIVE_LABEL_W: usize = 7;
/// Ancho de la guía del racimo (`│ ` o dos espacios) delante de esa etiqueta.
const LIVE_GUIDE_W: usize = 2;
/// Entre el nombre (con su etiqueta) y la tarea de una fila de activos.
const TASK_GAP: usize = 2;
/// La barra del ranking llena el hueco que dejan las columnas; si el hueco no
/// llega a `BAR_MIN`, no sale.
const BAR_MIN: usize = 8;
/// Alto del gráfico de la ficha, en filas.
const CHART_ROWS: usize = 4;
/// Bloques del gráfico de la ficha, de un octavo a lleno.
const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
/// El cero de la barrita de «todos»: raya fina, como una regla. Lo demás usa
/// las rayas del ranking (`RANK_HALF` y `RANK_FULL`), un carácter por día.
const SPARK_ZERO: char = '─';
/// Fracciones de la barra horizontal, de un octavo a siete.
const EIGHTHS: [char; 7] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉'];
/// La barra del ranking: raya gruesa por celda llena y media raya por media
/// celda. No llena el alto de la celda, así que las barras de filas seguidas
/// no se pegan como los bloques.
const RANK_FULL: char = '━';
const RANK_HALF: char = '╸';
/// Ancho máximo de la columna del proyecto en el ranking: un nombre largo se
/// corta con `…` en vez de empujar las cifras fuera de la pantalla.
const RANK_PROJECT_MAX: usize = 18;

/// Columnas del hueco de la estrella (`✦ `) entre el número y el nombre: se
/// reserva siempre, esté la fila en curso o no, para que los nombres alineen.
const STAR_W: usize = 2;
/// Huecos entre columnas: tras el nombre, tras la barrita y entre las de datos.
const NAME_GAP: usize = 2;
const SPARK_GAP: usize = 2;
const COL_GAP: usize = 3;

pub(super) fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Corta a `max` columnas y termina en `…` si hubo que cortar.
pub(super) fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += cw;
    }
    if max > 0 {
        out.push('…');
    }
    out
}

/// Parte un texto en líneas de como mucho `max` columnas, por palabras.
pub(super) fn wrap(text: &str, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && width(&line) + 1 + width(word) > max {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `s` alineado a la izquierda (`right == false`) o a la derecha en `w` columnas.
fn pad(s: &str, w: usize, right: bool) -> String {
    let fill = " ".repeat(w.saturating_sub(width(s)));
    if right {
        format!("{fill}{s}")
    } else {
        format!("{s}{fill}")
    }
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| width(&s.content)).sum()
}

/// Izquierda y derecha en la misma línea; si no caben las dos, gana la izquierda.
fn split_line(
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    w: usize,
    t: &Theme,
) -> Line<'static> {
    let lw = spans_width(&left);
    let rw = spans_width(&right);
    let mut spans = left;
    if let Some(fill) = w.checked_sub(lw + rw + 1) {
        spans.push(Span::styled(" ".repeat(fill), t.text()));
        spans.extend(right);
    }
    Line::from(spans)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Fracción `v / max` como número real, 0 si `max` es cero.
fn ratio(v: TimeDelta, max: TimeDelta) -> f64 {
    let max = max.num_seconds();
    if max <= 0 {
        0.0
    } else {
        v.num_seconds().max(0) as f64 / max as f64
    }
}

/// Barrita de días: un carácter por día, con el estilo de la barra del
/// ranking. `─` es el cero; hasta la mitad del máximo de la fila, media raya
/// `╸`; por encima, raya entera `━`. Lo poco que no sea cero nunca se
/// confunde con él.
fn spark(days: &[TimeDelta]) -> String {
    let max = days.iter().copied().max().unwrap_or_default();
    days.iter()
        .map(|&d| {
            if d <= TimeDelta::zero() {
                SPARK_ZERO
            } else if ratio(d, max) <= 0.5 {
                RANK_HALF
            } else {
                RANK_FULL
            }
        })
        .collect()
}

/// La barrita en tramos de un color: el cero (`─`) en `subtle`, las rayas
/// en `claude-soft`.
fn spark_spans(days: &[TimeDelta], t: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut run = String::new();
    let mut zero = None;
    let style = |z: bool| if z { t.line() } else { t.soft() };
    for ch in spark(days).chars() {
        let z = ch == SPARK_ZERO;
        if zero.is_some_and(|prev| prev != z) {
            spans.push(Span::styled(std::mem::take(&mut run), style(!z)));
        }
        zero = Some(z);
        run.push(ch);
    }
    if let Some(z) = zero {
        spans.push(Span::styled(run, style(z)));
    }
    spans
}

/// Barra horizontal: `width` columnas para `max`, con bloques enteros y la
/// fracción final en octavos. Lo que no es cero se ve siempre.
fn hbar(v: f64, max: f64, width: usize) -> String {
    if v <= 0.0 || width == 0 {
        return String::new();
    }
    let r = if max > 0.0 { v / max } else { 0.0 };
    let eighths = ((r * (width * 8) as f64).round() as usize).clamp(1, width * 8);
    let (full, rest) = (eighths / 8, eighths % 8);
    let mut bar = "█".repeat(full);
    if rest > 0 {
        bar.push(EIGHTHS[rest - 1]);
    }
    bar
}

/// Barra del ranking: `━` por celda llena y `╸` por media celda; no llena el
/// alto de la celda, así que no se pega a la fila de abajo. Lo que no es cero
/// se ve siempre, aunque sea media celda.
fn rank_bar(v: f64, max: f64, width: usize) -> String {
    if v <= 0.0 || width == 0 {
        return String::new();
    }
    let r = if max > 0.0 { v / max } else { 0.0 };
    let halves = ((r * (width * 2) as f64).round() as usize).clamp(1, width * 2);
    let mut bar = RANK_FULL.to_string().repeat(halves / 2);
    if halves % 2 == 1 {
        bar.push(RANK_HALF);
    }
    bar
}

/// La celda del gráfico de la ficha en la fila `level` (1 abajo, `CHART_ROWS`
/// arriba): llena, en octavos si es la cima de la columna, o vacía.
fn chart_cell(v: TimeDelta, max: TimeDelta, level: usize) -> char {
    if v <= TimeDelta::zero() {
        return ' ';
    }
    let h = ratio(v, max) * CHART_ROWS as f64;
    let below = (level - 1) as f64;
    if h >= level as f64 {
        '█'
    } else if h > below {
        let e = ((h - below) * 8.0).round() as usize;
        LEVELS[e.clamp(1, 8) - 1]
    } else {
        ' '
    }
}

/// La marca de una fila en curso, tras el nombre: ` ✦`/` ✧` en `claude`.
/// La misma en «todos» y en el ranking.
fn star_mark(app: &App, t: &Theme) -> Span<'static> {
    Span::styled(format!(" {}", star(app.now)), t.accent())
}

/// El hueco de la estrella de una fila de activos o de la ficha de ejecución:
/// `✦ `/`✧ ` en `claude` si está en curso; en blanco si terminó. Siempre
/// `STAR_W` columnas.
fn star_slot(running: bool, now: DateTime<Utc>, t: &Theme) -> Span<'static> {
    if running {
        Span::styled(format!("{} ", star(now)), t.accent())
    } else {
        Span::raw(" ".repeat(STAR_W))
    }
}

/// La marca de lo que está en curso: `✦` y `✧` cada medio segundo.
pub(super) fn star(now: DateTime<Utc>) -> &'static str {
    if now.timestamp_subsec_millis() < 500 {
        "✦"
    } else {
        "✧"
    }
}

/// La etiqueta de origen tras el nombre, con su espacio delante: `[own]`
/// para un agente de Jordi (definido o paso de workflow cuya `description` es
/// un agente definido; este último añade ` via workflow`), `[workflow]` para un
/// paso de workflow que no es ningún agente suyo y `[claude]` para uno de
/// Claude Code (built-in). Si no cabe entera en `room`, se queda con solo el
/// origen; si tampoco, nada.
fn kind_tag(kind: RunKind, room: usize) -> String {
    let (origin, extra) = match kind {
        RunKind::Defined => ("[own]", ""),
        RunKind::Workflow => ("[own]", " via workflow"),
        RunKind::WorkflowStep => ("[workflow]", ""),
        RunKind::BuiltIn => ("[claude]", ""),
    };
    let full = format!(" {origin}{extra}");
    if width(&full) <= room {
        full
    } else if width(origin) < room {
        format!(" {origin}")
    } else {
        String::new()
    }
}

/// La etiqueta de una fila de activos: la de `kind_tag` más ` ×N` en gris si
/// la fila funde varias ejecuciones (`runs > 1`). El orden es
/// `[own] via workflow ×3` (o `[workflow] ×3`); si no cabe en `room` se pierde
/// primero `via workflow`, luego ` ×N` y por último el origen: nombre > origen
/// > ×N > `via workflow`.
fn live_tag(kind: RunKind, runs: usize, room: usize) -> String {
    let (origin, extra) = match kind {
        RunKind::Defined => ("[own]", ""),
        RunKind::Workflow => ("[own]", " via workflow"),
        RunKind::WorkflowStep => ("[workflow]", ""),
        RunKind::BuiltIn => ("[claude]", ""),
    };
    let count = if runs > 1 {
        format!(" ×{runs}")
    } else {
        String::new()
    };
    [
        format!(" {origin}{extra}{count}"),
        format!(" {origin}{count}"),
        format!(" {origin}"),
    ]
    .into_iter()
    .find(|c| width(c) <= room)
    .unwrap_or_default()
}

/// El nombre según el estado, sin mirar el tipo: `claude-soft` en negrita en
/// curso, `ink` normal terminada.
fn state_name_style(done: bool, t: &Theme) -> Style {
    if done {
        t.text()
    } else {
        t.soft_bold()
    }
}

/// Hoy en la zona de la pantalla.
fn today(app: &App) -> NaiveDate {
    match app.tz {
        Some(off) => app.now.with_timezone(&off).date_naive(),
        None => app.now.with_timezone(&chrono::Local).date_naive(),
    }
}

pub fn render(app: &mut App, frame: &mut Frame, t: &Theme) {
    let area = frame.area();
    let w = area.width as usize;
    // cabecera, segunda línea, cuerpo, pie
    let body_h = area.height.saturating_sub(3) as usize;
    // el mapa de clics se rehace con lo que pinta este fotograma
    app.hits = Hits::default();
    let (top, tabs) = top_line(app, t, area);
    app.hits.tabs = tabs;
    let mut lines = vec![top, second_line(app, t, w)];
    let mut scrollbar = None;
    let footer = if app.run_detail.is_some() {
        lines.extend(run_detail_lines(app, t, w, body_h));
        run_footer(app, t, w)
    } else if app.detail && app.current().is_some() {
        lines.extend(detail_lines(app, t, w, body_h));
        footer_with_version(
            app,
            t,
            w,
            keys_spans(app, t, DETAIL_KEYS, false, false),
            None,
        )
    } else {
        let items = app.items();
        // el ranking con historial guardado lleva una nota bajo las filas, en
        // una línea que la lista deja libre
        let kept = match app.mode {
            Mode::Ranking if app.loaded && !items.is_empty() && body_h >= 2 => app.cached_only(),
            _ => 0,
        };
        let body_h = if kept > 0 { body_h - 1 } else { body_h };
        app.scroll_to_cursor(&items, body_h);
        // La barra ocupa la última columna: la lista se pinta una más estrecha.
        let bar = app.loaded && body_h > 0 && items.len() > body_h;
        let list_w = if bar { w.saturating_sub(1) } else { w };
        if bar {
            let body = Rect::new(area.x, area.y + 2, area.width, body_h as u16);
            scrollbar = Some((body, items.len()));
        }
        if !app.loaded {
            lines.extend(loading_lines(app, t, w, body_h));
        } else if items.is_empty() {
            let why = match (app.mode, app.filter.is_empty()) {
                // sin filtro, el monitor vacío es `Item::Quiet`
                (Mode::Active, _) => "adjust the filter or press tab".to_string(),
                (Mode::All, false) | (Mode::Ranking, false) => "adjust the filter".to_string(),
                (Mode::All, true) => "no .claude/agents/*.md in the roots".to_string(),
                (Mode::Ranking, true) => {
                    "no agent worked in this period · p changes the period".to_string()
                }
            };
            lines.push(Line::from(vec![
                Span::styled(" nothing to show", t.text()),
                Span::styled(format!(" · {why}"), t.muted()),
            ]));
        } else {
            // los umbrales responsive miran la terminal, no la barra de scroll
            let cols = Columns::of(app, &items, w, list_w);
            // la fila seleccionable de cada línea: la suya, o en un bloque de
            // activos la de su `Live`, aunque el scroll lo haya dejado arriba
            let mut block = None;
            for (i, item) in items.iter().enumerate().take(app.scroll + body_h) {
                let cursor = match item {
                    // la línea del agente (racimo o en curso sola) es de la fila de debajo
                    Item::LiveAgent(_) => items.get(i + 1).and_then(Item::n).map(|n| n - 1),
                    Item::LiveSkill(_) | Item::LiveNow(_) => block,
                    _ => item.n().map(|n| n - 1),
                };
                if matches!(item, Item::Live { .. }) {
                    block = cursor;
                }
                if i < app.scroll {
                    continue;
                }
                let first = lines.len();
                lines.extend(item_lines(app, item, &cols, t, list_w));
                if let Some(cursor) = cursor {
                    // solo las líneas que caben en el cuerpo
                    for y in first..lines.len().min(2 + body_h) {
                        app.hits.rows.push(RowHit {
                            row: area.y + y as u16,
                            cursor,
                        });
                    }
                }
            }
            if kept > 0 {
                // con sitio, separada de las filas por una línea en blanco
                if lines.len() < 2 + body_h {
                    lines.push(Line::default());
                }
                lines.truncate(2 + body_h);
                let note = format!(
                    "history kept: {} older than Claude's cleanup",
                    plural(kept, "run", "runs")
                );
                lines.push(Line::from(Span::styled(
                    format!(" {}", truncate(&note, w.saturating_sub(2))),
                    t.muted(),
                )));
            }
        }
        if app.loaded {
            footer_line(app, t, w, items.len(), body_h)
        } else {
            let keys = keys_spans(app, t, LOADING_KEYS, false, false);
            footer_with_version(app, t, w, keys, None)
        }
    };
    while lines.len() < 2 + body_h {
        lines.push(Line::default());
    }
    lines.truncate(2 + body_h);
    lines.push(footer);
    frame.render_widget(Paragraph::new(lines), area);
    if let Some((body, total)) = scrollbar {
        render_scrollbar(frame, t, body, total, body_h, app.scroll);
    }
}

/// ```text
///                      ✦ loading agents
///                ~/.claude/projects · 1.2 s
/// ```
///
/// El cuerpo mientras llega la primera lectura, centrado en los dos ejes: la
/// estrella que parpadea en `claude` y el texto en `ink`; debajo, en gris, la
/// carpeta de las transcripciones (la lectura más larga) y lo que se lleva
/// esperando. Estrecha, la ruta se corta con `…` y luego se va, quedando el
/// tiempo; el texto se corta igual y, si no queda ni una letra, solo la
/// estrella. Baja, sin sitio para dos líneas, solo la primera.
fn loading_lines(app: &App, t: &Theme, w: usize, body_h: usize) -> Vec<Line<'static>> {
    // una columna libre a cada lado, como el resto de la pantalla
    let room = w.saturating_sub(2);
    let mut first = vec![Span::styled(star(app.now), t.accent())];
    if room >= 2 + LOADING_TEXT_MIN {
        first.push(Span::styled(
            format!(" {}", truncate("loading agents", room - 2)),
            t.text(),
        ));
    }
    let mut shown = vec![centered(first, w)];
    let time = report::elapsed(app.now - app.started);
    let path_room = room.saturating_sub(width(&time) + 3);
    let second = match app.projects.as_os_str().is_empty() {
        false if path_room >= LOADING_PATH_MIN => {
            let path = report::shorten(&app.projects, &app.home);
            format!("{} · {time}", truncate(&path, path_room))
        }
        _ => time,
    };
    if body_h >= 2 && width(&second) <= room {
        shown.push(centered(vec![Span::styled(second, t.muted())], w));
    }
    if body_h < shown.len() {
        shown.truncate(body_h);
        return shown;
    }
    let mut lines = vec![Line::default(); (body_h - shown.len()) / 2];
    lines.extend(shown);
    lines
}

/// Una línea centrada en `w` columnas: el hueco sobrante, a medias delante.
fn centered(mut spans: Vec<Span<'static>>, w: usize) -> Line<'static> {
    let pad = w.saturating_sub(spans_width(&spans)) / 2;
    spans.insert(0, Span::raw(" ".repeat(pad)));
    Line::from(spans)
}

/// ` ✦ visual [propio] vía workflow · station              lleva 4 min · 38 herr.`
///
/// Cabecera de la ficha de ejecución. La estrella que parpadea en `claude` si
/// está en curso (en blanco si terminó, el hueco se guarda), el nombre (como en la lista: `claude-soft`
/// negrita en curso, `ink` terminada), la etiqueta de origen (`[propio]`,
/// `[claude]`, y `vía workflow` si procede) y el proyecto en gris, y a la derecha lo trabajado y las herramientas. Terminada, en vez de
/// lo trabajado, `terminó hace N min`. Estrecha, pierde `lleva …` (menos de 70
/// columnas) y la etiqueta (menos de 50).
fn run_header_line(app: &App, d: &RunDetail, t: &Theme, w: usize) -> Line<'static> {
    // los tokens, con el ancho de `lleva …`
    let tok = if w >= RUN_WORKED_MIN_W && d.tokens > 0 {
        format!(" · {} tok", report::tokens(d.tokens))
    } else {
        String::new()
    };
    let right = match d.ended {
        Some(at) => {
            format!("ended {}{tok}", report::relative(at, app.now))
        }
        None if w >= RUN_WORKED_MIN_W => {
            format!(
                "running {} · {}{tok}",
                report::worked(d.worked),
                tools(d.tools)
            )
        }
        None => tools(d.tools),
    };
    let right_w = width(&right) + 1;
    // el proyecto es la carpeta donde se lanzó la sesión; si no se sabe, el cwd
    let project_dir = d.launch_dir.as_ref().or(d.cwd.as_ref());
    let mut shown_project = project_dir.is_none();
    let mut spans = vec![Span::raw(" "), star_slot(d.ended.is_none(), app.now, t)];
    let mut room = w.saturating_sub(1 + STAR_W + right_w + 2);
    let name = truncate(&d.name, room);
    room = room.saturating_sub(width(&name));
    spans.push(Span::styled(name, state_name_style(d.ended.is_some(), t)));
    if w >= RUN_TAG_MIN_W {
        let tag = kind_tag(d.kind, room.saturating_sub(1));
        room -= width(&tag);
        if !tag.is_empty() {
            spans.push(Span::styled(tag, t.muted()));
        }
    }
    if let Some(dir) = project_dir {
        let project = short_project(&dir.to_string_lossy());
        if 3 + width(&project) <= room {
            spans.push(Span::styled(format!(" · {project}"), t.muted()));
            room -= 3 + width(&project);
            shown_project = true;
        }
    }
    // «ejecución i/n» cede antes que `lleva …` y que el proyecto: solo sale
    // con ancho para `lleva` y si sobra sitio tras el nombre y el proyecto
    let mut right = right;
    if let Some((i, n)) = app.run_nav() {
        let nav = format!("run {i}/{n} · ");
        if w >= RUN_WORKED_MIN_W && shown_project && width(&nav) <= room {
            right = format!("{nav}{right}");
        }
    }
    split_line(
        spans,
        vec![Span::styled(format!("{right} "), t.muted())],
        w,
        t,
    )
}

/// `   lanzada desde la carpeta temporal de Claude · proyecto tomado de la sesión · ~/tmp/x`
///
/// Aviso en gris, sangrado como las líneas de detalle, solo si la ejecución
/// se lanzó desde la carpeta temporal de Claude. Dice si el proyecto se
/// tomó de la sesión o no se pudo resolver; la ruta temporal va detrás,
/// recortada con `…`, si sobra sitio.
fn temp_notice(app: &App, d: &RunDetail, t: &Theme, w: usize) -> Option<Line<'static>> {
    let temp = d.temp_cwd.as_ref()?;
    let room = w.saturating_sub(4);
    let mut text = if d.cwd_resolved {
        "launched from Claude's temp folder · project taken from the session"
    } else {
        "launched from Claude's temp folder · project unresolved"
    }
    .to_string();
    text = truncate(&text, room);
    let left = room.saturating_sub(width(&text) + 3);
    if left >= NOTICE_PATH_MIN && !text.ends_with('…') {
        let path = report::shorten(temp, &app.home);
        text = format!("{text} · {}", truncate(&path, left));
    }
    Some(Line::from(Span::styled(format!("   {text}"), t.muted())))
}

/// `   cwd          ~/Offing/argi/station`
///
/// En gris, con el valor en la misma columna que `launched by`, solo si el
/// subagente trabaja en otra carpeta que la de lanzamiento (la del proyecto
/// de la cabecera). La ruta se recorta con `…`.
fn cwd_notice(app: &App, d: &RunDetail, t: &Theme, w: usize) -> Option<Line<'static>> {
    let (launch, cwd) = (d.launch_dir.as_ref()?, d.cwd.as_ref()?);
    if launch == cwd {
        return None;
    }
    let label = "   cwd          ";
    let room = w.saturating_sub(width(label) + 1);
    let path = truncate(&report::shorten(cwd, &app.home), room);
    Some(Line::from(Span::styled(
        format!("{label}{path}"),
        t.muted(),
    )))
}

/// El encargo entero, con sus saltos de línea, ajustado a `room` columnas.
fn run_task_lines(d: &RunDetail, room: usize) -> Vec<String> {
    let Some(full) = &d.task_full else {
        return Vec::new();
    };
    full.lines()
        .flat_map(|l| wrap(l, room))
        .map(|l| truncate(&l, room))
        .collect()
}

/// Las herramientas como `(nombre, cifra)` empaquetadas por líneas de `room`
/// columnas, separadas por ` · `.
fn run_tool_lines(d: &RunDetail, room: usize) -> Vec<Vec<(String, String)>> {
    let mut lines: Vec<Vec<(String, String)>> = Vec::new();
    let mut used = 0;
    for (name, n) in &d.tool_counts {
        let item = width(name) + 1 + n.to_string().len();
        match lines.last_mut() {
            Some(line) if used + 3 + item <= room => {
                line.push((name.clone(), n.to_string()));
                used += 3 + item;
            }
            _ => {
                lines.push(vec![(name.clone(), n.to_string())]);
                used = item;
            }
        }
    }
    lines
}

/// `   launched by  session 23dabecb · “primer prompt…”`
///
/// Quién lanzó la ejecución, en gris y sangrado como el aviso: la sesión (su
/// id corto y su primer prompt entre comillas, recortado), otra ejecución
/// (su nombre en `ink` y su etiqueta: `p` la abre) o una que ya no está.
fn launched_by_line(d: &RunDetail, t: &Theme, w: usize) -> Option<Line<'static>> {
    let label = "   launched by  ";
    let room = w.saturating_sub(width(label) + 1);
    let mut spans = vec![Span::styled(label, t.muted())];
    match d.launched_by.as_ref()? {
        LaunchedBy::Session { id, prompt } => {
            let short: String = id.chars().take(8).collect();
            let mut text = format!("session {short}");
            if let Some(p) = prompt.as_deref().map(one_line).filter(|p| !p.is_empty()) {
                // el prompt solo si caben las comillas y algo dentro
                let left = room.saturating_sub(width(&text) + 5);
                if left >= 4 {
                    text = format!("{text} · “{}”", truncate(&p, left));
                }
            }
            spans.push(Span::styled(truncate(&text, room), t.muted()));
        }
        LaunchedBy::Run(link) => {
            let name = truncate(&link.name, room);
            let tag = kind_tag(link.kind, room.saturating_sub(width(&name)));
            spans.push(Span::styled(name, t.text()));
            if !tag.is_empty() {
                spans.push(Span::styled(tag, t.muted()));
            }
        }
        LaunchedBy::GoneRun { .. } => {
            spans.push(Span::styled(
                truncate("a run no longer on disk", room),
                t.muted(),
            ));
        }
    }
    Some(Line::from(spans))
}

/// La barra de contexto de la ficha: `width` columnas, lo lleno en octavos
/// (`claude-soft`, ámbar desde `CTX_WARN`) y lo vacío en `░` (`subtle`).
fn context_gauge(c: Context, width: usize, t: &Theme) -> Vec<Span<'static>> {
    let pct = c.percent();
    let bar = hbar(pct.min(100) as f64, 100.0, width);
    let rest = width.saturating_sub(self::width(&bar));
    vec![
        Span::styled(bar, if pct >= CTX_WARN { t.warn() } else { t.soft() }),
        Span::styled("░".repeat(rest), t.line()),
    ]
}

/// ` usage ──────────── sonnet-5-5` y sus filas:
///
/// `   context       71%  ████████▍░░░░  142k of 200k`, y entrada, salida,
/// lectura y escritura de caché con la cifra en corto, alineada a la derecha
/// en `ink`. Sin coste en dólares: se va por suscripción y la cifra a precio
/// de API no es lo que se paga. Vacío si no hay nada que contar.
fn usage_lines(d: &RunDetail, t: &Theme, w: usize) -> Vec<Line<'static>> {
    if d.tokens == 0 && d.context.is_none() && d.usage.model.is_none() {
        return Vec::new();
    }
    let model = d
        .usage
        .model
        .as_deref()
        .map_or("—".to_string(), report::model);
    let tk = &d.usage.tokens;
    let ctx = d
        .context
        .map_or("—".to_string(), |c| format!("{}%", c.percent()));
    let rows = [
        ("input", report::tokens(tk.input)),
        ("output", report::tokens(tk.output)),
        ("cache read", report::tokens(tk.cache_read)),
        ("cache write", report::tokens(tk.cache_creation)),
    ];
    let value_w = rows
        .iter()
        .map(|(_, v)| width(v))
        .chain([width(&ctx)])
        .max()
        .unwrap_or(0);
    let row = |label: &str, value: &str| -> Vec<Span<'static>> {
        vec![
            Span::styled(
                format!("   {}", pad(label, USAGE_LABEL_W, false)),
                t.muted(),
            ),
            Span::styled(
                pad(value, value_w, true),
                if value == "—" { t.line() } else { t.text() },
            ),
        ]
    };
    let mut lines = vec![section_line(t, "usage", t.soft(), &model, w)];
    let mut first = row("context", &ctx);
    if d.context.is_some_and(|c| c.percent() >= CTX_WARN) {
        first[1] = Span::styled(first[1].content.to_string(), t.warn());
    }
    if let Some(c) = d.context {
        let of = format!("{} of {}", report::tokens(c.used), report::tokens(c.window));
        let room = w.saturating_sub(spans_width(&first) + 2 + 2 + width(&of) + 1);
        let gauge = room.min(USAGE_GAUGE_MAX);
        if gauge >= USAGE_GAUGE_MIN {
            first.push(Span::raw("  "));
            first.extend(context_gauge(c, gauge, t));
        }
        if spans_width(&first) + 2 + width(&of) < w {
            first.push(Span::styled(format!("  {of}"), t.muted()));
        }
    }
    lines.push(Line::from(first));
    lines.extend(
        rows.iter()
            .map(|(label, value)| Line::from(row(label, value))),
    );
    lines
}

/// Lo que va tras el nombre de una hija: `2 min · 9 tools · 64k tok` en
/// curso, `4 min ago · 21 tools · 210k tok` terminada (sin tokens si no hay).
fn launched_figures(app: &App, l: &RunLink) -> String {
    let when = match (l.running, l.ended) {
        (false, Some(at)) => report::relative(at, app.now),
        _ => report::worked(l.worked),
    };
    let tok = if l.tokens > 0 {
        format!(" · {} tok", report::tokens(l.tokens))
    } else {
        String::new()
    };
    format!("{when} · {}{tok}", tools(l.tools))
}

/// ` launched · 2 ───` y una línea por hija, como las filas de la lista:
///
/// `   1. ✦ Explore [claude]   2 min · 9 tools · 64k tok   Find where…`
///
/// Número (los dígitos la eligen), estrella si está en curso, nombre (en
/// curso `claude-soft` negrita, terminada `ink`) y etiqueta, las cifras en
/// gris y el encargo en `ink`, en columnas. La elegida lleva `❯` y se pinta
/// como el cursor de la lista. Como mucho `rows` hijas: una ventana que
/// sigue a la elegida.
fn launched_lines(
    app: &App,
    d: &RunDetail,
    t: &Theme,
    w: usize,
    rows: usize,
) -> Vec<Line<'static>> {
    let n = d.launched.len();
    let off = match app.launched_sel {
        Some(sel) if sel >= rows => (sel + 1 - rows).min(n.saturating_sub(rows)),
        _ => 0,
    };
    let info = if n > rows {
        format!("{}-{}/{n}", off + 1, (off + rows).min(n))
    } else {
        String::new()
    };
    let mut lines = vec![section_line(
        t,
        &format!("launched · {n}"),
        t.soft(),
        &info,
        w,
    )];
    let num_w = n.to_string().len();
    let lead = 3 + num_w + 2 + STAR_W;
    let shown: Vec<(usize, &RunLink)> =
        d.launched.iter().enumerate().skip(off).take(rows).collect();
    let label = |l: &RunLink| format!("{}{}", l.name, kind_tag(l.kind, usize::MAX));
    let name_w = shown
        .iter()
        .map(|(_, l)| width(&label(l)))
        .max()
        .unwrap_or(0);
    let figs: Vec<String> = shown
        .iter()
        .map(|(_, l)| launched_figures(app, l))
        .collect();
    let figs_w = figs.iter().map(|f| width(f)).max().unwrap_or(0);
    let avail = w.saturating_sub(lead + 1);
    let name_col = name_w.min(avail);
    let show_figs = name_col + 3 + figs_w <= avail;
    let task_room = avail.saturating_sub(name_col + 3 + figs_w + 3);
    for ((i, l), fig) in shown.into_iter().zip(&figs) {
        let selected = app.launched_sel == Some(i);
        let mut spans = vec![
            Span::styled(if selected { " ❯ " } else { "   " }, t.accent()),
            Span::styled(
                format!("{:>num_w$}. ", i + 1),
                if selected { t.soft() } else { t.muted() },
            ),
            star_slot(l.running, app.now, t),
        ];
        let name = truncate(&l.name, name_col);
        let tag = kind_tag(l.kind, name_col.saturating_sub(width(&name)));
        let used = width(&name) + width(&tag);
        let style = if selected {
            t.soft_bold()
        } else {
            state_name_style(!l.running, t)
        };
        spans.push(Span::styled(name, style));
        if !tag.is_empty() {
            spans.push(Span::styled(tag, t.muted()));
        }
        if show_figs {
            spans.push(Span::raw(" ".repeat(name_col - used + 3)));
            spans.push(Span::styled(pad(fig, figs_w, false), t.muted()));
            if let Some(task) = l.task.as_deref().map(one_line).filter(|s| !s.is_empty()) {
                if task_room >= 8 {
                    spans.push(Span::raw("   "));
                    spans.push(Span::styled(truncate(&task, task_room), t.text()));
                }
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Cuerpo de la ficha de una ejecución: cabecera, quién la lanzó, el aviso
/// de carpeta temporal y las secciones encargo, uso, herramientas, lanzadas y
/// pasos (el más reciente primero). Reparte el alto `h`: los pasos se
/// reservan `RUN_STEPS_MIN` y se quedan lo que sobra; lo demás entra entero o
/// no entra, por este orden: quién la lanzó, el aviso, la línea en blanco,
/// herramientas, las dos primeras líneas del encargo, lanzadas, uso y el
/// resto del encargo. Nunca se pinta más de `h` líneas.
fn run_detail_lines(app: &mut App, t: &Theme, w: usize, h: usize) -> Vec<Line<'static>> {
    let room = w.saturating_sub(4);
    let Some(d) = app.run_detail.as_ref() else {
        return Vec::new();
    };
    let task = run_task_lines(d, room);
    let tool_lines = run_tool_lines(d, room);
    let usage = usage_lines(d, t, w);
    let (n_steps, total) = (d.steps.len(), d.tools.max(d.steps.len()));
    let from_cache = d.from_cache;
    let header = run_header_line(app, d, t, w);
    let mut lines = vec![header];
    // cabecera, regla de pasos y los pasos mínimos (o la nota del historial)
    let steps_min = if from_cache {
        1
    } else {
        n_steps.clamp(1, RUN_STEPS_MIN)
    };
    let mut avail = h.saturating_sub(2 + steps_min);
    // bajo la cabecera, antes que lo demás: quién la lanzó y el aviso
    for extra in [
        launched_by_line(d, t, w),
        temp_notice(app, d, t, w),
        cwd_notice(app, d, t, w),
    ]
    .into_iter()
    .flatten()
    {
        if avail >= 1 {
            lines.push(extra);
            avail -= 1;
        }
    }
    if h >= RUN_SPACER_MIN_H && avail >= 1 {
        lines.push(Line::default());
        avail -= 1;
    }
    // herramientas primero: si no caben enteras, se quitan antes que el encargo
    let tools_h = tool_lines.len().min(RUN_TOOLS_MAX);
    let show_tools = tools_h > 0 && avail > tools_h;
    if show_tools {
        avail -= 1 + tools_h;
    }
    let show_task = avail >= 2;
    if show_task {
        avail -= 2;
    }
    let launched_h = d.launched.len().min(RUN_LAUNCHED_MAX);
    let show_launched = launched_h > 0 && avail > launched_h;
    if show_launched {
        avail -= 1 + launched_h;
    }
    let show_usage = !usage.is_empty() && avail >= usage.len();
    if show_usage {
        avail -= usage.len();
    }
    let want = task.len().clamp(1, RUN_TASK_MAX);
    let task_h = want.min(1 + avail);
    if show_task {
        lines.push(section_line(t, "task", t.soft(), "", w));
        if task.is_empty() {
            lines.push(Line::from(Span::styled("   —", t.muted())));
        }
        let cut = task.len() > task_h;
        for (i, l) in task.iter().take(task_h).enumerate() {
            let l = if cut && i + 1 == task_h {
                truncate(&format!("{l}…"), room)
            } else {
                l.clone()
            };
            lines.push(Line::from(Span::styled(format!("   {l}"), t.text())));
        }
    }
    if show_usage {
        lines.extend(usage);
    }
    if show_tools {
        lines.push(section_line(t, "tools", t.soft(), "", w));
        for (i, line) in tool_lines.iter().take(tools_h).enumerate() {
            let mut spans = vec![Span::raw("   ")];
            for (j, (name, n)) in line.iter().enumerate() {
                if j > 0 {
                    spans.push(Span::styled(" · ", t.muted()));
                }
                spans.push(Span::styled(name.clone(), t.text()));
                spans.push(Span::styled(format!(" {n}"), t.soft()));
            }
            if i + 1 == tools_h && tool_lines.len() > tools_h {
                spans.push(Span::styled(" …", t.muted()));
            }
            lines.push(Line::from(spans));
        }
    }
    if show_launched {
        lines.extend(launched_lines(app, d, t, w, launched_h));
    }
    if from_cache {
        if lines.len() < h {
            lines.push(section_line(t, "steps", t.soft(), "", w));
        }
        lines.push(Line::from(Span::styled(
            format!(
                "   {}",
                truncate(
                    "from the history cache · steps are not kept",
                    w.saturating_sub(4)
                )
            ),
            t.muted(),
        )));
        return lines;
    }
    // pasos: lo que queda bajo la regla
    let step_room = h.saturating_sub(lines.len() + 1);
    let off = app.steps_window(step_room);
    let Some(d) = app.run_detail.as_ref() else {
        return lines;
    };
    let title = if n_steps < total {
        format!("steps · last {n_steps} of {total}")
    } else {
        format!("steps · {total}")
    };
    let info = if step_room > 0 && n_steps > step_room {
        format!("{}-{}/{}", off + 1, (off + step_room).min(n_steps), n_steps)
    } else {
        String::new()
    };
    lines.push(section_line(t, &title, t.soft(), &info, w));
    if n_steps == 0 {
        lines.push(Line::from(Span::styled("   —", t.muted())));
        return lines;
    }
    let shown: Vec<_> = d.steps.iter().rev().skip(off).take(step_room).collect();
    let whens: Vec<String> = shown
        .iter()
        .map(|s| report::relative(s.at, app.now))
        .collect();
    let when_w = whens.iter().map(|s| width(s)).max().unwrap_or(0);
    let label_room = w.saturating_sub(3 + when_w + 2 + 1);
    for (i, (s, when)) in shown.iter().zip(&whens).enumerate() {
        // solo el paso más reciente va en `ink`, y mientras se mira desde arriba
        let (when_style, label_style) = if off == 0 && i == 0 {
            (t.muted(), t.text())
        } else {
            (t.muted(), t.muted())
        };
        lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled(pad(when, when_w, false), when_style),
            Span::raw("  "),
            Span::styled(truncate(&s.label(), label_room), label_style),
        ]));
    }
    lines
}

/// Pie de la ficha de ejecución: `a agente` solo si la ficha del agente se
/// puede abrir, `←→ otra ejecución` solo si hay varias, `tab launched` si
/// lanzó alguna, `enter open` con una elegida, `p parent` si la lanzó otra
/// ejecución que sigue en disco y `pgup/pgdn` si hay pasos; si no cabe, se
/// quitan en el orden de `RUN_MINOR_KEYS`.
fn run_footer(app: &App, t: &Theme, w: usize) -> Line<'static> {
    let d = app.run_detail.as_ref();
    let launched = d.is_some_and(|d| !d.launched.is_empty());
    let steps = d.is_some_and(|d| !d.from_cache);
    let mut keys: Vec<(&str, &str)> = RUN_KEYS
        .iter()
        .copied()
        .filter(|(k, _)| match *k {
            "a" => app.can_open_agent(),
            "←→" => app.run_nav().is_some(),
            "tab" => launched,
            // sin elegida, enter vuelve como esc: no se anuncia dos veces
            "enter" => app.launched_sel.is_some(),
            "p" => app.can_open_parent(),
            "pgup/pgdn" => steps,
            _ => true,
        })
        .collect();
    for minor in RUN_MINOR_KEYS {
        if spans_width(&keys_spans(app, t, &keys, false, false)) < w {
            break;
        }
        keys.retain(|(k, _)| k != minor);
    }
    footer_with_version(app, t, w, keys_spans(app, t, &keys, false, false), None)
}

/// Barra vertical a la derecha del cuerpo, como la de moon: carril `│` en
/// `subtle` y pulgar `█` en `claude-soft`, sin flechas en los extremos.
fn render_scrollbar(
    frame: &mut Frame,
    t: &Theme,
    body: Rect,
    total: usize,
    rows: usize,
    scroll: usize,
) {
    if total <= rows || rows == 0 || body.height == 0 {
        return;
    }
    let mut state = ScrollbarState::new(total - rows + 1)
        .position(scroll)
        .viewport_content_length(rows);
    let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(Some("│"))
        .thumb_symbol("█")
        .track_style(t.line())
        .thumb_style(t.soft());
    frame.render_stateful_widget(bar, body, &mut state);
}

/// `  activos   todos   ranking                    ventana 60 min · …`
///
/// Las pestañas (la abierta sobre `claude`) y a la derecha lo que
/// hay que contar: en las listas, ventana y recuentos; en el ranking, el
/// periodo y su total. Si no cabe, se acorta por el final; las pestañas no se
/// pierden nunca.
///
/// Devuelve también dónde ha caído cada pestaña en `area` (su fila de arriba),
/// para los clics, recortada al ancho si la terminal no llega.
fn top_line(app: &App, t: &Theme, area: Rect) -> (Line<'static>, Vec<TabHit>) {
    let w = area.width as usize;
    let mut used = 0;
    let mut spans = vec![];
    let mut tabs = vec![];
    for (label, mode) in [
        ("active", Mode::Active),
        ("all", Mode::All),
        ("ranking", Mode::Ranking),
    ] {
        let chip = format!(" {label} ");
        // el margen de la izquierda y, entre pestañas, un hueco
        let start = (used + 1).min(w);
        used += 1 + width(&chip);
        let end = used.min(w);
        if start < end {
            tabs.push(TabHit {
                mode,
                row: area.y,
                cols: area.x + start as u16..area.x + end as u16,
            });
        }
        spans.push(Span::styled(" ", t.text()));
        spans.push(Span::styled(
            chip,
            if app.mode == mode {
                t.selected()
            } else {
                t.muted()
            },
        ));
    }
    let candidates = if app.mode == Mode::Ranking {
        let total = app.ranking_total();
        let period = report::period(app.period);
        let worked = report::worked(total.worked);
        let runs = plural(total.runs, "run", "runs");
        if total.runs == 0 {
            vec![period.to_string()]
        } else {
            let tok = match total.tokens {
                0 => String::new(),
                n => format!(" · {} tok", report::tokens(n)),
            };
            vec![
                format!("{period} · {worked} in {runs}{tok}"),
                format!("{period} · {worked} in {runs}"),
                format!("{period} · {worked}"),
                period.to_string(),
            ]
        }
    } else if app.mode == Mode::Active {
        let window = format!("window {} min", app.minutes);
        let live = format!("{} running", app.live_count());
        let events = format!("{} in window", app.recent_count());
        let sessions = plural(app.sessions, "session", "sessions");
        // los tokens ceden después de las sesiones y antes que lo de la ventana
        let tok = match app.live_tokens() {
            0 => String::new(),
            n => format!(" · {} tok", report::tokens(n)),
        };
        vec![
            format!("{window} · {live} · {events} · {sessions}{tok}"),
            format!("{window} · {live} · {events}{tok}"),
            format!("{window} · {live} · {events}"),
            format!("{window} · {live}"),
            window,
        ]
    } else {
        // «todos» no habla de activos: lo de la ventana ya lo dice el
        // último uso de cada fila; lo en curso, solo si hay algo, y solo de
        // las filas de la lista (las que llevan ✦), como las reglas
        let window = format!("window {} min", app.minutes);
        let agents = plural(app.agent_count(), "agent", "agents");
        let running = app
            .items()
            .iter()
            .filter(|i| matches!(i, Item::Row(r) if r.running()))
            .count();
        let live = match running {
            0 => String::new(),
            n => format!(" · {n} running"),
        };
        let sessions = plural(app.sessions, "session", "sessions");
        let mut v = vec![format!("{window} · {agents}{live} · {sessions}")];
        if !live.is_empty() {
            v.push(format!("{window} · {agents}{live}"));
        }
        v.push(format!("{window} · {agents}"));
        v.push(window);
        v
    };
    if let Some(info) = candidates.into_iter().find(|i| used + width(i) + 2 <= w) {
        let fill = w - (used + width(&info) + 1);
        spans.push(Span::styled(" ".repeat(fill), t.text()));
        spans.push(Span::styled(format!("{info} "), t.muted()));
    }
    (Line::from(spans), tabs)
}

/// Segunda línea: el filtro mientras se escribe o está puesto, o el error del
/// hilo de lectura; a la derecha, el orden de la lista si no es el de siempre.
fn second_line(app: &App, t: &Theme, w: usize) -> Line<'static> {
    let left = if app.filtering || !app.filter.is_empty() {
        let mut spans = vec![
            Span::styled(" filter ", t.muted()),
            Span::styled(truncate(&app.filter, w.saturating_sub(10)), t.text()),
        ];
        if app.filtering {
            spans.push(Span::styled("█", t.accent()));
        } else {
            spans.push(Span::styled(" · / to edit", t.muted()));
        }
        spans
    } else if let Some(e) = &app.error {
        vec![
            Span::styled(" ✗ ", t.alert()),
            Span::styled(truncate(e, w.saturating_sub(4)), t.muted()),
        ]
    } else {
        Vec::new()
    };
    if app.mode == Mode::All && app.order == Order::Worked {
        let right = vec![
            Span::styled("sort: ", t.muted()),
            Span::styled("worked ", t.text()),
        ];
        return split_line(left, right, w, t);
    }
    Line::from(left)
}

/// Lo que va a la derecha de una fila: el total trabajado y las ejecuciones
/// (vacíos sin ejecuciones) y la última vez (vacía si está en curso: ya lo
/// dice la estrella).
fn row_right(app: &App, row: &Row) -> (String, String, String) {
    let (worked, runs) = if row.work.runs == 0 {
        (String::new(), String::new())
    } else {
        (report::worked(row.work.total), row.work.runs.to_string())
    };
    (worked, runs, row_when(app, row))
}

fn row_when(app: &App, row: &Row) -> String {
    match (row.work.running, row.last) {
        (Some(_), _) => String::new(),
        (None, Some(at)) => report::relative(at, app.now),
        (None, None) => report::last_seen(row.ever, app.now),
    }
}

/// Nunca usado: ni llamadas ni ejecuciones.
fn never(row: &Row) -> bool {
    row.ever.is_none() && row.work.runs == 0 && !row.running()
}

/// Un texto de varias líneas (las descripciones del meta) en una sola, con
/// los espacios seguidos juntados en uno.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tools(n: usize) -> String {
    plural(n, "tool", "tools")
}

/// Proyecto corto del ranking: el último componente del grupo, o "global".
fn short_project(section: &str) -> String {
    if section == "global" {
        return section.to_string();
    }
    std::path::Path::new(section)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| section.to_string())
}

/// Título de un grupo de «todos»: solo la carpeta del proyecto, o
/// «padre/carpeta» si otro grupo de la lista tiene la misma carpeta. La ruta
/// entera sigue siendo la clave del grupo y lo que mira el filtro.
fn group_title(cols: &Columns, section: &str) -> String {
    let folder = short_project(section);
    if !cols.twins.contains(&folder) {
        return folder;
    }
    let path = std::path::Path::new(section);
    match path.parent().and_then(|p| p.file_name()) {
        Some(parent) => format!("{}/{folder}", parent.to_string_lossy()),
        None => folder,
    }
}

/// Anchos de la vista, sacados de todas sus líneas para que no bailen al
/// desplazarse, y qué columnas caben.
#[derive(Default)]
struct Columns {
    /// Cursor, número y punto.
    prefix: usize,
    /// Nombre y marca, ya recortado a lo que cabe.
    name: usize,
    /// Del nombre al bloque de datos, hueco incluido: lo que hace falta para
    /// que el bloque acabe en el borde, como el texto de las reglas.
    name_room: usize,
    spark: bool,
    runs: bool,
    worked_w: usize,
    runs_w: usize,
    when_w: usize,
    // activos: bloques en curso
    /// Ancho de la columna «tiempo» (con su cabecera): «lleva» en curso,
    /// «hace N min» en las terminadas.
    live_worked: usize,
    /// Anchos de «ctx» y «tokens»: salen y se van juntas (cero en estrecho).
    live_ctx: usize,
    live_tokens: usize,
    /// La etiqueta tras el nombre («[propio]», «[claude]», «vía workflow») cabe.
    live_tag: bool,
    // ranking
    project_w: usize,
    bar: usize,
    /// Ancho de las cifras de la derecha, con el margen de la regla.
    rank_right: usize,
    /// El máximo de la clave del ranking, para la barra.
    max: f64,
    /// Las columnas de cifras del ranking que caben, en orden.
    rank_cols: Vec<RankKey>,
    /// Ancho de la columna «tokens» del ranking.
    tokens_w: usize,
    /// todos: las cifras de cada grupo abierto, por su ruta.
    groups: std::collections::HashMap<String, GroupSum>,
    /// Carpetas que llevan dos o más grupos de la lista.
    twins: std::collections::HashSet<String>,
}

/// Lo que suma un grupo abierto de «todos» para su regla: ejecuciones,
/// cuántas filas están en curso, la última llamada de cualquiera y el trabajo
/// de cada día.
#[derive(Default)]
struct GroupSum {
    runs: usize,
    running: usize,
    last: Option<DateTime<Utc>>,
    days: Vec<TimeDelta>,
}

impl GroupSum {
    fn add(&mut self, row: &Row) {
        self.runs += row.work.runs;
        self.running += usize::from(row.running());
        self.last = self.last.max(row.last).max(row.ever);
        if self.days.len() < row.history.len() {
            self.days.resize(row.history.len(), TimeDelta::zero());
        }
        for (sum, d) in self.days.iter_mut().zip(&row.history) {
            *sum += *d;
        }
    }

    /// Ejecuciones (vacías si no hay) y último uso (vacío si hay algo en
    /// curso), como en las filas.
    fn right(&self, app: &App) -> (String, String) {
        let runs = if self.runs == 0 {
            String::new()
        } else {
            self.runs.to_string()
        };
        let when = if self.running > 0 {
            String::new()
        } else {
            report::last_seen(self.last, app.now)
        };
        (runs, when)
    }
}

fn spark_label() -> String {
    format!("last {DAYS} days")
}

/// Ancho de la columna de la barrita: la cabecera es una más ancha que sus
/// días; la barrita se alinea a la derecha, pegada al bloque de datos.
fn spark_w() -> usize {
    width(&spark_label()).max(DAYS)
}

impl Columns {
    /// `term_w` decide qué columnas salen; `w` es lo que hay para pintar.
    fn of(app: &App, items: &[Item], term_w: usize, w: usize) -> Self {
        let num_w = app.count().to_string().len().max(2);
        let mut c = Columns {
            prefix: 3 + num_w + 2,
            spark: term_w >= SPARK_MIN_W,
            runs: term_w >= RUNS_MIN_W,
            ..Default::default()
        };
        let ranking = app.mode == Mode::Ranking;
        c.worked_w = width("worked");
        c.runs_w = width("runs");
        if ranking {
            c.tokens_w = width("tokens");
        } else {
            c.when_w = width("last used");
        }
        let mut name = 0;
        let (mut live_ctx, mut live_tok) = (0, 0);
        let mut folders = std::collections::HashMap::<String, usize>::new();
        for item in items {
            // las reglas de lo en curso cuentan para desambiguar carpetas iguales
            if let Item::Section { title, .. }
            | Item::Folded { title, .. }
            | Item::LiveGroup { section: title, .. } = item
            {
                *folders.entry(short_project(title)).or_default() += 1;
            }
            match item {
                Item::Row(r) => {
                    c.groups.entry(r.section.clone()).or_default().add(r);
                    let (worked, runs, when) = row_right(app, r);
                    name = name.max(width(&app.agent(r.agent).name) + 2);
                    c.worked_w = c.worked_w.max(width(&worked));
                    c.runs_w = c.runs_w.max(width(&runs));
                    c.when_w = c.when_w.max(width(&when));
                }
                Item::Rank { row, work } => {
                    // con sitio para la estrella, como en «todos»
                    name = name.max(width(&app.agent(row.agent).name) + 2);
                    c.project_w = c.project_w.max(width(&short_project(&row.section)));
                    c.worked_w = c.worked_w.max(width(&rank_cell(work, RankKey::Worked)));
                    c.tokens_w = c.tokens_w.max(width(&rank_cell(work, RankKey::Tokens)));
                    c.runs_w = c.runs_w.max(width(&rank_cell(work, RankKey::Runs)));
                    c.max = c.max.max(rank_value(work, app.rank_key));
                }
                Item::Section { worked, .. } if !worked.is_zero() => {
                    c.worked_w = c.worked_w.max(width(&report::worked(*worked)));
                }
                Item::Live { line, .. } => {
                    c.live_worked = c.live_worked.max(width(&live_time(app, line)));
                    live_ctx = live_ctx.max(spans_width(&ctx_spans(line.context, t_plain())));
                    live_tok = live_tok.max(width(&live_tokens_cell(line)));
                }
                _ => {}
            }
        }
        // lo de la derecha del nombre y el margen de la última columna
        if !ranking {
            for g in c.groups.values() {
                let (runs, when) = g.right(app);
                c.runs_w = c.runs_w.max(width(&runs));
                c.when_w = c.when_w.max(width(&when));
            }
        }
        let rest = if ranking {
            c.project_w = c.project_w.min(RANK_PROJECT_MAX);
            // las cifras pegadas al borde, acabando donde el texto de la regla
            let left = c.prefix + name + NAME_GAP + c.project_slot();
            // la columna de la clave primero, pegada a la barra que mide
            c.rank_cols = vec![app.rank_key];
            c.rank_cols.extend(
                [RankKey::Worked, RankKey::Tokens, RankKey::Runs]
                    .into_iter()
                    .filter(|&k| k != app.rank_key),
            );
            // la barra se queda el hueco entero, menos NAME_GAP antes de las
            // cifras; antes de que baje de BAR_MIN se van «runs» y luego
            // «tokens» (nunca la de la clave), y si aun así no llega, se va ella
            for drop in [RankKey::Runs, RankKey::Tokens, RankKey::Worked] {
                let room = w.saturating_sub(left + c.rank_cols_w() + NAME_GAP);
                if room >= BAR_MIN {
                    break;
                }
                if drop != app.rank_key {
                    c.rank_cols.retain(|&k| k != drop);
                }
            }
            c.rank_right = c.rank_cols_w();
            // con el nombre al mínimo, el proyecto encoge (hasta irse) antes
            // de que las cifras se salgan del borde
            let project_room = w.saturating_sub(c.prefix + 6 + NAME_GAP + c.rank_right);
            if c.project_slot() > project_room {
                c.project_w = project_room.saturating_sub(NAME_GAP);
            }
            let left = c.prefix + name + NAME_GAP + c.project_slot();
            let room = w.saturating_sub(left + c.rank_right + NAME_GAP);
            if room >= BAR_MIN {
                c.bar = room;
            }
            let bar = if c.bar > 0 { c.bar + NAME_GAP } else { 0 };
            c.prefix + NAME_GAP + c.project_slot() + bar + c.rank_right
        } else {
            c.prefix + NAME_GAP + c.data_w() + 1
        };
        c.name = name.min(w.saturating_sub(rest)).max(6);
        c.twins = folders
            .into_iter()
            .filter(|&(_, n)| n > 1)
            .map(|(f, _)| f)
            .collect();
        if !ranking {
            // el bloque de datos pegado al borde; el hueco sobrante, tras el nombre
            let edge = w.saturating_sub(c.prefix + c.data_w() + 1);
            c.name_room = (c.name + NAME_GAP).max(edge);
        }
        // activos: «time» siempre, «ctx» y «tokens» desde 70 columnas y la
        // etiqueta del tipo desde 50; las cabeceras dan el ancho mínimo
        c.live_worked = c.live_worked.max(width("time"));
        if term_w >= LIVE_FIGURES_MIN_W {
            c.live_ctx = live_ctx.max(width("ctx"));
            c.live_tokens = live_tok.max(width("tokens"));
        }
        c.live_tag = term_w >= LIVE_TAG_MIN_W;
        c
    }

    /// Ancho de lo que va pegado al borde derecho en una fila de activos:
    /// «ctx», «time» y «tokens» (o solo «time» en estrecho) y el margen.
    fn live_right(&self) -> usize {
        let figures = if self.live_figures() {
            2 * COL_GAP + self.live_ctx + self.live_tokens
        } else {
            0
        };
        self.live_worked + figures + 1
    }

    /// Salen «ctx» y «tokens» (la terminal llega a `LIVE_FIGURES_MIN_W`).
    fn live_figures(&self) -> bool {
        self.live_ctx > 0
    }

    /// Lo que ocupa el proyecto del ranking con su hueco; nada si no cabe.
    fn project_slot(&self) -> usize {
        if self.project_w > 0 {
            self.project_w + NAME_GAP
        } else {
            0
        }
    }

    /// Ancho de una columna de cifras del ranking.
    fn rank_w(&self, key: RankKey) -> usize {
        match key {
            RankKey::Worked => self.worked_w,
            RankKey::Tokens => self.tokens_w,
            RankKey::Runs => self.runs_w,
        }
    }

    /// Ancho de las cifras del ranking que salen, con sus huecos y el margen.
    fn rank_cols_w(&self) -> usize {
        let cols: usize = self.rank_cols.iter().map(|&k| self.rank_w(k)).sum();
        cols + COL_GAP * self.rank_cols.len().saturating_sub(1) + 1
    }

    /// Ancho del bloque de datos de la lista, sin el hueco tras el nombre.
    fn data_w(&self) -> usize {
        let spark = if self.spark { spark_w() + SPARK_GAP } else { 0 };
        let runs = if self.runs { self.runs_w + COL_GAP } else { 0 };
        spark + self.worked_w + runs + COL_GAP + self.when_w
    }
}

/// Las líneas de un `Item`: una, salvo el monitor vacío, que lleva dos. Una
/// fila de activos en curso son dos `Item` (fila y `now`) o tres, con `skill`
/// entre ambos; la cabeza de un racimo, o la en curso sola, suma encima la
/// línea del agente. Una terminada, uno.
fn item_lines(app: &App, item: &Item, cols: &Columns, t: &Theme, w: usize) -> Vec<Line<'static>> {
    let line = match item {
        Item::Blank => Line::default(),
        Item::LiveGroup {
            section,
            running,
            recent,
        } => live_group_line(cols, section, *running, *recent, t, w),
        Item::Live { n, line, .. } => live_line(app, *n, line, cols, t, w),
        Item::LiveAgent(line) => live_agent_line(line, cols, t, w),
        Item::LiveSkill(line) => live_skill_line(line, cols, t, w),
        Item::LiveNow(line) => live_now_line(line, cols, t, w),
        Item::Quiet { last } => return quiet_lines(app, last.as_ref(), t, w),
        Item::Columns if app.mode == Mode::Active => live_columns_line(cols, t, w),
        Item::Columns => columns_line(cols, t),
        Item::Section {
            title,
            total,
            worked,
            ..
        } => group_line(app, title, *total, *worked, cols, t),
        Item::Row(row) => row_line(app, row, cols, t),
        Item::Folded {
            title,
            total,
            n,
            open,
        } => folded_line(
            app,
            &group_title(cols, title),
            *total,
            *n,
            *open,
            cols,
            t,
            w,
        ),
        Item::RankRule { agents } => rank_rule_line(app, *agents, cols, t, w),
        Item::Rank { row, work } => rank_line(app, row, work, cols, t, w),
    };
    vec![line]
}

/// ` station · 6 agentes · 1 en curso ──   ╸━━─  6 h 12 min      51            `
///
/// La regla de un grupo abierto: carpeta en `claude`, cuántos agentes en
/// `ink-muted` y, si alguno está en curso, cuántos en `claude`; la regla acaba
/// dos columnas antes del bloque de la derecha y, en las mismas columnas que
/// las filas, las cifras del grupo: la barrita (suma por día), el total en
/// `ink` negrita, las ejecuciones y el último uso.
fn group_line(
    app: &App,
    section: &str,
    total: usize,
    worked: TimeDelta,
    cols: &Columns,
    t: &Theme,
) -> Line<'static> {
    let block = cols.prefix + cols.name_room;
    let empty = GroupSum::default();
    let sum = cols.groups.get(section).unwrap_or(&empty);
    // el título cede antes que las cuentas; si no cabe ni la regla, se corta todo
    let count = format!(" · {}", plural(total, "agent", "agents"));
    let live = match sum.running {
        0 => String::new(),
        n => format!(" · {n} running"),
    };
    let room = block.saturating_sub(1 + 2 + 2);
    let title = truncate(
        &group_title(cols, section),
        room.saturating_sub(width(&count) + width(&live)).max(4),
    );
    let head = truncate(&format!("{title}{count}{live}"), room);
    let mut spans = vec![Span::raw(" ")];
    let (t_part, rest) = split_at_width(&head, width(&title));
    let (c_part, l_part) = split_at_width(&rest, width(&count));
    spans.push(Span::styled(t_part, t.accent()));
    spans.push(Span::styled(c_part, t.muted()));
    if !l_part.is_empty() {
        // el separador en muted, la cuenta en claude
        let (sep, n) = split_at_width(&l_part, width(" · "));
        spans.push(Span::styled(sep, t.muted()));
        spans.push(Span::styled(n, t.accent()));
    }
    let fill = block.saturating_sub(2 + spans_width(&spans) + 1);
    spans.push(Span::styled(format!(" {}", "─".repeat(fill)), t.line()));
    spans.push(Span::raw(
        " ".repeat(block.saturating_sub(spans_width(&spans))),
    ));
    if cols.spark {
        let mut days = sum.days.clone();
        days.resize(MAX_DAYS, TimeDelta::zero());
        let days = &days[MAX_DAYS - DAYS..];
        spans.push(Span::raw(" ".repeat(spark_w() - DAYS)));
        spans.extend(spark_spans(days, t));
        spans.push(Span::raw(" ".repeat(SPARK_GAP)));
    }
    let worked = if worked.is_zero() {
        String::new()
    } else {
        report::worked(worked)
    };
    spans.push(Span::styled(
        pad(&worked, cols.worked_w, true),
        t.text_bold(),
    ));
    let (runs, when) = sum.right(app);
    if cols.runs {
        spans.push(Span::styled(
            format!("{}{}", " ".repeat(COL_GAP), pad(&runs, cols.runs_w, true)),
            t.muted(),
        ));
    }
    spans.push(Span::raw(" ".repeat(COL_GAP)));
    spans.push(Span::styled(pad(&when, cols.when_w, true), t.muted()));
    Line::from(spans)
}

/// Parte `s` en las primeras `w` columnas y el resto.
fn split_at_width(s: &str, w: usize) -> (String, String) {
    let mut used = 0;
    for (i, ch) in s.char_indices() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw > w {
            return (s[..i].to_string(), s[i..].to_string());
        }
        used += cw;
    }
    (s.to_string(), String::new())
}

/// ` título ─────────── info `
pub(super) fn section_line(
    t: &Theme,
    title: &str,
    style: Style,
    info: &str,
    w: usize,
) -> Line<'static> {
    let title = format!(" {title}");
    let fill = w.saturating_sub(width(&title) + 1 + width(info) + 2);
    Line::from(vec![
        Span::styled(title, style),
        Span::styled(format!(" {} ", "─".repeat(fill)), t.line()),
        Span::styled(format!("{info} "), t.muted()),
    ])
}

/// Cursor y número de una línea seleccionable. `sober`: el número va en `ink`
/// aunque esté seleccionada (las terminadas no se tiñen).
fn prefix(
    app: &App,
    n: usize,
    cols: &Columns,
    t: &Theme,
    sober: bool,
) -> (bool, Vec<Span<'static>>) {
    let selected = n == app.cursor + 1;
    let num_w = cols.prefix - 5;
    (
        selected,
        vec![
            Span::styled(if selected { " ❯ " } else { "   " }, t.accent()),
            Span::styled(
                format!("{n:>num_w$}. "),
                if sober {
                    t.text()
                } else if selected {
                    t.soft()
                } else {
                    t.muted()
                },
            ),
        ],
    )
}

/// ` claude-agents · 2 en curso · 1 reciente ───────────────────`
///
/// La regla de un grupo de activos: la carpeta (con su padre si otra regla
/// lleva la misma) en `claude-soft`; en gris, `· N en curso` si hay en curso y
/// `· N recientes` si hay terminadas en la ventana.
fn live_group_line(
    cols: &Columns,
    section: &str,
    running: usize,
    recent: usize,
    t: &Theme,
    w: usize,
) -> Line<'static> {
    let mut count = String::new();
    if running > 0 {
        count += &format!(" · {running} running");
    }
    if recent > 0 {
        count += &format!(" · {recent} recent");
    }
    let room = w.saturating_sub(1 + 2 + 1);
    let title = truncate(
        &group_title(cols, section),
        room.saturating_sub(width(&count)).max(4),
    );
    let head = truncate(&format!("{title}{count}"), room);
    let (t_part, c_part) = split_at_width(&head, width(&title));
    let used = 1 + width(&head);
    let fill = w.saturating_sub(used + 2);
    Line::from(vec![
        Span::raw(" "),
        Span::styled(t_part, t.soft()),
        Span::styled(c_part, t.muted()),
        Span::styled(format!(" {}", "─".repeat(fill)), t.line()),
    ])
}

/// Lo que va en la columna «tiempo» de un bloque: lo trabajado si está en
/// curso, `hace N min` si terminó.
fn live_time(app: &App, r: &NowLine) -> String {
    match r.state {
        RunState::Running => report::worked(r.worked),
        RunState::Done { ended } => report::relative(ended, app.now),
    }
}

/// Desde este porcentaje de contexto, la celda va en ámbar.
const CTX_WARN: u64 = 85;
/// Pasos del medidor de contexto de activos.
const CTX_STEPS: usize = 8;

/// Una paleta cualquiera para medir anchos de spans sin pintar.
fn t_plain() -> &'static Theme {
    static PLAIN: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
    PLAIN.get_or_init(Theme::claude_256)
}

/// `▰▰▰▰▰▰▱▱ 71%`: el contexto de una ejecución en curso en ocho pasos (cada
/// uno, un octavo empezado) y el porcentaje. La barra en `claude`, lo
/// vacío en `subtle` y la cifra en gris; desde `CTX_WARN`, barra y cifra en
/// ámbar. Sin contexto, `—` apagado.
fn ctx_spans(ctx: Option<Context>, t: &Theme) -> Vec<Span<'static>> {
    let Some(c) = ctx else {
        return vec![Span::styled("—", t.line())];
    };
    let pct = c.percent();
    let full = ((pct as usize * CTX_STEPS).div_ceil(100)).min(CTX_STEPS);
    let warn = pct >= CTX_WARN;
    vec![
        Span::styled("▰".repeat(full), if warn { t.warn() } else { t.accent() }),
        Span::styled("▱".repeat(CTX_STEPS - full), t.line()),
        Span::styled(
            format!(" {pct:>2}%"),
            if warn { t.warn() } else { t.muted() },
        ),
    ]
}

/// La columna «tokens»: en corto, o `—` si no hay ninguno.
fn live_tokens_cell(r: &NowLine) -> String {
    if r.tokens == 0 {
        "—".to_string()
    } else {
        report::tokens(r.tokens)
    }
}

/// Hueco entre el borde de la izquierda del bloque y las columnas de la
/// derecha: lo que queda para el nombre y su etiqueta.
fn live_name_room(cols: &Columns, w: usize) -> usize {
    w.saturating_sub(cols.prefix + STAR_W + NAME_GAP + cols.live_right())
}

/// `         agent                                      ctx       time   tokens`
///
/// Cabecera de columnas de activos, una vez, en gris: «agent» sobre el
/// nombre y «ctx»/«time»/«tokens» alineadas a la derecha como sus cifras;
/// «tokens» la última, pegada al borde. En estrecho solo queda «time».
fn live_columns_line(cols: &Columns, t: &Theme, w: usize) -> Line<'static> {
    let name_room = live_name_room(cols, w);
    let mut s = " ".repeat(cols.prefix + STAR_W);
    s += &pad("agent", name_room + NAME_GAP, false);
    if cols.live_figures() {
        s += &pad("ctx", cols.live_ctx, true);
        s += &" ".repeat(COL_GAP);
        s += &pad("time", cols.live_worked, true);
        s += &" ".repeat(COL_GAP);
        s += &pad("tokens", cols.live_tokens, true);
    } else {
        s += &pad("time", cols.live_worked, true);
    }
    Line::from(Span::styled(truncate(&s, w), t.muted()))
}

/// ` ❯  1. ✦ docs [own] ×3  Revisando convenciones   ▰▰▰▱▱▱▱▱ 41%   1 min   140k`
///
/// Una fila de activos: número, el hueco de la estrella (`✦`/`✧` que
/// parpadea en `claude` si está en curso; en blanco si terminó, siempre
/// reservado), lo de la izquierda (`live_left`) y, a la derecha, pegadas al
/// borde, «ctx», «time» y, la última, «tokens» (en estrecho, solo «time»).
/// Las cifras en gris; terminada, `hace N min` en vez de lo trabajado.
fn live_line(
    app: &App,
    n: usize,
    r: &NowLine,
    cols: &Columns,
    t: &Theme,
    w: usize,
) -> Line<'static> {
    let done = matches!(r.state, RunState::Done { .. });
    let (_, mut spans) = prefix(app, n, cols, t, false);
    spans.push(star_slot(!done, app.now, t));
    let room = live_name_room(cols, w);
    let left = live_left(r, room, cols.live_tag, done, t);
    let used = spans_width(&left);
    spans.extend(left);
    spans.push(Span::raw(" ".repeat(room + NAME_GAP - used)));
    // las cifras, en gris en curso y terminadas; solo el medidor lleva color
    let time = Span::styled(pad(&live_time(app, r), cols.live_worked, true), t.muted());
    if cols.live_figures() {
        let ctx = ctx_spans(r.context, t);
        spans.push(Span::raw(" ".repeat(cols.live_ctx - spans_width(&ctx))));
        spans.extend(ctx);
        spans.push(Span::raw(" ".repeat(COL_GAP)));
        spans.push(time);
        spans.push(Span::raw(" ".repeat(COL_GAP)));
        // «tokens», la última, pegada al borde
        spans.push(Span::styled(
            pad(&live_tokens_cell(r), cols.live_tokens, true),
            if r.tokens == 0 { t.line() } else { t.muted() },
        ));
    } else {
        spans.push(time);
    }
    Line::from(spans)
}

/// Lo de la izquierda de una fila de activos, en `room` columnas como mucho.
///
/// - Terminada: el nombre con su etiqueta (`live_name`), dos espacios y la
///   tarea en gris (`—` si no hay).
/// - En curso, en un racimo o sola (un racimo de uno): `├` (`└` la última o
///   la sola) en `subtle`, en la columna del nombre, un espacio y la tarea en
///   `ink`; el nombre va en su propia línea encima de la cabeza
///   (`live_agent_line`).
///
/// Lo primero que se recorta es la tarea; luego la etiqueta y por último el
/// nombre.
fn live_left(r: &NowLine, room: usize, tag: bool, done: bool, t: &Theme) -> Vec<Span<'static>> {
    let task = r
        .task
        .as_deref()
        .map(one_line)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".to_string());
    let task_style = if done { t.muted() } else { t.text() };
    let (mut spans, gap) = match r.chain {
        Chain::Head(_) | Chain::Middle | Chain::Last => {
            let glyph = if r.chain == Chain::Last { "└" } else { "├" };
            (vec![Span::styled(truncate(glyph, room), t.line())], 1)
        }
        Chain::Single if !done => (vec![Span::styled(truncate("└", room), t.line())], 1),
        Chain::Single => (live_name(r, room, tag, done, t), TASK_GAP),
    };
    // la tarea, con lo que quede; sin sitio ni para dos columnas, fuera
    let task_room = room.saturating_sub(spans_width(&spans) + gap);
    if task_room >= width(&task).min(2) && task_room > 0 {
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(truncate(&task, task_room), task_style));
    }
    spans
}

/// `docs [own] ×3`: el nombre (`claude-soft` negrita en curso, gris
/// terminada), su etiqueta de origen en gris si `tag` (desde 50 columnas) y
/// ` ×N` en `subtle` (N el del racimo en la cabeza; ninguno en una en curso
/// sola; en una terminada, las ejecuciones fundidas si son más de una), en
/// `room` columnas como mucho:
/// la etiqueta cede antes que el nombre (`live_tag`).
fn live_name(r: &NowLine, room: usize, tag: bool, done: bool, t: &Theme) -> Vec<Span<'static>> {
    let name = truncate(&r.name, room);
    let used = width(&name);
    let style = if done { t.muted() } else { t.soft_bold() };
    let mut spans = vec![Span::styled(name, style)];
    if tag {
        let count = match r.chain {
            Chain::Head(n) => n,
            // una en curso sola es un racimo de uno: sin «×1»
            Chain::Single if !done => 1,
            _ => r.runs,
        };
        let tag = live_tag(r.kind, count, room - used);
        // el ×N, más apagado que el origen
        let (origin, times) = tag.split_at(tag.find(" ×").unwrap_or(tag.len()));
        if !origin.is_empty() {
            spans.push(Span::styled(origin.to_string(), t.muted()));
        }
        if !times.is_empty() {
            spans.push(Span::styled(times.to_string(), t.line()));
        }
    }
    spans
}

/// `         docs [own] ×3`
///
/// La línea del agente de un racimo en curso, encima de su cabeza y en la
/// columna del nombre: nombre, etiqueta y `×N` (`live_name`; sin `×N` si va
/// sola, un racimo de uno), sin número, estrella ni cifras.
fn live_agent_line(r: &NowLine, cols: &Columns, t: &Theme, w: usize) -> Line<'static> {
    let indent = cols.prefix + STAR_W;
    let room = w.saturating_sub(indent + 1);
    let mut spans = vec![Span::raw(" ".repeat(indent))];
    spans.extend(live_name(r, room, cols.live_tag, false, t));
    Line::from(spans)
}

/// `         │ skill  superpowers:brainstorming`
///
/// Una sublínea de una fila en curso, en la columna del nombre: la guía del
/// racimo (`│ ` en `subtle` si el racimo sigue debajo, en `Head` y `Middle`;
/// dos espacios en `Single` y `Last`), la etiqueta en `subtle` con ancho fijo
/// y el valor en gris, recortado con `…` a una sola línea. Sin valor, `—`.
fn live_sub_line(
    label: &str,
    value: Option<&str>,
    r: &NowLine,
    cols: &Columns,
    t: &Theme,
    w: usize,
) -> Line<'static> {
    let indent = cols.prefix + STAR_W;
    let guide = match r.chain {
        Chain::Head(_) | Chain::Middle => "│ ",
        Chain::Single | Chain::Last => "  ",
    };
    let room = w.saturating_sub(indent + LIVE_GUIDE_W + LIVE_LABEL_W + 1);
    let value = value
        .map(one_line)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "—".to_string());
    Line::from(vec![
        Span::raw(" ".repeat(indent)),
        Span::styled(guide, t.line()),
        Span::styled(pad(label, LIVE_LABEL_W, false), t.line()),
        Span::styled(truncate(&value, room), t.muted()),
    ])
}

/// `         │ skill  superpowers:brainstorming`
fn live_skill_line(r: &NowLine, cols: &Columns, t: &Theme, w: usize) -> Line<'static> {
    live_sub_line("skill", r.skill.as_deref(), r, cols, t, w)
}

/// `           now    Bash · git diff --stat`: la última herramienta de una
/// fila en curso (las terminadas no llevan sublíneas).
fn live_now_line(r: &NowLine, cols: &Columns, t: &Theme, w: usize) -> Line<'static> {
    live_sub_line("now", r.last_tool.as_deref(), r, cols, t, w)
}

/// El monitor sin nada en curso ni en la ventana: lo dice y, debajo, cuándo
/// se usó por última vez un agente y que tab lleva a todos.
fn quiet_lines(app: &App, last: Option<&LastUse>, t: &Theme, w: usize) -> Vec<Line<'static>> {
    let hint = match last {
        Some(l) => format!(
            "last use: {} from {} {}",
            app.agent(l.agent).name,
            short_project(&l.section),
            report::relative(l.at, app.now)
        ),
        None => "no use recorded".to_string(),
    };
    let room = w.saturating_sub(2);
    vec![
        Line::from(Span::styled(
            truncate(
                &format!(" nothing in the last {} min", app.minutes),
                room + 1,
            ),
            t.text(),
        )),
        Line::from(Span::styled(
            format!(" {}", truncate(&format!("{hint} · tab to see all"), room)),
            t.muted(),
        )),
    ]
}

/// La cabecera de las columnas, alineada con los datos de las filas.
fn columns_line(cols: &Columns, t: &Theme) -> Line<'static> {
    let mut s = " ".repeat(cols.prefix + cols.name_room);
    if cols.spark {
        s += &pad(&spark_label(), spark_w(), true);
        s += &" ".repeat(SPARK_GAP);
    }
    s += &pad("worked", cols.worked_w, true);
    if cols.runs {
        s += &" ".repeat(COL_GAP);
        s += &pad("runs", cols.runs_w, true);
    }
    s += &" ".repeat(COL_GAP);
    s += &pad("last used", cols.when_w, true);
    Line::from(Span::styled(s, t.muted()))
}

/// ` ❯  1. backend ✦          ▁▁▂▁▄█      48 min      5              `
///
/// Nombre y marca (✦/✧ si está en curso; nada si no, con su hueco guardado
/// para que no bailen las columnas) a la izquierda; a la
/// derecha, pegados al borde como el texto de las reglas, la barrita de los
/// últimos días, trabajado, ejecuciones y último uso (vacío si está en curso),
/// alineados a la derecha bajo su cabecera. El hueco sobrante queda entre el
/// nombre y la barrita.
fn row_line(app: &App, row: &Row, cols: &Columns, t: &Theme) -> Line<'static> {
    let (selected, mut spans) = prefix(app, row.n, cols, t, false);
    let never = never(row);
    let running = row.running();
    let mark = if running {
        star_mark(app, t)
    } else {
        Span::raw("")
    };
    let name = truncate(
        &app.agent(row.agent).name,
        cols.name.saturating_sub(mark.width()),
    );
    let name_style = if selected {
        t.soft_bold()
    } else if never {
        t.muted()
    } else {
        t.text()
    };
    let used = width(&name) + mark.width();
    spans.push(Span::styled(name, name_style));
    spans.push(mark);
    // con la barrita alineada a la derecha de su columna
    let lead = if cols.spark { spark_w() - DAYS } else { 0 };
    spans.push(Span::raw(
        " ".repeat(cols.name_room.saturating_sub(used) + lead),
    ));
    if cols.spark {
        spans.extend(spark_spans(row.days(DAYS), t));
        spans.push(Span::raw(" ".repeat(SPARK_GAP)));
    }
    let (worked, runs, when) = row_right(app, row);
    spans.push(Span::styled(pad(&worked, cols.worked_w, true), t.text()));
    if cols.runs {
        spans.push(Span::styled(
            format!("{}{}", " ".repeat(COL_GAP), pad(&runs, cols.runs_w, true)),
            t.muted(),
        ));
    }
    spans.push(Span::raw(" ".repeat(COL_GAP)));
    spans.push(Span::styled(pad(&when, cols.when_w, true), t.muted()));
    Line::from(spans)
}

/// `    6. ▸ _argibox · 4 agentes sin uso ─────────────────────────────────`
///
/// Un grupo en el que nadie ha usado nunca a nadie: seleccionable, en
/// `ink-muted`, con ▸ plegado y ▾ abierto, y la regla hasta el borde, donde
/// acaba todo lo demás, sin nada en las columnas.
#[allow(clippy::too_many_arguments)]
fn folded_line(
    app: &App,
    title: &str,
    total: usize,
    n: usize,
    open: bool,
    cols: &Columns,
    t: &Theme,
    w: usize,
) -> Line<'static> {
    let (selected, mut spans) = prefix(app, n, cols, t, false);
    let head = format!("{} {title}", if open { "▾" } else { "▸" });
    let info = format!(" · {} unused", plural(total, "agent", "agents"));
    let room = w.saturating_sub(cols.prefix + 1 + 2);
    let head = truncate(&head, room.saturating_sub(width(&info)).max(4));
    let info = truncate(&info, room.saturating_sub(width(&head)));
    spans.push(Span::styled(
        head,
        if selected { t.soft_bold() } else { t.muted() },
    ));
    spans.push(Span::styled(info, t.muted()));
    let fill = w.saturating_sub(spans_width(&spans) + 1 + 1);
    spans.push(Span::styled(format!(" {}", "─".repeat(fill)), t.line()));
    Line::from(spans)
}

/// ` ❯  1. kiosk ✦  station  ━━━━━━━━━━━━╸   1 h 9 min`
///
/// Nombre (con ✦/✧ si está en curso) y proyecto a la izquierda; el total
/// trabajado a la derecha, acabando donde el texto de la regla; la barra
/// llena el hueco entre los dos.
fn rank_line(
    app: &App,
    row: &Row,
    work: &crate::status::PeriodStats,
    cols: &Columns,
    t: &Theme,
    w: usize,
) -> Line<'static> {
    let (selected, mut spans) = prefix(app, row.n, cols, t, false);
    let mark = if row.running() {
        star_mark(app, t)
    } else {
        Span::raw("")
    };
    let name = truncate(
        &app.agent(row.agent).name,
        cols.name.saturating_sub(mark.width()),
    );
    let used = width(&name) + mark.width();
    spans.push(Span::styled(
        name,
        if selected { t.soft_bold() } else { t.text() },
    ));
    spans.push(mark);
    spans.push(Span::raw(
        " ".repeat(cols.name.saturating_sub(used) + NAME_GAP),
    ));
    if cols.project_w > 0 {
        let project = truncate(&short_project(&row.section), cols.project_w);
        spans.push(Span::styled(
            pad(&project, cols.project_slot(), false),
            t.muted(),
        ));
    }
    if cols.bar > 0 {
        let bar = rank_bar(rank_value(work, app.rank_key), cols.max, cols.bar);
        spans.push(Span::styled(
            pad(&bar, cols.bar + NAME_GAP, false),
            t.soft(),
        ));
    }
    // sin barra, el hueco se rellena para que el total siga en el borde
    let fill = w.saturating_sub(spans_width(&spans) + cols.rank_right);
    if fill > 0 {
        spans.push(Span::raw(" ".repeat(fill)));
    }
    for (i, &key) in cols.rank_cols.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" ".repeat(COL_GAP)));
        }
        let cell = rank_cell(work, key);
        let style = if cell == "—" {
            t.line()
        } else if key == app.rank_key {
            t.text()
        } else {
            t.muted()
        };
        spans.push(Span::styled(pad(&cell, cols.rank_w(key), true), style));
    }
    Line::from(spans)
}

/// La cifra de una columna del ranking: lo trabajado, los tokens en corto
/// (`—` si no hay) o las ejecuciones.
fn rank_cell(work: &PeriodStats, key: RankKey) -> String {
    match key {
        RankKey::Worked => report::worked(work.worked),
        RankKey::Tokens if work.tokens == 0 => "—".to_string(),
        RankKey::Tokens => report::tokens(work.tokens),
        RankKey::Runs => work.runs.to_string(),
    }
}

/// Lo que mide la barra del ranking según la clave.
fn rank_value(work: &PeriodStats, key: RankKey) -> f64 {
    match key {
        RankKey::Worked => work.worked.num_seconds().max(0) as f64,
        RankKey::Tokens => work.tokens as f64,
        RankKey::Runs => work.runs as f64,
    }
}

/// Rótulo de una columna del ranking (cabecera y tecla `s`).
fn rank_label(key: RankKey) -> &'static str {
    match key {
        RankKey::Worked => "worked",
        RankKey::Tokens => "tokens",
        RankKey::Runs => "runs",
    }
}

/// ` By tokens · 16 agents with work ──────────   tokens   worked   runs`
///
/// La regla del ranking: por qué se ordena en `claude`, cuántos agentes en
/// gris y, en las columnas de las cifras, sus cabeceras (la de la clave en
/// `claude-soft`). La raya llega hasta la primera cabecera.
fn rank_rule_line(app: &App, agents: usize, cols: &Columns, t: &Theme, w: usize) -> Line<'static> {
    let title = match app.rank_key {
        RankKey::Worked => "By time worked",
        RankKey::Tokens => "By tokens",
        RankKey::Runs => "By runs",
    };
    let count = format!(
        " · {}",
        plural(agents, "agent with work", "agents with work")
    );
    let mut heads: Vec<Span<'static>> = Vec::new();
    for (i, &key) in cols.rank_cols.iter().enumerate() {
        let label = rank_label(key);
        let lead = cols.rank_w(key).saturating_sub(width(label));
        // el hueco delante de la primera es raya
        let lead = if i == 0 { 0 } else { COL_GAP + lead };
        heads.push(Span::raw(" ".repeat(lead)));
        heads.push(Span::styled(
            label,
            if key == app.rank_key {
                t.soft()
            } else {
                t.muted()
            },
        ));
    }
    let heads_w = spans_width(&heads);
    // un espacio, la raya (una como poco) y otro antes de las cabeceras
    let room = w.saturating_sub(1 + 3 + heads_w + 1);
    let head = truncate(&format!("{title}{count}"), room);
    let (t_part, c_part) = split_at_width(&head, width(title));
    let fill = w.saturating_sub(1 + width(&head) + 2 + heads_w + 1);
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(t_part, t.accent()),
        Span::styled(c_part, t.muted()),
        Span::styled(format!(" {} ", "─".repeat(fill)), t.line()),
    ];
    if fill > 0 {
        spans.extend(heads);
    }
    Line::from(spans)
}

/// La ficha del agente bajo el cursor, a pantalla completa: nombre y grupo,
/// ruta, cifras, descripción, trabajo por día y ejecuciones. Como mucho `h`
/// líneas; lo que se recorta son las ejecuciones.
fn detail_lines(app: &App, t: &Theme, w: usize, h: usize) -> Vec<Line<'static>> {
    let Some(row) = app.current() else {
        return Vec::new();
    };
    let agent = app.agent(row.agent);
    let mut lines = vec![
        section_line(t, &agent.name, t.accent_bold(), &row.section, w),
        Line::from(Span::styled(
            format!(
                " {}",
                truncate(
                    &report::shorten(&agent.path, &app.home),
                    w.saturating_sub(2)
                )
            ),
            t.muted(),
        )),
        Line::default(),
        figures_line(app, &row, t, w),
        Line::default(),
    ];
    let description = agent.description.as_deref().unwrap_or("no description");
    for l in wrap(description, w.saturating_sub(2)) {
        lines.push(Line::from(Span::styled(format!(" {l}"), t.text())));
    }
    lines.push(Line::default());
    lines.extend(chart_lines(app, &row, t, w));
    lines.push(Line::default());
    let room = h.saturating_sub(lines.len() + 1);
    lines.extend(runs_lines(app, row.agent, t, w, room));
    lines.truncate(h);
    lines
}

/// ` 1 h 9 min trabajado   ·   10 ejecuciones   ·   7 min de media   ·   último uso hace 1 h`
///
/// Cifras en `ink` negrita, rótulos en `ink-muted` y los `·` en `subtle`;
/// si no cabe, los separadores se estrechan.
fn figures_line(app: &App, row: &Row, t: &Theme, w: usize) -> Line<'static> {
    let mut parts: Vec<Vec<Span<'static>>> = Vec::new();
    if let Some(r) = row.work.running {
        parts.push(vec![
            Span::styled("✦ ", t.accent()),
            Span::styled("running", t.accent()),
            Span::styled(", ", t.muted()),
            Span::styled(report::worked(r.worked), t.text_bold()),
        ]);
    }
    if row.work.runs == 0 {
        parts.push(vec![Span::styled("no runs recorded", t.muted())]);
    } else {
        parts.push(vec![
            Span::styled(report::worked(row.work.total), t.text_bold()),
            Span::styled(" worked", t.muted()),
        ]);
        parts.push(vec![
            Span::styled(row.work.runs.to_string(), t.text_bold()),
            Span::styled(if row.work.runs == 1 { " run" } else { " runs" }, t.muted()),
        ]);
        if let Some(mean) = row.mean() {
            parts.push(vec![
                Span::styled(report::worked(mean), t.text_bold()),
                Span::styled(" avg", t.muted()),
            ]);
        }
    }
    parts.push(vec![
        Span::styled("last used ", t.muted()),
        Span::styled(report::last_seen(row.ever, app.now), t.text()),
    ]);
    let total: usize = parts.iter().map(|p| spans_width(p)).sum();
    let gaps = parts.len().saturating_sub(1);
    let sep = if 1 + total + gaps * 7 < w {
        "   ·   "
    } else {
        " · "
    };
    let mut spans = vec![Span::raw(" ")];
    for (i, p) in parts.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(sep, t.line()));
        }
        spans.extend(p);
    }
    Line::from(spans)
}

/// ` Trabajo por día ─── últimos 14 días` y el gráfico: `CHART_ROWS` filas con
/// el máximo arriba y 0 abajo, dos columnas por día y los días del mes debajo.
fn chart_lines(app: &App, row: &Row, t: &Theme, w: usize) -> Vec<Line<'static>> {
    let mut lines = vec![section_line(
        t,
        "Work per day",
        t.accent(),
        &spark_label(),
        w,
    )];
    let days = row.days(DAYS);
    let max = days.iter().copied().max().unwrap_or_default();
    if max <= TimeDelta::zero() {
        lines.push(Line::from(Span::styled(
            format!(" no work in the {}", spark_label()),
            t.muted(),
        )));
        return lines;
    }
    let top = report::worked(max);
    // etiqueta: un espacio delante, el máximo y otro antes del eje
    let label_w = width(&top) + 2;
    for level in (1..=CHART_ROWS).rev() {
        let mut spans = if level == CHART_ROWS {
            vec![
                Span::styled(pad(&format!("{top} "), label_w, true), t.muted()),
                Span::styled("┤", t.line()),
            ]
        } else {
            vec![Span::raw(" ".repeat(label_w)), Span::styled("│", t.line())]
        };
        let cells: String = days
            .iter()
            .map(|&d| {
                let c = chart_cell(d, max, level);
                format!(" {c}{c}")
            })
            .collect();
        spans.push(Span::styled(cells, t.soft()));
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(vec![
        Span::styled(pad("0", label_w, true), t.muted()),
        Span::styled(format!("└{}", "─".repeat(3 * days.len() + 1)), t.line()),
    ]));
    let today = today(app);
    let first = today - chrono::Days::new(days.len().saturating_sub(1) as u64);
    let labels: String = (0..days.len())
        .map(|i| {
            let d = first + chrono::Days::new(i as u64);
            format!(" {:>2}", chrono::Datelike::day(&d))
        })
        .collect();
    let mut text = format!("{}{labels}", " ".repeat(label_w + 1));
    if let Some(span) = report::month_span(first, today) {
        text += &format!("   {span}");
    }
    lines.push(Line::from(Span::styled(text, t.muted())));
    lines
}

/// ` Ejecuciones ─── N · las K más recientes` y una línea por ejecución en
/// `room` líneas: fecha y hora local, duración, herramientas, descripción y,
/// si no es la raíz del proyecto, desde dónde.
fn runs_lines(app: &App, agent: usize, t: &Theme, w: usize, room: usize) -> Vec<Line<'static>> {
    let runs = app.runs_of(agent);
    let root = match &app.agent(agent).scope {
        crate::agents::Scope::Project(dir) => Some(dir.as_path()),
        crate::agents::Scope::Global => None,
    };
    let old = app.old_calls(agent);
    let (shown, more) = if runs.len() <= room {
        (runs.len(), false)
    } else {
        (room.saturating_sub(1), true)
    };
    let info = match shown {
        _ if !more => runs.len().to_string(),
        0 => runs.len().to_string(),
        1 => format!("{} · most recent", runs.len()),
        k => format!("{} · {k} most recent", runs.len()),
    };
    let mut lines = vec![section_line(t, "Runs", t.accent(), &info, w)];
    if runs.is_empty() && old == 0 {
        lines.push(Line::from(Span::styled("   none", t.muted())));
    }
    let shown_runs = &runs[..shown];
    let when = |r: &RunLine| r.start.map(report::day_time).unwrap_or_else(|| "—".into());
    let worked = |r: &RunLine| {
        let worked = report::worked(r.worked);
        if r.in_progress {
            format!("running {worked}")
        } else {
            worked
        }
    };
    let tools = |r: &RunLine| plural(r.tools, "tool", "tools");
    let max_of =
        |f: &dyn Fn(&RunLine) -> String| shown_runs.iter().map(|r| width(&f(r))).max().unwrap_or(0);
    let (when_w, worked_w, tools_w) = (max_of(&when), max_of(&worked), max_of(&tools));
    for r in shown_runs {
        let mut spans = vec![
            Span::styled(format!("   {}", pad(&when(r), when_w, false)), t.muted()),
            Span::raw("  "),
            Span::styled(
                pad(&worked(r), worked_w, true),
                if r.in_progress { t.accent() } else { t.text() },
            ),
            Span::styled(format!("  {}", pad(&tools(r), tools_w, true)), t.muted()),
        ];
        let used = spans_width(&spans);
        let room = w.saturating_sub(used + 3);
        let cwd = r.cwd.map(|c| report::within(c, root, &app.home));
        let cwd_w = cwd.as_deref().map_or(0, |c| width(c) + 2);
        if let Some(d) = r.description {
            spans.push(Span::styled(
                format!("  {}", truncate(d, room.saturating_sub(cwd_w).max(8))),
                t.text(),
            ));
        }
        if let Some(c) = cwd {
            let used = spans_width(&spans);
            spans.push(Span::styled(
                format!("  {}", truncate(&c, w.saturating_sub(used + 3))),
                t.muted(),
            ));
        }
        lines.push(Line::from(spans));
    }
    if more {
        lines.push(Line::from(Span::styled(
            format!("   … {} more", runs.len() - shown),
            t.muted(),
        )));
    }
    // las llamadas antiguas, solo si queda sitio
    if old > 0 && lines.len() <= room {
        lines.push(Line::from(Span::styled(
            format!(
                "   + {}",
                plural(
                    old,
                    "old call with no recorded duration",
                    "old calls with no recorded duration"
                )
            ),
            t.muted(),
        )));
    }
    lines
}

/// Teclas: la tecla en `claude` y la acción en `ink`. La de periodo lleva
/// detrás los tres, con el abierto en `claude-soft`, si `periods`.
fn keys_spans(
    app: &App,
    t: &Theme,
    keys: &[(&str, &str)],
    periods: bool,
    sorts: bool,
) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw(" ")];
    for (i, (k, label)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", t.muted()));
        }
        // en activos, enter solo vale con el cursor en un bloque
        let off = *k == "enter" && app.mode == Mode::Active && !app.can_open();
        let (key, action) = if off {
            (t.muted(), t.muted())
        } else {
            (t.accent(), t.text())
        };
        spans.push(Span::styled(k.to_string(), key));
        spans.push(Span::styled(format!(" {label}"), action));
        if *k == "p" && periods {
            spans.push(Span::styled(":", t.text()));
            for (j, p) in [Period::Week, Period::Month, Period::Ever]
                .into_iter()
                .enumerate()
            {
                spans.push(Span::styled(if j == 0 { " " } else { " · " }, t.muted()));
                spans.push(Span::styled(
                    report::period(p),
                    if p == app.period { t.soft() } else { t.muted() },
                ));
            }
        }
        if *k == "s" && sorts {
            spans.push(Span::styled(":", t.text()));
            for (j, key) in [RankKey::Worked, RankKey::Tokens, RankKey::Runs]
                .into_iter()
                .enumerate()
            {
                spans.push(Span::styled(if j == 0 { " " } else { " · " }, t.muted()));
                let label = if key == RankKey::Worked {
                    "time"
                } else {
                    rank_label(key)
                };
                spans.push(Span::styled(
                    label,
                    if key == app.rank_key {
                        t.soft()
                    } else {
                        t.muted()
                    },
                ));
            }
        }
    }
    spans
}

/// ` ↑↓ mover · enter ficha · …                                   1-12/35`
///
/// Si no cabe todo, se quitan primero los periodos del ranking y luego las
/// teclas de `MINOR_KEYS`, antes que el contador.
fn footer_line(app: &App, t: &Theme, w: usize, total: usize, rows: usize) -> Line<'static> {
    let pos = (rows > 0 && total > rows).then(|| {
        format!(
            "{}-{}/{}",
            app.scroll + 1,
            (app.scroll + rows).min(total),
            total
        )
    });
    // el contador, un hueco delante y la última columna libre
    let pos_w = pos.as_deref().map_or(0, |p| width(p) + 2);
    let ranking = app.mode == Mode::Ranking;
    let mut keys: Vec<(&str, &str)> = match app.mode {
        Mode::Ranking => RANKING_KEYS,
        Mode::Active => ACTIVE_KEYS,
        Mode::All => LIST_KEYS,
    }
    .to_vec();
    let (mut periods, mut sorts) = (ranking, ranking);
    let fits = |keys: &[(&str, &str)], periods: bool, sorts: bool| {
        spans_width(&keys_spans(app, t, keys, periods, sorts)) + pos_w <= w
    };
    // primero los periodos y luego las claves; la regla ya dice la abierta
    if !fits(&keys, periods, sorts) {
        periods = false;
    }
    if !fits(&keys, periods, sorts) {
        sorts = false;
    }
    for minor in MINOR_KEYS {
        if fits(&keys, periods, sorts) {
            break;
        }
        keys.retain(|(k, _)| k != minor);
    }
    footer_with_version(app, t, w, keys_spans(app, t, &keys, periods, sorts), pos)
}

/// Pie con lo que hay a la derecha: el contador de scroll, si lo hay, el
/// aviso de versión nueva y la versión (`1-27/30  ↑ v1.2.0 available ·
/// claude-agents update  v1.1.0`), en `ink-muted` salvo `↑ v1.2.0`, que va en
/// `claude`, y con una columna libre al borde. Lo que no cabe se va en este
/// orden: el aviso largo pasa a `↑ v1.2.0`, luego se va el aviso, luego la
/// versión (el contador se queda); las teclas se recortan antes, por quien llama.
fn footer_with_version(
    app: &App,
    t: &Theme,
    w: usize,
    left: Vec<Span<'static>>,
    pos: Option<String>,
) -> Line<'static> {
    let version = format!("v{}", app.version);
    let lead = pos.as_ref().map(|p| format!("{p}  ")).unwrap_or_default();
    let mut candidates: Vec<Vec<Span<'static>>> = Vec::new();
    if let Some(next) = &app.update_available {
        let arrow = format!("↑ v{next}");
        for tail in [" available · claude-agents update", ""] {
            candidates.push(vec![
                Span::styled(lead.clone(), t.muted()),
                Span::styled(arrow.clone(), t.accent()),
                Span::styled(format!("{tail}  {version}"), t.muted()),
            ]);
        }
    }
    candidates.push(vec![Span::styled(format!("{lead}{version}"), t.muted())]);
    if let Some(p) = pos {
        candidates.push(vec![Span::styled(p, t.muted())]);
    }
    // un hueco mínimo entre las teclas y lo de la derecha, y la última columna;
    // el contador, si es lo último que queda, se pone aunque no quepa el hueco
    let lw = spans_width(&left);
    let last = candidates.len() - 1;
    let right = candidates
        .into_iter()
        .enumerate()
        .find(|(i, c)| lw + spans_width(c) + 2 <= w || (*i == last && !lead.is_empty()))
        .map(|(_, c)| c);
    match right {
        Some(right) => split_line(left, right, w, t),
        None => Line::from(left),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{Context, LaunchedBy, RunLink};
    use crate::tui::app::tests::{fixture, run, with_runs};
    use crate::tui::app::{Action, App, Msg};
    use chrono::TimeDelta;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::style::Modifier;
    use ratatui::Terminal;
    use std::path::PathBuf;

    const STATION: &str = "/Users/x/Offing/argi/station";

    fn draw(app: &mut App, w: u16, h: u16) -> Vec<String> {
        let buf = draw_buf(app, w, h);
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Dibuja y devuelve el buffer, para mirar estilos.
    fn draw_buf(app: &mut App, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let t = Theme::claude_256();
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render(app, f, &t)).unwrap();
        term.backend().buffer().clone()
    }

    /// La línea sin la versión de la esquina, para mirar solo las teclas.
    fn bare(line: &str) -> String {
        let v = format!("v{}", fixture().version);
        line.trim_end_matches(v.as_str()).trim_end().to_string()
    }

    fn press(app: &mut App, code: KeyCode) {
        app.update(Action::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    #[test]
    fn the_order_hint_only_shows_in_all_mode() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('o'));
        assert!(draw(&mut app, 110, 20)[1].contains("sort: worked"));
        app.mode = crate::tui::app::Mode::Active;
        let lines = draw(&mut app, 110, 20);
        assert!(!lines[1].contains("sort"), "{:?}", lines[1]);
    }

    /// El texto de la fila `y` desde la columna `x` con `n` celdas.
    fn cells(buf: &ratatui::buffer::Buffer, x: u16, y: u16, n: u16) -> String {
        (x..x + n).map(|x| buf[(x, y)].symbol()).collect()
    }

    /// Columna (en celdas) donde empieza `pat` dentro de `line`.
    fn col(line: &str, pat: &str) -> u16 {
        let i = line
            .find(pat)
            .unwrap_or_else(|| panic!("sin {pat:?} en {line:?}"));
        width(&line[..i]) as u16
    }

    fn line_of<'a>(lines: &'a [String], name: &str) -> &'a str {
        let text = lines.join("\n");
        lines
            .iter()
            .find(|l| l.contains(&format!(". {name}")))
            .unwrap_or_else(|| panic!("sin {name}: {text}"))
    }

    fn y_of(lines: &[String], line: &str) -> u16 {
        lines.iter().position(|l| l == line).unwrap() as u16
    }

    /// docs lleva 1 h 12 min en dos ejecuciones (ayer y anteayer) y frontend
    /// 5 min en una.
    fn worked() -> App {
        with_runs(
            fixture(),
            vec![
                run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true),
                run("docs", STATION, "2026-11-03T09:00:00Z", 22, None, true),
                run("frontend", STATION, "2026-11-04T10:00:00Z", 5, None, true),
            ],
        )
    }

    /// backend en curso desde las 11:50 (lleva 10 min a las 12:00).
    fn running() -> App {
        with_runs(
            fixture(),
            vec![run(
                "backend",
                STATION,
                "2026-11-05T11:50:00Z",
                8,
                None,
                false,
            )],
        )
    }

    /// El fixture con un grupo más en el que nadie ha llamado nunca a nadie.
    fn with_unused() -> App {
        let mut app = fixture();
        let mut agents: Vec<_> = (0..app.agent_count())
            .map(|i| app.agent(i).clone())
            .collect();
        agents.push(crate::agents::Agent {
            name: "ai-engineer".into(),
            path: PathBuf::from("/Users/x/Offing/argi/_argibox/.claude/agents/ai-engineer.md"),
            scope: crate::agents::Scope::Project(PathBuf::from("/Users/x/Offing/argi/_argibox")),
            description: None,
        });
        app.update(Action::Msg(Msg::Agents(agents)));
        app
    }

    fn open(app: &mut App, name: &str) {
        let n = app
            .rows()
            .iter()
            .find(|r| app.agent(r.agent).name == name)
            .unwrap()
            .n;
        app.cursor = n - 1;
        press(app, KeyCode::Enter);
    }

    // --- formas: barrita, barra del ranking, gráfico y estrella ---

    fn mins(v: &[i64]) -> Vec<TimeDelta> {
        v.iter().map(|&m| TimeDelta::minutes(m)).collect()
    }

    #[test]
    fn spark_is_half_or_full_dash_by_half_of_the_row_max() {
        let days = mins(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 22, 50, 0]);
        assert_eq!(spark(&days), format!("{}╸━─", "─".repeat(11)));
        // hasta la mitad del máximo, media raya; por encima, raya entera
        assert_eq!(spark(&mins(&[10, 20, 30, 40])), "╸╸━━");
        // lo poco que no sea cero no se confunde con el cero
        assert_eq!(spark(&mins(&[1, 100])), "╸━");
        // un agente nunca usado: catorce ceros
        assert_eq!(spark(&mins(&[0; 14])), "─".repeat(14));
    }

    #[test]
    fn spark_spans_put_zeros_in_subtle_and_the_rest_in_claude_soft() {
        let t = Theme::claude_256();
        let spans = spark_spans(&mins(&[0, 0, 22, 50, 0]), &t);
        let got: Vec<(&str, Style)> = spans.iter().map(|s| (&*s.content, s.style)).collect();
        assert_eq!(
            got,
            vec![("──", t.line()), ("╸━", t.soft()), ("─", t.line())]
        );
        let spans = spark_spans(&mins(&[0; 14]), &t);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "─".repeat(14));
        assert_eq!(spans[0].style, t.line());
    }

    #[test]
    fn horizontal_bar_uses_eighths() {
        let m = |v: i64| v as f64;
        assert_eq!(hbar(m(69), m(69), 24), "█".repeat(24));
        assert_eq!(hbar(m(55), m(69), 24), format!("{}▏", "█".repeat(19)));
        assert_eq!(hbar(m(5), m(72), 24), "█▋");
        // lo que tenga algo se ve, aunque sea un octavo
        assert_eq!(hbar(m(1), m(1000), 24), "▏");
        assert_eq!(hbar(m(0), m(10), 24), "");
    }

    #[test]
    fn chart_cells_fill_from_the_bottom_with_eighths() {
        let m = TimeDelta::minutes;
        // 22 de 50 en 4 filas: 1,76 filas
        assert_eq!(chart_cell(m(22), m(50), 1), '█');
        assert_eq!(chart_cell(m(22), m(50), 2), '▆');
        assert_eq!(chart_cell(m(22), m(50), 3), ' ');
        assert_eq!(chart_cell(m(50), m(50), 4), '█');
        assert_eq!(chart_cell(m(1), m(50), 1), '▁');
        assert_eq!(chart_cell(m(0), m(50), 1), ' ');
    }

    #[test]
    fn the_running_star_blinks_every_half_second() {
        let at = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
        assert_eq!(star(at("2026-11-05T12:00:00Z")), "✦");
        assert_eq!(star(at("2026-11-05T12:00:00.499Z")), "✦");
        assert_eq!(star(at("2026-11-05T12:00:00.500Z")), "✧");
    }

    // --- cabecera y segunda línea ---

    #[test]
    fn top_row_has_the_three_tabs_and_the_counts() {
        let lines = draw(&mut fixture(), 110, 14);
        assert!(
            lines[0].starts_with("  active   all   ranking "),
            "{:?}",
            lines[0]
        );
        assert!(
            lines[0].ends_with("window 10 min · 5 agents · 4 sessions"),
            "{:?}",
            lines[0]
        );
        // a la derecha de las pestañas no se habla de activos
        let right = &lines[0][lines[0].find(" ranking ").unwrap() + 9..];
        assert!(!right.contains("activo"), "{right:?}");
    }

    /// Con algo en curso, la cabecera de «todos» lo cuenta entre agentes y
    /// sesiones; sin nada, no dice «0 en curso».
    #[test]
    fn all_header_counts_what_is_in_progress_only_when_there_is_some() {
        let lines = draw(&mut running(), 110, 22);
        assert!(
            lines[0].ends_with("window 10 min · 5 agents · 1 running · 4 sessions"),
            "{:?}",
            lines[0]
        );
        let right = &lines[0][lines[0].find(" ranking ").unwrap() + 9..];
        assert!(!right.contains("activo"), "{right:?}");
        let lines = draw(&mut fixture(), 110, 14);
        assert!(!lines[0].contains("running"), "{:?}", lines[0]);
        // solo los agentes de la lista: los built-in en curso no cuentan aquí,
        // que no tienen fila ni grupo donde verse
        let mut app = busy();
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.mode, Mode::All);
        let lines = draw(&mut app, 120, 22);
        assert!(
            lines[0].ends_with("· 5 agents · 1 running · 4 sessions"),
            "{:?}",
            lines[0]
        );
    }

    #[test]
    fn the_open_tab_is_painted_on_claude() {
        let t = Theme::claude_256();
        let mut app = fixture();
        // margen (1) + " active " (8) + " " + " all " (5) + " " + " ranking " (9)
        let buf = draw_buf(&mut app, 110, 14);
        assert_eq!(cells(&buf, 1, 0, 8), " active ");
        assert_eq!(cells(&buf, 10, 0, 5), " all ");
        assert_eq!(cells(&buf, 16, 0, 9), " ranking ");
        assert_eq!(buf[(11, 0)].style().bg, Some(t.claude), "all abierto");
        assert_eq!(buf[(11, 0)].style().fg, Some(t.on_claude));
        assert_ne!(buf[(2, 0)].style().bg, Some(t.claude), "active cerrado");
        press(&mut app, KeyCode::Tab);
        let buf = draw_buf(&mut app, 110, 14);
        assert_eq!(buf[(17, 0)].style().bg, Some(t.claude), "ranking abierto");
        assert_ne!(buf[(11, 0)].style().bg, Some(t.claude), "all cerrado");
        press(&mut app, KeyCode::Char('a'));
        let buf = draw_buf(&mut app, 110, 14);
        assert_eq!(buf[(2, 0)].style().bg, Some(t.claude), "active abierto");
    }

    /// La versión sola (sin el nombre) va en la esquina inferior derecha de la
    /// última línea, en `ink-muted`, con una columna libre al borde.
    #[test]
    fn the_version_sits_bottom_right_without_the_name() {
        let t = Theme::claude_256();
        let mut app = fixture();
        let v = format!("v{}", app.version);
        let buf = draw_buf(&mut app, 110, 14);
        let lines = draw(&mut app, 110, 14);
        assert!(!lines[0].contains("claude-agents"), "{:?}", lines[0]);
        assert!(lines[0].starts_with("  active   all   ranking "));
        let last = &lines[13];
        assert!(last.ends_with(&v), "{last:?}");
        // la última columna queda libre y la versión acaba en la 108
        let x = 110 - 1 - v.len() as u16;
        assert_eq!(cells(&buf, x, 13, v.len() as u16), v);
        assert_eq!(buf[(x, 13)].style().fg, Some(t.ink_muted));
        assert_eq!(buf[(109, 13)].symbol(), " ");
    }

    /// El contador de scroll queda a la izquierda de la versión, a dos espacios.
    #[test]
    fn the_scroll_counter_sits_left_of_the_version() {
        let mut app = fixture();
        let lines = draw(&mut app, 110, 8);
        let v = format!("v{}", app.version);
        assert!(
            lines[7].ends_with(&format!("1-5/11  {v}")),
            "{:?}",
            lines[7]
        );
    }

    /// Estrecha, se cae la versión antes que las teclas.
    #[test]
    fn a_narrow_footer_drops_the_version_before_the_keys() {
        let mut app = fixture();
        let v = format!("v{}", app.version);
        let wide = draw(&mut app, 110, 14);
        let keys = wide[13].trim_end_matches(v.as_str()).trim_end().to_string();
        let kw = width(&keys);
        // teclas + un hueco + versión + la columna libre: cabe; una menos, la
        // versión se cae y las teclas se quedan
        let fits = draw(&mut app, (kw + 1 + v.len() + 1) as u16, 14);
        assert_eq!(fits[13], format!("{keys} {v}"), "{:?}", fits[13]);
        let tight = draw(&mut app, (kw + v.len() + 1) as u16, 14);
        assert_eq!(tight[13], keys, "{:?}", tight[13]);
    }

    /// Con versión nueva, el pie la anuncia a la izquierda de la versión en
    /// marcha: `↑ v1.2.0` en `claude`, el resto en `ink-muted`.
    #[test]
    fn a_new_release_is_announced_left_of_the_version() {
        let t = Theme::claude_256();
        let mut app = fixture();
        app.update_available = Some("1.2.0".into());
        let v = format!("v{}", app.version);
        let lines = draw(&mut app, 140, 14);
        let notice = format!("↑ v1.2.0 available · claude-agents update  {v}");
        assert!(lines[13].ends_with(&notice), "{:?}", lines[13]);
        let buf = draw_buf(&mut app, 140, 14);
        let x = 140 - 1 - width(&notice) as u16;
        assert_eq!(buf[(x, 13)].symbol(), "↑");
        assert_eq!(buf[(x, 13)].style().fg, Some(t.claude));
        assert_eq!(buf[(x + 9, 13)].style().fg, Some(t.ink_muted));
    }

    /// Estrecha, el aviso se acorta y luego se va, antes que las teclas y que
    /// la versión en marcha.
    #[test]
    fn a_narrow_footer_shortens_the_notice_before_the_keys() {
        let mut app = fixture();
        let v = format!("v{}", app.version);
        let keys = bare(&draw(&mut app, 110, 14)[13]);
        let kw = width(&keys);
        app.update_available = Some("1.2.0".into());
        let short = format!("↑ v1.2.0  {v}");
        let fits = draw(&mut app, (kw + 1 + width(&short) + 1) as u16, 14);
        assert_eq!(fits[13], format!("{keys} {short}"), "{:?}", fits[13]);
        let tight = draw(&mut app, (kw + width(&short) + 1) as u16, 14);
        assert!(tight[13].starts_with(&keys), "{:?}", tight[13]);
        assert!(tight[13].ends_with(&format!(" {v}")), "{:?}", tight[13]);
        assert!(!tight[13].contains("↑ v"), "{:?}", tight[13]);
    }

    /// Las fichas (agente y ejecución) llevan la versión en su pie.
    #[test]
    fn the_detail_screens_carry_the_version_too() {
        let mut app = worked();
        open(&mut app, "docs");
        let v = format!(" v{}", app.version);
        let lines = draw(&mut app, 100, 26);
        assert!(lines[25].starts_with(" esc back"), "{:?}", lines[25]);
        assert!(lines[25].ends_with(&v), "{:?}", lines[25]);
        let mut app = run_app();
        let lines = draw(&mut app, 100, 26);
        assert!(lines[25].ends_with(&v), "{:?}", lines[25]);
        assert!(!lines[0].contains("claude-agents"), "{:?}", lines[0]);
    }

    #[test]
    fn narrow_top_row_keeps_the_tabs_and_shortens_the_counts() {
        // sin el nombre hay más sitio: a 80 columnas caben ya las sesiones
        let lines = draw(&mut fixture(), 80, 14);
        assert!(
            lines[0].ends_with("window 10 min · 5 agents · 4 sessions"),
            "{:?}",
            lines[0]
        );
        let lines = draw(&mut fixture(), 62, 14);
        assert!(
            lines[0].starts_with("  active   all   ranking "),
            "{:?}",
            lines[0]
        );
        assert!(
            lines[0].ends_with("window 10 min · 5 agents"),
            "{:?}",
            lines[0]
        );
    }

    #[test]
    fn ranking_header_says_the_period_and_its_total() {
        let mut app = worked();
        press(&mut app, KeyCode::Tab);
        let lines = draw(&mut app, 110, 14);
        assert!(
            lines[0].ends_with("all time · 1 h 17 min in 3 runs"),
            "{:?}",
            lines[0]
        );
        press(&mut app, KeyCode::Char('p'));
        let lines = draw(&mut app, 110, 14);
        assert!(
            lines[0].ends_with("7 days · 1 h 17 min in 3 runs"),
            "{:?}",
            lines[0]
        );
    }

    #[test]
    fn second_line_says_the_order_only_when_it_is_not_the_default() {
        let mut app = fixture();
        assert_eq!(draw(&mut app, 110, 14)[1], "");
        press(&mut app, KeyCode::Char('o'));
        let lines = draw(&mut app, 110, 14);
        assert!(lines[1].ends_with("sort: worked"), "{:?}", lines[1]);
    }

    #[test]
    fn filter_row_shows_what_is_typed() {
        let mut app = fixture();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('x'));
        let lines = draw(&mut app, 100, 8);
        assert!(lines[1].starts_with(" filter x"), "{:?}", lines[1]);
        assert!(lines[2].starts_with(" nothing to show"), "{:?}", lines[2]);
    }

    // --- lista ---

    #[test]
    fn groups_carry_their_folder_and_how_many_agents() {
        let lines = draw(&mut worked(), 110, 16);
        // la línea 2 es la de columnas
        assert!(
            lines[3].starts_with(" taskflow · 1 agent ─"),
            "{:?}",
            lines[3]
        );
        assert!(lines[4].starts_with(" ❯  1. visual   "), "{:?}", lines[4]);
        assert!(
            lines[6].starts_with(" global · 1 agent ─"),
            "{:?}",
            lines[6]
        );
        assert!(
            lines[9].starts_with(" station · 3 agents ─"),
            "{:?}",
            lines[9]
        );
        // en «todos» no se habla de activos ni hay puntos
        for l in &lines {
            assert!(!l.contains('●'), "{l:?}");
        }
        let body = &lines[1..];
        assert!(!body.iter().any(|l| l.contains("activo")), "{body:#?}");
        assert!(!body.iter().any(|l| l.contains("running")), "{body:#?}");
        assert!(!body.iter().any(|l| l.contains("unused")), "{body:#?}");
    }

    /// Las cifras de la regla de un grupo abierto caen en las columnas de las
    /// filas: la barrita del grupo bajo «últimos 14 días», el total bajo
    /// «trabajado», la suma de ejecuciones bajo «ejec.» y el último uso del
    /// grupo bajo «último uso», en el borde. La regla acaba dos columnas antes
    /// del bloque; la de un plegado, en el borde.
    #[test]
    fn open_group_rules_put_their_totals_in_the_row_columns() {
        for w in [150u16, 110, 90] {
            let app = with_runs(
                with_unused(),
                vec![
                    run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true),
                    run("docs", STATION, "2026-11-03T09:00:00Z", 22, None, true),
                    run("frontend", STATION, "2026-11-04T10:00:00Z", 5, None, true),
                ],
            );
            let lines = draw(&mut { app }, w, 24);
            let edge = w as usize - 1;
            let head = &lines[2];
            let station = lines
                .iter()
                .find(|l| l.starts_with(" station · "))
                .unwrap_or_else(|| panic!("{w}: {lines:#?}"));
            let docs = line_of(&lines, "docs");
            // total del grupo bajo trabajado, ejecuciones bajo ejec.
            assert_eq!(
                end_col(station, "1 h 17 min"),
                end_col(head, "worked"),
                "{w}:\n{head}\n{station}"
            );
            assert_eq!(end_col(station, "1 h 17 min"), end_col(docs, "1 h 12 min"));
            assert!(
                station.contains("1 h 17 min      3    2 min ago"),
                "{w}: {station:?}"
            );
            assert_eq!(col(station, "3    2 min") + 1, end_col(head, "runs") as u16);
            // último uso del grupo (el más reciente, frontend) en el borde
            assert_eq!(end_col(station, "2 min ago"), edge, "{w}: {station:?}");
            assert!(line_of(&lines, "frontend").ends_with("2 min ago"));
            assert_eq!(end_of(station), edge, "{w}: {station:?}");
            // la regla acaba dos columnas antes del bloque de la derecha
            // (en ancha, la barrita del grupo acaba en `─`: se busca antes de ella)
            let before = match station.find(&format!("  {}╸━─", "─".repeat(11))) {
                Some(i) if w >= 100 => &station[..i],
                _ => station.as_str(),
            };
            let rule_end = width(&before[..before.rfind('─').unwrap()]) + 1;
            let block = if w >= 100 {
                col(head, "last 14 days") as usize - (spark_w() - width("last 14 days"))
            } else {
                col(head, "worked") as usize - (width("1 h 17 min") - width("worked"))
            };
            assert_eq!(rule_end + 2, block, "{w}:\n{head}\n{station}");
            if w >= 100 {
                // la barrita del grupo, en rayas y bajo su cabecera
                let spark = format!("{}╸━─", "─".repeat(11));
                assert!(station.contains(&spark), "{w}: {station:?}");
                assert_eq!(end_col(station, &spark), end_col(head, "last 14 days"));
            }
            // el plegado: su regla hasta el borde, sin cifras
            let folded = lines.iter().find(|l| l.contains("_argibox")).unwrap();
            assert!(
                folded.contains("▸ _argibox · 1 agent unused ─"),
                "{w}: {folded:?}"
            );
            assert!(folded.ends_with('─'), "{w}: {folded:?}");
            assert_eq!(end_of(folded), edge, "{w}: {folded:?}");
            // en «todos» no queda ningún bloque de los de antes
            for l in &lines[2..] {
                assert!(!l.chars().any(|c| ('▁'..='█').contains(&c)), "{w}: {l:?}");
                assert!(!l.contains("active ·"), "{w}: {l:?}");
            }
        }
    }

    /// La regla de un grupo con algo en curso lo cuenta tras sus agentes: la
    /// cuenta de agentes en `ink-muted` y la de en curso en `claude`.
    #[test]
    fn a_running_group_counts_its_agents_in_progress_on_claude() {
        let t = Theme::claude_256();
        let mut app = running();
        let lines = draw(&mut app, 110, 22);
        let station = lines
            .iter()
            .find(|l| l.starts_with(" station · "))
            .unwrap_or_else(|| panic!("{lines:#?}"));
        assert!(
            station.starts_with(" station · 3 agents · 1 running ─"),
            "{station:?}"
        );
        let y = y_of(&lines, station);
        let buf = draw_buf(&mut app, 110, 22);
        assert_eq!(buf[(col(station, "station"), y)].style().fg, Some(t.claude));
        assert_eq!(
            buf[(col(station, "3 agents"), y)].style().fg,
            Some(t.ink_muted)
        );
        assert_eq!(
            buf[(col(station, "1 running"), y)].style().fg,
            Some(t.claude)
        );
        // los demás grupos, sin nada en curso, no lo dicen
        let global = lines.iter().find(|l| l.starts_with(" global · ")).unwrap();
        assert!(global.starts_with(" global · 1 agent ─"), "{global:?}");
        for l in &lines {
            assert!(!l.contains('●'), "{l:?}");
        }
    }

    /// Un grupo con algo en curso deja vacío su último uso, como la fila.
    #[test]
    fn a_running_group_has_no_last_use_in_its_rule() {
        let lines = draw(&mut running(), 110, 22);
        let station = lines
            .iter()
            .find(|l| l.starts_with(" station · "))
            .unwrap_or_else(|| panic!("{lines:#?}"));
        assert!(!station.contains("ago"), "{station:?}");
        let head = &lines[2];
        assert_eq!(end_of(station), end_col(head, "runs"), "{station:?}");
    }

    #[test]
    fn the_group_total_is_bold_ink_and_its_runs_muted() {
        let t = Theme::claude_256();
        let mut app = worked();
        let lines = draw(&mut app, 110, 16);
        let station = lines.iter().find(|l| l.starts_with(" station · ")).unwrap();
        let y = y_of(&lines, station);
        let buf = draw_buf(&mut app, 110, 16);
        let cell = &buf[(col(station, "1 h 17 min"), y)];
        assert_eq!(cell.style().fg, Some(t.ink));
        assert!(cell
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD));
        let at = col(station, "3    2 min");
        assert_eq!(buf[(at, y)].style().fg, Some(t.ink_muted));
        assert_eq!(buf[(col(station, "╸━"), y)].style().fg, Some(t.claude_soft));
        assert_eq!(
            buf[(col(station, "╸━") + 1, y)].style().fg,
            Some(t.claude_soft)
        );
        let zero = col(station, "╸━") + 2;
        assert_eq!(buf[(zero, y)].symbol(), "─");
        assert_eq!(buf[(zero, y)].style().fg, Some(t.subtle));
    }

    #[test]
    fn wide_rows_have_the_spark_worked_runs_and_last_use_under_their_headers() {
        let lines = draw(&mut worked(), 110, 16);
        let head = &lines[2];
        let docs = line_of(&lines, "docs");
        assert!(docs.contains(&format!("{}╸━─", "─".repeat(11))), "{docs:?}");
        assert!(docs.ends_with("1 h 12 min      2    8 min ago"), "{docs:?}");
        // cada columna bajo su cabecera; la de la barrita, alineada a la derecha
        assert_eq!(
            end_col(head, "days"),
            end_col(docs, "╸━─"),
            "{head:?}\n{docs:?}"
        );
        let end = |l: &str, pat: &str| col(l, pat) + width(pat) as u16;
        assert_eq!(end(head, "worked"), end(docs, "1 h 12 min"));
        assert_eq!(end(head, "runs"), col(docs, "2    8 min") + 1);
        assert_eq!(width(head), width(docs), "{head:?}\n{docs:?}");
        assert!(head.ends_with("last used"), "{head:?}");
        // sin ejecuciones: barrita a cero y columnas vacías
        let visual = line_of(&lines, "visual");
        assert!(visual.contains(&"─".repeat(14)), "{visual:?}");
        let gap = SPARK_GAP + 10 + COL_GAP + 4 + COL_GAP + 3;
        assert!(
            visual.ends_with(&format!("─{}5 s ago", " ".repeat(gap))),
            "{visual:?}"
        );
    }

    #[test]
    fn the_spark_is_soft_and_its_zero_days_are_subtle() {
        let t = Theme::claude_256();
        let mut app = worked();
        let lines = draw(&mut app, 110, 16);
        let docs = line_of(&lines, "docs");
        let (y, x) = (y_of(&lines, docs), col(docs, "╸━"));
        let buf = draw_buf(&mut app, 110, 16);
        assert_eq!(buf[(x, y)].style().fg, Some(t.claude_soft));
        assert_eq!(buf[(x - 1, y)].style().fg, Some(t.subtle));
    }

    #[test]
    fn under_100_columns_the_spark_goes_and_under_80_the_runs_too() {
        let lines = draw(&mut worked(), 90, 16);
        assert!(!lines[2].contains("last 14"), "{:?}", lines[2]);
        assert!(lines[2].contains("runs"), "{:?}", lines[2]);
        let docs = line_of(&lines, "docs");
        assert!(!docs.contains(['─', '╸', '━']), "{docs:?}");
        assert!(docs.ends_with("1 h 12 min      2    8 min ago"), "{docs:?}");
        let lines = draw(&mut worked(), 79, 16);
        assert!(!lines[2].contains("runs"), "{:?}", lines[2]);
        let docs = line_of(&lines, "docs");
        assert!(docs.ends_with("1 h 12 min    8 min ago"), "{docs:?}");
    }

    #[test]
    fn very_narrow_rows_keep_the_last_use() {
        let lines = draw(&mut worked(), 40, 16);
        let docs = line_of(&lines, "docs");
        assert!(docs.ends_with("8 min ago"), "{docs:?}");
    }

    #[test]
    fn inactive_rows_show_their_last_call_ever_or_never() {
        let mut app = fixture();
        let lines = draw(&mut app, 110, 16);
        let backend = line_of(&lines, "backend");
        assert!(backend.ends_with("30 min ago"), "{backend:?}");
        assert!(!backend.contains('●'), "{backend:?}");
    }

    #[test]
    fn a_running_agent_blinks_its_star_instead_of_a_last_use() {
        let t = Theme::claude_256();
        let mut app = running();
        let lines = draw(&mut app, 110, 22);
        let backend = line_of(&lines, "backend");
        assert!(backend.contains("backend ✦"), "{backend:?}");
        assert!(!backend.contains("running"), "{backend:?}");
        let y = y_of(&lines, backend);
        let buf = draw_buf(&mut app, 110, 22);
        assert_eq!(buf[(col(backend, "✦"), y)].style().fg, Some(t.claude));
        // medio segundo después, la estrella hueca
        app.now += TimeDelta::milliseconds(500);
        let lines = draw(&mut app, 110, 22);
        assert!(line_of(&lines, "backend").contains("backend ✧"));
        // usado en la ventana pero no en curso: sin marca, y su nombre
        // empieza donde el de backend; el último uso lo dice la columna
        let frontend = line_of(&lines, "frontend");
        assert!(!frontend.contains('●'), "{frontend:?}");
        assert!(frontend.ends_with("2 min ago"), "{frontend:?}");
        assert_eq!(col(frontend, "frontend"), col(backend, "backend"));
        // el bloque de la derecha no se mueve por la marca
        assert_eq!(end_col(frontend, "ago"), end_of(&lines[2]));
    }

    // --- todos a todo el ancho, títulos con la carpeta ---

    /// El fixture con un agente más, sin usar, en `dir`.
    fn with_agent(mut app: App, name: &str, dir: &str) -> App {
        let mut agents: Vec<_> = (0..app.agent_count())
            .map(|i| app.agent(i).clone())
            .collect();
        agents.push(crate::agents::Agent {
            name: name.into(),
            path: PathBuf::from(format!("{dir}/.claude/agents/{name}.md")),
            scope: crate::agents::Scope::Project(PathBuf::from(dir)),
            description: None,
        });
        app.update(Action::Msg(Msg::Agents(agents)));
        app
    }

    /// A 150, 110 y 90 columnas: el último uso de cada fila y la cabecera de
    /// columnas acaban donde el texto de las reglas de grupo; la barrita va
    /// pegada al bloque de datos, a dos espacios, y el hueco sobrante queda
    /// entre el nombre y la barrita.
    #[test]
    fn all_rows_fill_the_width_and_end_the_last_use_at_the_rule() {
        let mut gaps = Vec::new();
        for w in [150u16, 110, 90] {
            let lines = draw(&mut worked(), w, 20);
            let rules: Vec<&String> = lines.iter().filter(|l| l.contains('─')).collect();
            assert!(!rules.is_empty(), "{w}: {lines:#?}");
            let edge = w as usize - 1;
            for l in &rules {
                assert_eq!(end_of(l), edge, "{w}: {l:?}");
            }
            let head = &lines[2];
            assert_eq!(end_col(head, "last used"), edge, "{w}: {head:?}");
            for name in ["docs", "frontend", "backend", "visual"] {
                let row = line_of(&lines, name);
                assert_eq!(end_of(row), edge, "{w}: {row:?}");
            }
            let docs = line_of(&lines, "docs");
            assert_eq!(end_col(head, "worked"), end_col(docs, "1 h 12 min"));
            assert_eq!(end_col(docs, "8 min ago"), edge, "{w}: {docs:?}");
            if w >= 100 {
                // la barrita acaba dos antes de lo trabajado, y su cabecera con ella
                let spark_end = end_col(docs, "╸━─");
                assert_eq!(spark_end + SPARK_GAP, col(docs, "1 h 12 min") as usize);
                assert_eq!(end_col(head, "last 14 days"), spark_end, "{head:?}");
                // el hueco sobrante, entre el nombre y la barrita
                let name_end = end_col(docs, "docs");
                let gap = spark_end - DAYS - name_end;
                assert!(gap >= NAME_GAP, "{w}: {docs:?}");
                gaps.push(gap);
            } else {
                assert!(!docs.contains(['─', '╸', '━']), "{w}: {docs:?}");
            }
        }
        // cuarenta columnas más, cuarenta más de hueco
        assert_eq!(gaps[0], gaps[1] + 40, "{gaps:?}");
    }

    #[test]
    fn group_titles_show_only_the_folder() {
        let lines = draw(&mut worked(), 110, 16);
        assert!(lines[3].starts_with(" taskflow · "), "{:?}", lines[3]);
        assert!(
            lines.iter().any(|l| l.starts_with(" station · ")),
            "{lines:#?}"
        );
        assert!(lines.iter().any(|l| l.starts_with(" global · ")));
        assert!(!lines.iter().any(|l| l.contains("~/")), "{lines:#?}");
        let lines = draw(&mut with_unused(), 110, 20);
        let folded = lines.iter().find(|l| l.contains("_argibox")).unwrap();
        assert!(folded.starts_with("    6. ▸ _argibox · "), "{folded:?}");
    }

    /// Dos grupos con la misma carpeta se distinguen por la de encima.
    #[test]
    fn groups_with_the_same_folder_carry_their_parent() {
        let mut app = with_agent(fixture(), "kiosk", "/Users/x/Towerforge/station");
        let lines = draw(&mut app, 110, 20);
        assert!(
            lines.iter().any(|l| l.starts_with(" argi/station · ")),
            "{lines:#?}"
        );
        let folded = lines.iter().find(|l| l.contains('▸')).unwrap();
        assert!(folded.contains("▸ Towerforge/station · "), "{folded:?}");
        // si el filtro deja uno solo, vuelve a ser solo la carpeta
        app.filter = "argi".into();
        let lines = draw(&mut app, 110, 20);
        assert!(
            lines.iter().any(|l| l.starts_with(" station · ")),
            "{lines:#?}"
        );
    }

    /// El título es presentación: el filtro sigue mirando la ruta entera.
    #[test]
    fn the_filter_still_finds_a_group_by_a_component_of_its_path() {
        let mut app = worked();
        app.filter = "argi".into();
        let lines = draw(&mut app, 110, 16);
        assert!(
            lines.iter().any(|l| l.starts_with(" station · ")),
            "{lines:#?}"
        );
        assert!(line_of(&lines, "docs").contains("docs"));
        assert!(!lines.iter().any(|l| l.contains("taskflow")), "{lines:#?}");
    }

    /// En curso: la celda de último uso va vacía; ya lo dice la estrella.
    #[test]
    fn a_running_agent_has_no_last_use() {
        for w in [150u16, 110, 90] {
            let lines = draw(&mut running(), w, 22);
            let backend = line_of(&lines, "backend");
            assert!(!backend.contains("running"), "{w}: {backend:?}");
            assert!(!backend.contains("ago"), "{w}: {backend:?}");
            let head = &lines[2];
            let at = col(head, "last used") as usize;
            let tail: String = format!("{backend:<width$}", width = w as usize)
                .chars()
                .skip(at)
                .collect();
            assert!(tail.trim().is_empty(), "{w}: {backend:?}");
            let frontend = line_of(&lines, "frontend");
            assert_eq!(end_of(frontend), w as usize - 1, "{w}: {frontend:?}");
        }
    }

    // --- activos: el monitor ---

    /// backend en curso y dos built-in; en la ventana, las llamadas de
    /// visual, react-structure-checker, frontend y docs.
    fn busy() -> App {
        let mut app = with_runs(
            fixture(),
            vec![
                run("backend", STATION, "2026-11-05T11:50:00Z", 8, None, false),
                run(
                    "general-purpose",
                    "/Users/x/otro",
                    "2026-11-05T11:55:00Z",
                    4,
                    None,
                    false,
                ),
                run("Explore", STATION, "2026-11-05T11:57:00Z", 2, None, false),
            ],
        );
        press(&mut app, KeyCode::Char('a'));
        app
    }

    fn find<'a>(lines: &'a [String], pat: &str) -> &'a str {
        let text = lines.join("\n");
        lines
            .iter()
            .find(|l| l.contains(pat))
            .unwrap_or_else(|| panic!("sin {pat:?}: {text}"))
    }

    /// Un workflow de `visual` con skill (sin herramientas aún), un built-in con
    /// encargo largo y una herramienta, y backend definido sin encargo, skill ni
    /// herramienta.
    fn blocks_runs() -> Vec<crate::runs::Run> {
        let mut wf = run(
            "workflow-subagent",
            STATION,
            "2026-11-05T11:59:30Z",
            0,
            None,
            false,
        );
        wf.description = Some("visual".into());
        wf.task = Some("pintar la pantalla de activos".into());
        wf.skill = Some("superpowers:brainstorming".into());
        wf.tools = 0;
        let mut gp = run(
            "general-purpose",
            "/Users/x/otro",
            "2026-11-05T11:59:00Z",
            1,
            None,
            false,
        );
        gp.tools = 37;
        gp.task = Some("Revisión independiente del cambio sin commitear en /Users/x/otro".into());
        gp.last_tool = Some("Bash · git diff --stat".into());
        let mut be = run("backend", STATION, "2026-11-05T11:50:00Z", 8, None, false);
        be.description = None;
        vec![wf, gp, be]
    }

    fn blocks() -> App {
        let mut app = with_runs(fixture(), blocks_runs());
        press(&mut app, KeyCode::Char('a'));
        app
    }

    /// La primera línea de bloque (numerada) que lleva `name`.
    fn blk(lines: &[String], name: &str) -> usize {
        lines
            .iter()
            .position(|l| {
                let head = l.trim_start().trim_start_matches('❯').trim_start();
                head.starts_with(|c: char| c.is_ascii_digit()) && l.contains(name)
            })
            .unwrap_or_else(|| panic!("sin bloque {name:?}: {lines:#?}"))
    }

    /// La fila de una ejecución en curso de `name`, que va sola o encabeza
    /// su racimo: la línea justo bajo la de su agente.
    fn run_of(lines: &[String], name: &str) -> usize {
        lines
            .iter()
            .position(|l| {
                l.starts_with("         ")
                    && l.trim_start()
                        .strip_prefix(name)
                        .is_some_and(|r| r.is_empty() || r.starts_with(' '))
            })
            .map(|a| a + 1)
            .unwrap_or_else(|| panic!("sin agente {name:?}: {lines:#?}"))
    }

    /// Posición de la primera línea que contiene `pat`.
    fn pos(lines: &[String], pat: &str) -> usize {
        lines
            .iter()
            .position(|l| l.contains(pat))
            .unwrap_or_else(|| panic!("sin {pat:?}: {lines:#?}"))
    }

    /// Un racimo de tres `docs` en curso en station (el primero con skill y
    /// herramienta, el segundo con herramienta, el tercero con skill y sin
    /// herramienta), con contexto y tokens, junto a las llamadas terminadas
    /// del fixture.
    fn chain_app() -> App {
        let mut runs = Vec::new();
        for (i, (start, task, skill, tool, ctx)) in [
            (
                "2026-11-05T11:59:00Z",
                "Revisando convenciones de docs/README",
                Some("superpowers:brainstorming"),
                Some("Read · docs/interface.md"),
                82_000,
            ),
            (
                "2026-11-05T11:58:30Z",
                "Revisando targets del Makefile",
                None,
                Some("Bash · grep -n release Makefile"),
                44_000,
            ),
            (
                "2026-11-05T11:58:00Z",
                "Leyendo contact-form.astro y cookie-table",
                Some("frontend-design:frontend-design"),
                None,
                96_000,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let mut r = run(
                "docs",
                STATION,
                start,
                1,
                Some(&format!("toolu_d{i}")),
                false,
            );
            r.task = Some(task.into());
            r.skill = skill.map(String::from);
            r.last_tool = tool.map(String::from);
            r.usage = usage(140_000 + i as u64 * 1000, Some(ctx));
            runs.push(r);
        }
        let mut app = with_runs(fixture(), runs);
        press(&mut app, KeyCode::Char('a'));
        app
    }

    /// El racimo: el agente en su propia línea encima de la cabeza (nombre,
    /// etiqueta y ×N, sin número, estrella ni cifras), las tareas colgadas
    /// de `├`/`└` en la columna del nombre, la guía `│` bajo las que siguen
    /// y dos espacios bajo la última.
    #[test]
    fn a_chain_puts_the_agent_on_its_own_line_and_hangs_its_tasks_from_a_tree() {
        let mut app = chain_app();
        let lines = draw(&mut app, 110, 30);
        let g = pos(&lines, " station · 3 running · 1 recent ──");
        assert_eq!(lines[g + 1], "         docs [own] ×3", "{lines:#?}");
        let tree = [
            "    3. ✦ ├ Revisando convenciones de docs/README  ",
            "         │ skill  superpowers:brainstorming",
            "         │ now    Read · docs/interface.md",
            "    4. ✦ ├ Revisando targets del Makefile  ",
            "         │ now    Bash · grep -n release Makefile",
            "    5. ✦ └ Leyendo contact-form.astro y cookie-table  ",
            "           skill  frontend-design:frontend-design",
            "           now    —",
        ];
        for (k, want) in tree.iter().enumerate() {
            let got = &lines[g + 2 + k];
            if want.ends_with("  ") {
                assert!(got.starts_with(want), "{got:?}");
            } else {
                assert_eq!(got, want, "{lines:#?}");
            }
        }
        // las cifras de cada fila del racimo, pegadas al borde
        let head = &lines[2];
        for (y, tail) in [
            (g + 2, "▰▰▰▰▱▱▱▱ 41%       1 min     140k"),
            (g + 5, "▰▰▱▱▱▱▱▱ 22%       1 min     141k"),
            (g + 7, "▰▰▰▰▱▱▱▱ 48%       2 min     142k"),
        ] {
            assert!(lines[y].ends_with(tail), "{:?}", lines[y]);
            assert_eq!(end_of(&lines[y]), end_col(head, "tokens"));
        }
        // la terminada que sigue, en su fila de siempre
        assert!(
            lines[g + 10].starts_with("    6.   frontend [own]  —"),
            "{lines:#?}"
        );
    }

    #[test]
    fn a_chain_paints_its_name_soft_and_its_tree_subtle() {
        let t = Theme::claude_256();
        let mut app = chain_app();
        let lines = draw(&mut app, 110, 30);
        let buf = draw_buf(&mut app, 110, 30);
        let a = pos(&lines, "docs [own] ×3");
        let at = |y: usize, pat: &str| buf[(col(&lines[y], pat), y as u16)].style();
        let name = at(a, "docs");
        assert_eq!(name.fg, Some(t.claude_soft));
        assert!(name.add_modifier.contains(Modifier::BOLD));
        assert_eq!(at(a, "[own]").fg, Some(t.ink_muted));
        assert_eq!(at(a, "×3").fg, Some(t.subtle));
        let h = blk(&lines, "Revisando convenciones");
        assert_eq!(at(h, "├").fg, Some(t.subtle));
        assert_eq!(at(h, "Revisando").fg, Some(t.ink));
        assert_eq!(at(h + 1, "│").fg, Some(t.subtle));
        let l = blk(&lines, "Leyendo");
        assert_eq!(at(l, "└").fg, Some(t.subtle));
        // la cabeza y su agente alinean: glifo y nombre en la misma columna
        assert_eq!(col(&lines[h], "├"), col(&lines[a], "docs"));
        assert_eq!(col(&lines[l], "└"), col(&lines[a], "docs"));
    }

    /// Una en curso sola es un racimo de uno: el agente en su propia línea
    /// encima (sin ×N), la tarea colgada de `└` y las sublíneas tras la guía
    /// de dos espacios; la línea del agente se pulsa como su fila. La
    /// terminada sigue en una sola línea.
    #[test]
    fn a_single_run_in_progress_puts_the_agent_on_its_own_line_like_a_chain_of_one() {
        let t = Theme::claude_256();
        let mut app = blocks();
        let lines = draw(&mut app, 110, 60);
        let buf = draw_buf(&mut app, 110, 60);
        let g = pos(&lines, " station · 2 running · 1 recent ──");
        assert_eq!(lines[g + 1], "         visual [workflow]", "{lines:#?}");
        assert!(
            lines[g + 2].starts_with("    3. ✦ └ pintar la pantalla de activos  "),
            "{:?}",
            lines[g + 2]
        );
        assert!(
            lines[g + 2].ends_with("—      <1 min        —"),
            "{:?}",
            lines[g + 2]
        );
        assert_eq!(lines[g + 3], "           skill  superpowers:brainstorming");
        assert_eq!(lines[g + 4], "           now    —");
        assert_eq!(lines[g + 5], "         backend [own]", "sin ×1");
        assert!(
            lines[g + 6].starts_with("    4. ✦ └ —  "),
            "{:?}",
            lines[g + 6]
        );
        // estilos: nombre claude-soft negrita, etiqueta gris, └ en subtle
        let y = (g + 1) as u16;
        let name = buf[(col(&lines[g + 1], "visual"), y)].style();
        assert_eq!(name.fg, Some(t.claude_soft));
        assert!(name.add_modifier.contains(Modifier::BOLD));
        let tag = buf[(col(&lines[g + 1], "[workflow]"), y)].style();
        assert_eq!(tag.fg, Some(t.ink_muted));
        let y = (g + 2) as u16;
        assert_eq!(buf[(col(&lines[g + 2], "└"), y)].style().fg, Some(t.subtle));
        assert_eq!(
            buf[(col(&lines[g + 2], "pintar"), y)].style().fg,
            Some(t.ink)
        );
        // la línea del agente es de su fila
        let hits = row_hits(&app);
        let at = |y: usize| hits.iter().find(|h| h.0 as usize == y).map(|h| h.1);
        assert_eq!(at(g + 1), Some(2), "{lines:#?}");
        assert_eq!(at(g + 5), Some(3), "{lines:#?}");
        // la terminada no cambia: una línea con nombre, etiqueta y tarea
        let d = blk(&lines, "  frontend [own]  ");
        assert!(lines[d].contains(". "), "{:?}", lines[d]);
        assert!(
            !lines[d - 1].trim_start().starts_with("frontend"),
            "{lines:#?}"
        );
    }

    /// La línea del agente es de la cabeza: pulsarla abre esa fila.
    #[test]
    fn the_agent_line_of_a_chain_clicks_its_head() {
        let mut app = chain_app();
        let lines = draw(&mut app, 110, 30);
        let a = pos(&lines, "docs [own] ×3");
        let h = blk(&lines, "Revisando convenciones");
        assert_eq!(h, a + 1);
        let hits = row_hits(&app);
        let at = |y: usize| hits.iter().find(|x| x.0 as usize == y).map(|x| x.1);
        assert_eq!(at(a), at(h), "{lines:#?}");
        assert!(at(a).is_some());
    }

    /// Estrecho: la tarea se corta con «…» y el árbol sigue en su sitio; bajo
    /// 50 columnas el agente se queda sin etiqueta ni ×N.
    #[test]
    fn a_chain_cuts_its_tasks_first_and_never_overflows() {
        let mut app = chain_app();
        // bajo 70 columnas se van ctx y tokens: la tarea aún cabe entera
        let lines = draw(&mut app, 60, 30);
        let h = blk(&lines, "├ Revisando");
        assert!(lines[h].contains("docs/README "), "{:?}", lines[h]);
        assert!(lines[h].ends_with("1 min"), "{:?}", lines[h]);
        assert!(lines.iter().any(|l| l == "         docs [own] ×3"));
        let lines = draw(&mut app, 45, 30);
        let h = blk(&lines, "├ Revisando");
        assert!(lines[h].contains('…'), "{:?}", lines[h]);
        assert!(lines[h].ends_with("1 min"), "{:?}", lines[h]);
        assert!(lines.iter().any(|l| l == "         docs"), "{lines:#?}");
        for w in 30u16..=160 {
            let lines = draw(&mut app, w, 30);
            assert!(
                lines[2..29].iter().all(|l| width(l) < w as usize),
                "{w}: {lines:#?}"
            );
        }
    }

    /// Las filas de activos van pegadas, sin blanco entre ellas (las
    /// sublíneas de una en curso tampoco lo llevan); solo hay aire antes de
    /// cada regla de grupo, salvo la primera, pegada a las columnas.
    #[test]
    fn active_rows_are_tight_with_air_only_before_each_group_rule() {
        let mut app = blocks();
        let lines = draw(&mut app, 110, 60);
        let g = pos(&lines, " station · 2 running · 1 recent ──");
        // fila 3 (agente encima) con skill y now, fila 4 (agente encima) con
        // now, fila 5 terminada: seguidas
        assert_eq!(lines[g + 1], "         visual [workflow]", "{lines:#?}");
        assert!(lines[g + 2].starts_with("    3. ✦ └ pintar"), "{lines:#?}");
        assert_eq!(lines[g + 3], "           skill  superpowers:brainstorming");
        assert_eq!(lines[g + 4], "           now    —");
        assert_eq!(lines[g + 5], "         backend [own]", "{lines:#?}");
        assert!(lines[g + 6].starts_with("    4. ✦ └ —"), "{lines:#?}");
        assert_eq!(lines[g + 7], "           now    —");
        assert!(lines[g + 8].starts_with("    5.   frontend"), "{lines:#?}");
        // un solo blanco, justo antes de la regla siguiente
        assert_eq!(lines[g + 9], "", "{lines:#?}");
        let o = pos(&lines, " otro · 1 running ──");
        assert_eq!(o, g + 10, "{lines:#?}");
        // dentro de un grupo, ni un blanco
        assert!(lines[g..o - 1].iter().all(|l| !l.is_empty()), "{lines:#?}");
        // la primera regla, justo bajo la cabecera de columnas
        assert!(lines[3].contains("──"), "{lines:#?}");
    }

    #[test]
    fn a_run_in_progress_is_a_row_with_its_task_and_skill_and_now_under_it() {
        let mut app = blocks();
        let lines = draw(&mut app, 110, 60);
        // cabecera de columnas una vez: «agent» sobre el nombre y, a la
        // derecha, ctx, time y tokens (la última); ni task ni tools
        let head = &lines[2];
        assert!(head.starts_with("         agent  "), "{head:?}");
        assert!(head.ends_with("ctx        time   tokens"), "{head:?}");
        assert!(
            !head.contains("tools") && !head.contains("task"),
            "{head:?}"
        );
        assert_eq!(lines.iter().filter(|l| l.contains("agent  ")).count(), 1);
        // la regla: carpeta, cuántas en curso, cuántas recientes y raya hasta el borde
        let g = pos(&lines, " station · 2 running · 1 recent ──");
        assert_eq!(end_of(&lines[g]), 109);
        let o = pos(&lines, " otro · 1 running ──");
        assert!(!lines[o].contains("recent"), "{:?}", lines[o]);
        // ni rutas ni cwd en la lista (siguen en la ficha)
        assert!(
            !lines.iter().any(|l| l.contains("~/") || l.contains("cwd")),
            "{lines:#?}"
        );
        // en curso y sola, un racimo de uno: el agente en su línea y la
        // fila, tras el número, con la tarea colgada de └
        assert_eq!(lines[g + 1], "         visual [workflow]");
        assert!(
            lines[g + 2].starts_with("    3. ✦ └ pintar la pantalla de activos  "),
            "{:?}",
            lines[g + 2]
        );
        assert!(
            lines[g + 2].ends_with("—      <1 min        —"),
            "{:?}",
            lines[g + 2]
        );
        // con skill, tres líneas: skill y now en la columna del nombre, tras
        // la guía (dos espacios en una sola)
        assert_eq!(lines[g + 3], "           skill  superpowers:brainstorming");
        assert_eq!(lines[g + 4], "           now    —");
        // sin tarea, «—»; sin skill, dos líneas
        assert_eq!(lines[g + 5], "         backend [own]");
        assert!(
            lines[g + 6].starts_with("    4. ✦ └ —  "),
            "{:?}",
            lines[g + 6]
        );
        assert!(
            lines[g + 6].ends_with("—      10 min        —"),
            "{:?}",
            lines[g + 6]
        );
        assert_eq!(lines[g + 7], "           now    —");
        // sin el nombre delante, la tarea larga cabe entera antes de las
        // cifras (el corte con … lo cubre el racimo estrecho)
        let gp = pos(&lines, "general-purpose [claude]") + 1;
        assert!(
            lines[gp]
                .contains("✦ └ Revisión independiente del cambio sin commitear en /Users/x/otro  "),
            "{:?}",
            lines[gp]
        );
        assert!(
            lines[gp].ends_with("—       1 min        —"),
            "{:?}",
            lines[gp]
        );
        assert_eq!(lines[gp + 1], "           now    Bash · git diff --stat");
        for (l, time) in [
            (&lines[g + 2], "<1 min"),
            (&lines[g + 6], "10 min"),
            (&lines[gp], "1 min"),
        ] {
            // «tokens» es la última columna, pegada al borde; «time» justo antes
            assert_eq!(end_of(l), end_col(head, "tokens"), "{l:?}");
            assert_eq!(end_col(l, time), end_col(head, "time"), "{l:?}");
        }
    }

    #[test]
    fn the_name_style_depends_on_the_state_and_the_tag_is_muted() {
        let t = Theme::claude_256();
        let mut app = blocks();
        let lines = draw(&mut app, 110, 40);
        let buf = draw_buf(&mut app, 110, 40);
        // en curso, el nombre y la etiqueta en la línea del agente
        let (wf, be, gp) = (
            pos(&lines, "visual [workflow]"),
            run_of(&lines, "backend") - 1,
            pos(&lines, "general-purpose [claude]"),
        );
        // en curso, sea cual sea el tipo (paso de workflow, definido y built-in):
        // claude-soft negrita; la etiqueta, gris; la tarea, en ink
        for (y, name) in [(wf, "visual"), (be, "backend"), (gp, "general-purpose")] {
            let st = buf[(col(&lines[y], name), y as u16)].style();
            assert_eq!(st.fg, Some(t.claude_soft), "{name}");
            assert!(st.add_modifier.contains(Modifier::BOLD), "{name}");
        }
        let tag = buf[(col(&lines[wf], "[workflow]"), wf as u16)].style();
        assert_eq!(tag.fg, Some(t.ink_muted));
        let tag = buf[(col(&lines[gp], "[claude]"), gp as u16)].style();
        assert_eq!(tag.fg, Some(t.ink_muted));
        let task = buf[(col(&lines[wf + 1], "pintar"), (wf + 1) as u16)].style();
        assert_eq!(task.fg, Some(t.ink));
        // un definido no lleva etiqueta
        assert!(!lines[be].contains("built-in") && !lines[be].contains("via"));
        // terminadas, sea cual sea el tipo: nombre y tarea en gris, sin negrita
        let mut done = done_only();
        let dlines = draw(&mut done, 110, 22);
        let dbuf = draw_buf(&mut done, 110, 22);
        for name in ["backend", "visual"] {
            let d = blk(&dlines, name);
            let st = dbuf[(col(&dlines[d], name), d as u16)].style();
            assert_eq!(st.fg, Some(t.ink_muted), "{name}");
            assert!(!st.add_modifier.contains(Modifier::BOLD), "{name}");
        }
        let d = blk(&dlines, "backend");
        let st = dbuf[(col(&dlines[d], "backend: tarea"), d as u16)].style();
        assert_eq!(st.fg, Some(t.ink_muted));
        // el tiempo, en gris también en curso
        let y = (wf + 1) as u16;
        assert_eq!(
            buf[(col(&lines[wf + 1], "<1 min"), y)].style().fg,
            Some(t.ink_muted)
        );
        // sin fondo en ninguna celda de la lista, tampoco en la seleccionada
        for y in 3..22 {
            for x in 0..110 {
                assert!(
                    matches!(
                        buf[(x, y)].style().bg,
                        None | Some(ratatui::style::Color::Reset)
                    ),
                    "({x},{y})"
                );
            }
        }
    }

    #[test]
    fn group_and_sub_lines_are_painted_in_soft_muted_and_subtle() {
        let t = Theme::claude_256();
        let mut app = blocks();
        let lines = draw(&mut app, 110, 60);
        let buf = draw_buf(&mut app, 110, 60);
        let g = pos(&lines, " station · 2 running") as u16;
        let at = |y: usize, pat: &str| buf[(col(&lines[y], pat), y as u16)].style().fg;
        let gy = g as usize;
        assert_eq!(at(gy, "station"), Some(t.claude_soft));
        assert_eq!(at(gy, "· 2"), Some(t.ink_muted));
        assert_eq!(at(gy, "running"), Some(t.ink_muted));
        assert_eq!(at(gy, "· 1"), Some(t.ink_muted));
        assert_eq!(at(gy, "recent"), Some(t.ink_muted));
        assert_eq!(at(gy, "─"), Some(t.subtle));
        assert_eq!(at(2, "agent"), Some(t.ink_muted));
        assert_eq!(at(2, "time"), Some(t.ink_muted));
        // la etiqueta de la sublínea, apagada; skill y now, en gris
        assert_eq!(at(gy + 3, "skill"), Some(t.subtle));
        assert_eq!(at(gy + 3, "superpowers"), Some(t.ink_muted));
        assert_eq!(at(gy + 4, "now"), Some(t.subtle));
        // sin herramienta, el guion en gris
        assert_eq!(at(gy + 4, "—"), Some(t.ink_muted));
        assert_eq!(at(gy + 7, "—"), Some(t.ink_muted));
        let gp = pos(&lines, "general-purpose [claude]");
        assert_eq!(at(gp + 2, "Bash"), Some(t.ink_muted));
    }

    #[test]
    fn a_long_skill_and_last_tool_are_cut_with_an_ellipsis_on_one_line() {
        let mut r = run("docs", STATION, "2026-11-05T11:55:00Z", 3, None, false);
        r.skill = Some("superpowers:una-skill-con-un-nombre-larguísimo-de-verdad".into());
        r.last_tool = Some("Bash · git log --oneline --graph --decorate --all".into());
        let mut app = with_runs(fixture(), vec![r]);
        press(&mut app, KeyCode::Char('a'));
        let lines = draw(&mut app, 60, 60);
        assert!(lines.iter().all(|l| width(l) <= 59), "{lines:#?}");
        for pat in ["superpowers:una", "Bash · git log"] {
            let l = lines.iter().find(|l| l.contains(pat)).unwrap();
            assert_eq!(width(l), 59, "{l:?}");
            assert!(l.ends_with('…'), "{l:?}");
        }
        // una herramienta de varias líneas va en una
        let mut r = run("docs", STATION, "2026-11-05T11:55:00Z", 3, None, false);
        r.last_tool = Some("Edit\n README.md".into());
        let mut app = with_runs(fixture(), vec![r]);
        press(&mut app, KeyCode::Char('a'));
        let lines = draw(&mut app, 110, 60);
        let i = pos(&lines, "Edit README.md");
        assert_eq!(lines[i], "           now    Edit README.md");
    }

    #[test]
    fn under_70_columns_ctx_and_tokens_go_and_under_50_the_tag_too() {
        let mut app = blocks();
        let lines = draw(&mut app, 69, 60);
        let (wf, gp) = (
            pos(&lines, "visual [workflow]"),
            pos(&lines, "general-purpose"),
        );
        assert!(!lines[2].contains("tools"), "{:?}", lines[2]);
        assert!(lines[2].ends_with("time"), "{:?}", lines[2]);
        assert!(lines[wf + 1].ends_with("<1 min"), "{:?}", lines[wf + 1]);
        assert!(lines[gp].contains("[claude]"), "{:?}", lines[gp]);
        assert!(lines[gp + 1].ends_with("1 min"), "{:?}", lines[gp + 1]);
        let lines = draw(&mut app, 49, 60);
        assert!(!lines
            .iter()
            .any(|l| l.contains("[workflow]") || l.contains("[claude]") || l.contains("[own]")));
        let wf = run_of(&lines, "visual");
        assert_eq!(lines[wf - 1], "         visual", "{lines:#?}");
        assert!(lines[wf].starts_with("    3. ✦ └ "), "{:?}", lines[wf]);
        assert!(lines[wf].ends_with("<1 min"), "{:?}", lines[wf]);
        assert!(lines.iter().any(|l| l == "         general-purpose"));
    }

    #[test]
    fn the_tagged_rows_never_overflow_from_30_to_160_columns() {
        for mut app in [blocks(), busy(), done_only()] {
            for w in 30u16..=160 {
                let lines = draw(&mut app, w, 60);
                assert!(
                    lines[2..59].iter().all(|l| width(l) < w as usize),
                    "{w}: {lines:#?}"
                );
            }
        }
    }

    #[test]
    fn the_blocks_never_overflow_at_any_width() {
        for mut app in [blocks(), busy(), done_only()] {
            for w in [30u16, 36, 40, 49, 50, 60, 69, 70, 71, 85, 110, 160] {
                let lines = draw(&mut app, w, 60);
                assert!(
                    lines[2..59].iter().all(|l| width(l) < w as usize),
                    "{w}: {lines:#?}"
                );
            }
        }
    }

    #[test]
    fn a_long_name_is_cut_on_its_own_line_and_the_figures_stay() {
        let mut r = run(
            "workflow-subagent",
            STATION,
            "2026-11-05T11:55:00Z",
            3,
            None,
            false,
        );
        r.description = Some("un-agente-con-un-nombre-larguisimo-de-verdad".into());
        let mut app = with_runs(fixture(), vec![r]);
        press(&mut app, KeyCode::Char('a'));
        // en curso y sola, el nombre va en su línea, encima de la fila
        let lines = draw(&mut app, 60, 40);
        let a = pos(&lines, "un-agente-con");
        assert_eq!(
            lines[a], "         un-agente-con-un-nombre-larguisimo-de-verdad",
            "sin sitio para la etiqueta, el nombre entero"
        );
        assert!(
            lines[a + 1].ends_with("└ —                                          5 min"),
            "{lines:#?}"
        );
        // estrecho, el nombre se corta con … y nada desborda
        let lines = draw(&mut app, 40, 40);
        let a = pos(&lines, "un-agente-con");
        let l = &lines[a];
        assert!(l.ends_with('…'), "{l:?}");
        assert!(!l.contains("via"), "{l:?}");
        assert!(width(l) < 40, "{l:?}");
        assert!(lines[a + 1].ends_with("5 min"), "{lines:#?}");
    }

    #[test]
    fn live_groups_with_the_same_folder_carry_their_parent() {
        let mut app = with_runs(
            fixture(),
            vec![
                run("backend", STATION, "2026-11-05T11:50:00Z", 8, None, false),
                run(
                    "docs",
                    "/Users/x/Towerforge/station",
                    "2026-11-05T11:55:00Z",
                    4,
                    None,
                    false,
                ),
            ],
        );
        press(&mut app, KeyCode::Char('a'));
        let lines = draw(&mut app, 110, 40);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with(" argi/station · 1 running")),
            "{lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with(" Towerforge/station · 1 running")),
            "{lines:#?}"
        );
        // con una sola, solo la carpeta
        let mut one = busy();
        let lines = draw(&mut one, 110, 40);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with(" station · 2 running · 1 recent ─")),
            "{lines:#?}"
        );
    }

    /// Activos sin nada en curso: las llamadas del fixture (sin ejecución) y
    /// una ejecución terminada de backend (de 11:45 a 11:55).
    fn done_only() -> App {
        let mut app = with_runs(
            fixture(),
            vec![run(
                "backend",
                STATION,
                "2026-11-05T11:45:00Z",
                10,
                None,
                true,
            )],
        );
        press(&mut app, KeyCode::Char('a'));
        app
    }

    #[test]
    fn group_rules_count_running_and_recent_and_omit_the_zero_ones() {
        // solo terminadas
        let lines = draw(&mut done_only(), 110, 60);
        assert!(
            lines[3].starts_with(" taskflow · 2 recent ──"),
            "{:?}",
            lines[3]
        );
        assert!(lines
            .iter()
            .any(|l| l.starts_with(" station · 2 recent ──")));
        // singular
        assert!(lines.iter().any(|l| l.starts_with(" front · 1 recent ──")));
        assert!(lines.iter().all(|l| !l.contains("running ──")));
        // solo en curso
        let lines = draw(&mut busy(), 110, 60);
        let o = pos(&lines, " otro · 1 running ──");
        assert!(!lines[o].contains("recent"), "{:?}", lines[o]);
        // las dos
        let b = pos(&lines, " station · 2 running · 1 recent ──");
        assert_eq!(end_of(&lines[b]), 109);
    }

    #[test]
    fn a_finished_row_is_one_muted_line_without_star_with_its_task_and_ago() {
        let t = Theme::claude_256();
        let mut app = done_only();
        let lines = draw(&mut app, 110, 60);
        let buf = draw_buf(&mut app, 110, 60);
        let b = blk(&lines, "backend");
        let y = b as u16;
        // el hueco de la estrella en blanco: el nombre alinea con los demás,
        // y la tarea dos espacios detrás
        assert!(
            lines[b].starts_with("    4.   backend [own]  backend: tarea  "),
            "{:?}",
            lines[b]
        );
        assert!(
            lines[b].ends_with("—   5 min ago        —"),
            "{:?}",
            lines[b]
        );
        // una sola línea: ni skill, ni now/last, ni cwd debajo
        assert!(
            !lines
                .iter()
                .any(|l| l.contains("last ") || l.contains("now ") || l.contains("cwd")),
            "{lines:#?}"
        );
        assert!(!lines.iter().any(|l| l.contains('✦') || l.contains('✧')));
        // número, nombre, tarea y tiempo en gris, sin negrita ni naranja
        for pat in ["4.", "backend", "tarea", "5 min ago"] {
            let c = col(&lines[b], pat);
            let st = buf[(c, y)].style();
            assert_eq!(st.fg, Some(t.ink_muted), "{pat}");
            assert!(!st.add_modifier.contains(Modifier::BOLD), "{pat}");
        }
        // el cursor tiñe el número de su fila, también terminada; el nombre no
        let first = blk(&lines, "visual");
        assert!(lines[first].starts_with(" ❯  1. "), "{:?}", lines[first]);
        let st = buf[(col(&lines[first], "1."), first as u16)].style();
        assert_eq!(st.fg, Some(t.claude_soft));
        let st = buf[(col(&lines[first], "visual"), first as u16)].style();
        assert_eq!(st.fg, Some(t.ink_muted));
        assert!(!st.add_modifier.contains(Modifier::BOLD));
    }

    /// Lo último que hizo una terminada ya no sale en la lista (sigue en la
    /// ficha): la fila es una sola línea.
    #[test]
    fn a_finished_run_with_a_last_tool_is_still_one_line() {
        let mut r = run("backend", STATION, "2026-11-05T11:45:00Z", 10, None, true);
        r.last_tool = Some("Edit · src/a.rs".into());
        r.skill = Some("superpowers:brainstorming".into());
        let mut app = with_runs(fixture(), vec![r]);
        press(&mut app, KeyCode::Char('a'));
        let lines = draw(&mut app, 110, 60);
        let b = blk(&lines, "backend");
        assert!(!lines.iter().any(|l| l.contains("Edit · src/a.rs")));
        assert!(!lines.iter().any(|l| l.contains("brainstorming")));
        let next = &lines[b + 1];
        assert!(
            next.is_empty() || next.contains("──") || next.contains(". "),
            "{lines:#?}"
        );
    }

    #[test]
    fn a_call_without_a_run_says_dash_in_ctx_and_tokens() {
        let t = Theme::claude_256();
        let mut app = done_only();
        let lines = draw(&mut app, 110, 60);
        let buf = draw_buf(&mut app, 110, 60);
        let v = blk(&lines, "visual");
        assert!(
            lines[v].ends_with("—     5 s ago        —"),
            "{:?}",
            lines[v]
        );
        // los guiones de ctx y tokens, apagados; el tiempo, en gris
        let ctx_end = width(lines[v][..lines[v].rfind("5 s ago").unwrap()].trim_end());
        let ctx_x = ctx_end as u16 - 1;
        assert_eq!(buf[(ctx_x, v as u16)].style().fg, Some(t.subtle));
        assert_eq!(
            buf[(end_of(&lines[v]) as u16 - 1, v as u16)].style().fg,
            Some(t.subtle)
        );
        // cada guion cae bajo su cabecera, como las cifras
        let head = &lines[2];
        assert_eq!(ctx_end, end_col(head, "ctx"));
        assert_eq!(end_col(&lines[v], "5 s ago"), end_col(head, "time"));
        assert_eq!(end_of(&lines[v]), end_col(head, "tokens"));
    }

    #[test]
    fn every_row_has_its_origin_tag_after_the_name_and_names_align() {
        let t = Theme::claude_256();
        let mut app = busy();
        let lines = draw(&mut app, 110, 40);
        let buf = draw_buf(&mut app, 110, 40);
        // en curso, el nombre va en la línea de su agente, sobre la fila
        let (ex_a, be_a) = (
            pos(&lines, "Explore [claude]"),
            pos(&lines, " backend [own]"),
        );
        let (ex, be) = (ex_a + 1, be_a + 1);
        let fe = blk(&lines, "frontend");
        let wf = blk(&lines, "visual");
        assert_eq!(lines[ex_a], "         Explore [claude]", "{lines:#?}");
        assert!(lines[ex].starts_with("    3. ✦ └ "), "{:?}", lines[ex]);
        // texto por tipo: [propio] en definidos y pasos de workflow, [claude] en built-in
        assert_eq!(lines[be_a], "         backend [own]", "{lines:#?}");
        assert!(lines[fe].contains("frontend [own]"), "{:?}", lines[fe]);
        assert!(lines[wf].contains("visual [own]"));
        // ni `✻`, ni punto, ni la etiqueta vieja
        assert!(!lines
            .iter()
            .any(|l| l.contains(['✻', '●']) || l.contains("built-in")));
        // la estrella, en claude, solo en las filas en curso; en las terminadas, hueco en blanco
        let star_x = col(&lines[ex], "✦");
        assert_eq!(star_x, 7);
        for l in [ex, be] {
            assert_eq!(col(&lines[l], "✦"), star_x, "{:?}", lines[l]);
            assert_eq!(buf[(star_x, l as u16)].style().fg, Some(t.claude));
        }
        for l in [fe, wf] {
            assert!(!lines[l].contains(['✦', '✧']), "{:?}", lines[l]);
            assert_eq!(buf[(star_x, l as u16)].symbol(), " ");
            assert_eq!(buf[(star_x + 1, l as u16)].symbol(), " ");
        }
        // parpadea cada medio segundo en la lista
        app.now += TimeDelta::milliseconds(500);
        let lines2 = draw(&mut app, 110, 40);
        assert!(lines2[be].contains("✧ └"), "{:?}", lines2[be]);
        assert!(lines2[ex].contains("✧ └"), "{:?}", lines2[ex]);
        assert_eq!(lines2.iter().filter(|l| l.contains('✧')).count(), 3);
        // la etiqueta, en gris, tanto en curso como terminada
        for (l, pat) in [(ex_a, "[claude]"), (be_a, "[own]"), (fe, "[own]")] {
            let st = buf[(col(&lines[l], pat), l as u16)].style();
            assert_eq!(st.fg, Some(t.ink_muted), "{pat}");
        }
        // el nombre empieza justo tras el número; cabecera y detalles, en esa columna
        let name = col(&lines[ex_a], "Explore");
        assert_eq!(name, 9);
        assert_eq!(col(&lines[be_a], "backend"), name);
        // y el └ de una en curso, en esa misma columna
        assert_eq!(col(&lines[ex], "└"), name);
        assert_eq!(col(&lines[fe], "frontend"), name);
        assert_eq!(col(&lines[2], "agent"), name);
        // las sublíneas, tras la guía del racimo (dos columnas)
        assert_eq!(col(&lines[ex + 1], "now"), name + LIVE_GUIDE_W as u16);
        // un built-in terminado: misma etiqueta, nombre en gris sin negrita
        let mut r = run("Explore", STATION, "2026-11-05T11:55:00Z", 3, None, true);
        r.task = None;
        let mut app = with_runs(fixture(), vec![r]);
        press(&mut app, KeyCode::Char('a'));
        let lines = draw(&mut app, 110, 40);
        let buf = draw_buf(&mut app, 110, 40);
        let ex = pos(&lines, "Explore [claude]");
        let st = buf[(name, ex as u16)].style();
        assert_eq!(st.fg, Some(t.ink_muted));
        assert!(!st.add_modifier.contains(Modifier::BOLD));
        // «hace N min» en la columna de tiempo; tokens, la última
        assert!(lines[ex].ends_with(" ago        —"), "{:?}", lines[ex]);
    }

    /// Un uso de `total` tokens con `context` del último mensaje sobre 200k.
    fn usage(total: u64, context: Option<u64>) -> crate::runs::usage::Usage {
        crate::runs::usage::Usage {
            tokens: crate::runs::usage::Tokens {
                input: total - total / 4,
                output: total / 4,
                cache_read: 0,
                cache_creation: 0,
            },
            model: Some("claude-sonnet-5-5".into()),
            context,
            context_window: 200_000,
        }
    }

    /// `blocks()` con tokens: general-purpose 1.1M al 92 %, backend 412k al
    /// 71 % y el workflow sin uso aún.
    fn metered() -> App {
        let mut runs = blocks_runs();
        for r in &mut runs {
            match r.agent_type.as_str() {
                "general-purpose" => r.usage = usage(1_100_000, Some(184_000)),
                "backend" => r.usage = usage(412_000, Some(142_000)),
                _ => {}
            }
        }
        let mut app = with_runs(fixture(), runs);
        press(&mut app, KeyCode::Char('a'));
        app
    }

    #[test]
    fn active_rows_carry_ctx_then_time_then_tokens_at_the_edge() {
        let t = Theme::claude_256();
        let mut app = metered();
        let lines = draw(&mut app, 110, 60);
        let buf = draw_buf(&mut app, 110, 60);
        let head = &lines[2];
        assert!(head.ends_with("ctx        time   tokens"), "{head:?}");
        // ctx en ocho pasos (cada octavo empezado llena uno) y el porcentaje
        let gp = pos(&lines, "general-purpose [claude]") + 1;
        assert!(
            lines[gp].ends_with("▰▰▰▰▰▰▰▰ 92%       1 min     1.1M"),
            "{:?}",
            lines[gp]
        );
        let be = run_of(&lines, "backend");
        assert!(
            lines[be].ends_with("▰▰▰▰▰▰▱▱ 71%      10 min     412k"),
            "{:?}",
            lines[be]
        );
        let wf = pos(&lines, "visual [workflow]") + 1;
        assert!(
            lines[wf].ends_with("—      <1 min        —"),
            "{:?}",
            lines[wf]
        );
        for (l, pat) in [(gp, "%"), (be, "%"), (wf, "—      <1")] {
            let end = end_col(&lines[l], pat) - width(pat) + 1;
            assert_eq!(end, end_col(head, "ctx"), "{:?}", lines[l]);
        }
        for l in [gp, be, wf] {
            assert_eq!(end_of(&lines[l]), end_col(head, "tokens"), "{:?}", lines[l]);
        }
        let at = |y: usize, pat: &str| buf[(col(&lines[y], pat), y as u16)].style().fg;
        // desde el 85 %, ámbar; por debajo, la barra en claude y lo demás en gris
        assert_eq!(at(gp, "▰"), Some(t.warning));
        assert_eq!(at(gp, "92%"), Some(t.warning));
        assert_eq!(at(be, "▰"), Some(t.claude));
        assert_eq!(at(be, "▱"), Some(t.subtle));
        assert_eq!(at(be, "71%"), Some(t.ink_muted));
        assert_eq!(at(be, "10 min"), Some(t.ink_muted));
        assert_eq!(at(be, "412k"), Some(t.ink_muted));
        // sin tokens, el guion apagado
        let x = end_of(&lines[wf]) as u16 - 1;
        assert_eq!(buf[(x, wf as u16)].style().fg, Some(t.subtle));
        // la cabecera suma los tokens que se ven
        assert!(
            lines[0].ends_with("· 4 sessions · 1.5M tok"),
            "{:?}",
            lines[0]
        );
        // por debajo de 70 columnas, ctx y tokens se van; time se queda
        let lines = draw(&mut app, 69, 60);
        assert!(lines[2].ends_with("time"), "{:?}", lines[2]);
        assert!(!lines.iter().any(|l| l.contains('▰') || l.contains("1.1M")));
    }

    #[test]
    fn the_time_column_aligns_in_progress_and_finished_under_one_header() {
        let mut app = busy();
        let lines = draw(&mut app, 110, 40);
        let head = end_col(&lines[2], "time");
        for (name, pat) in [
            ("frontend", "2 min ago"),
            ("backend", "10 min"),
            ("visual", "5 s ago"),
        ] {
            let l = blk(&lines, name);
            assert_eq!(end_col(&lines[l], pat), head, "{pat}");
        }
    }

    #[test]
    fn active_header_counts_in_progress_window_and_sessions() {
        let lines = draw(&mut busy(), 120, 22);
        assert!(
            lines[0].ends_with("window 10 min · 3 running · 4 in window · 4 sessions"),
            "{:?}",
            lines[0]
        );
        let lines = draw(&mut busy(), 80, 22);
        assert!(lines[0].contains("active"), "{:?}", lines[0]);
        assert!(width(&lines[0]) <= 80, "{:?}", lines[0]);
        // en todos, agentes y sesiones, sin activos
        let lines = draw(&mut fixture(), 110, 14);
        assert!(
            lines[0].ends_with("· 5 agents · 4 sessions"),
            "{:?}",
            lines[0]
        );
    }

    #[test]
    fn active_footer_has_no_order_and_paints_keys_claude_and_actions_ink() {
        let t = Theme::claude_256();
        let mut app = busy();
        let lines = draw(&mut app, 100, 60);
        assert_eq!(
            bare(&lines[59]),
            " ↑↓ move · enter details · tab mode · / filter · q quit"
        );
        // enter abre la ficha de cualquier ejecución: nunca va apagada
        let buf = draw_buf(&mut app, 100, 60);
        let x = col(&lines[59], "enter");
        assert_eq!(buf[(x, 59)].style().fg, Some(t.claude));
        assert_eq!(buf[(x + 6, 59)].style().fg, Some(t.ink));
    }

    /// La ficha de una ejecución con 38 herramientas y 5 pasos, abierta sobre
    /// la ejecución de `backend` (con agente definido) de `busy()`.
    fn run_app() -> App {
        let mut app = busy();
        // la primera ejecución en curso con agente definido
        for n in ['1', '2', '3'] {
            press(&mut app, KeyCode::Char(n));
            press(&mut app, KeyCode::Enter);
            if app.can_open_agent() {
                break;
            }
            press(&mut app, KeyCode::Esc);
        }
        let at = |s: &str| -> DateTime<Utc> { s.parse().unwrap() };
        let step = |when: &str, name: &str, arg: &str| crate::runs::ToolStep {
            at: at(when),
            name: name.into(),
            arg: Some(arg.into()),
        };
        let d = app.run_detail.as_mut().unwrap();
        d.name = "visual".into();
        d.kind = RunKind::Workflow;
        d.cwd = Some(PathBuf::from(STATION));
        d.worked = TimeDelta::minutes(4);
        d.tools = 5;
        d.ended = None;
        d.task_full = Some("pintar la pantalla de activos\ncon el diseño B".into());
        d.tool_counts = vec![("Read".into(), 3), ("Bash".into(), 1), ("Edit".into(), 1)];
        d.steps = vec![
            step("2026-11-05T11:56:00Z", "Read", "src/a.rs"),
            step("2026-11-05T11:57:00Z", "Read", "src/b.rs"),
            step("2026-11-05T11:58:00Z", "Edit", "src/c.rs"),
            step("2026-11-05T11:59:00Z", "Read", "src/d.rs"),
            step("2026-11-05T11:59:55Z", "Bash", "git diff --stat"),
        ];
        app
    }

    /// Una hija de la ficha, para «launched».
    fn link(
        name: &str,
        running: bool,
        mins: i64,
        tools: usize,
        tokens: u64,
        task: &str,
    ) -> RunLink {
        let at = |s: &str| -> DateTime<Utc> { s.parse().unwrap() };
        RunLink {
            id: format!("toolu_{name}"),
            name: name.into(),
            kind: RunKind::BuiltIn,
            owner: None,
            running,
            since: Some(at("2026-11-05T11:50:00Z")),
            ended: (!running).then(|| at("2026-11-05T11:56:00Z")),
            worked: TimeDelta::minutes(mins),
            tools,
            tokens,
            task: Some(task.into()),
        }
    }

    /// `run_app()` con uso (412k en sonnet-5-5, contexto al 71 %), lanzada por
    /// la sesión y con dos hijas: Explore en curso y general-purpose terminada.
    fn rich_run() -> App {
        let mut app = run_app();
        let d = app.run_detail.as_mut().unwrap();
        d.usage = crate::runs::usage::Usage {
            tokens: crate::runs::usage::Tokens {
                input: 18_000,
                output: 24_000,
                cache_read: 351_000,
                cache_creation: 19_000,
            },
            model: Some("claude-sonnet-5-5".into()),
            context: Some(142_000),
            context_window: 200_000,
        };
        d.tokens = 412_000;
        d.context = Some(Context {
            used: 142_000,
            window: 200_000,
        });
        d.launched_by = Some(LaunchedBy::Session {
            id: "23dabecb-2007-46ae-9152-b3b2ccdc4b59".into(),
            prompt: Some("paint the active screen with tokens".into()),
        });
        d.launched = vec![
            link("Explore", true, 2, 9, 64_000, "Find where ctx is computed"),
            link(
                "general-purpose",
                false,
                3,
                21,
                210_000,
                "Check ratatui gauge widths",
            ),
        ];
        app
    }

    #[test]
    fn run_details_show_the_launcher_usage_and_launched_runs_in_order() {
        let mut app = rich_run();
        let lines = draw(&mut app, 100, 40);
        assert!(
            lines[2].ends_with("running 4 min · 5 tools · 412k tok"),
            "{:?}",
            lines[2]
        );
        assert_eq!(
            lines[3],
            "   launched by  session 23dabecb · “paint the active screen with tokens”"
        );
        assert_eq!(lines[4], "");
        assert!(lines[5].starts_with(" task ─"), "{:?}", lines[5]);
        assert!(lines[8].starts_with(" usage ─"), "{:?}", lines[8]);
        assert!(lines[8].ends_with("─ sonnet-5-5"), "{:?}", lines[8]);
        assert!(
            lines[9].starts_with("   context       71%  ███"),
            "{:?}",
            lines[9]
        );
        assert!(lines[9].ends_with("░░  142k of 200k"), "{:?}", lines[9]);
        // la barra: 40 columnas como mucho, el 71 % lleno
        let gauge: String = lines[9]
            .chars()
            .filter(|c| "█▉▊▋▌▍▎▏░".contains(*c))
            .collect();
        assert_eq!(gauge.chars().count(), 40, "{:?}", lines[9]);
        assert_eq!(gauge.chars().filter(|&c| c == '█').count(), 28);
        assert_eq!(lines[10], "   input         18k");
        assert_eq!(lines[11], "   output        24k");
        assert_eq!(lines[12], "   cache read   351k");
        assert_eq!(lines[13], "   cache write   19k");
        // ningún coste en dólares: va por suscripción y la cifra no es lo que paga
        assert!(
            !lines
                .iter()
                .any(|l| l.contains('$') || l.contains("API prices")),
            "{lines:?}"
        );
        assert!(lines[14].starts_with(" tools ─"), "{:?}", lines[14]);
        assert!(lines[16].starts_with(" launched · 2 ─"), "{:?}", lines[16]);
        assert_eq!(
            lines[17],
            "   1. ✦ Explore [claude]           2 min · 9 tools · 64k tok         Find where ctx is computed"
        );
        assert_eq!(
            lines[18],
            "   2.   general-purpose [claude]   4 min ago · 21 tools · 210k tok   Check ratatui gauge widths"
        );
        assert!(lines[19].starts_with(" steps · 5 ─"), "{:?}", lines[19]);
    }

    #[test]
    fn run_usage_and_launched_are_painted_by_role() {
        let t = Theme::claude_256();
        let mut app = rich_run();
        let lines = draw(&mut app, 100, 40);
        let buf = draw_buf(&mut app, 100, 40);
        let at = |y: usize, pat: &str| buf[(col(&lines[y], pat), y as u16)].style();
        assert_eq!(at(3, "launched by").fg, Some(t.ink_muted));
        assert_eq!(at(8, "usage").fg, Some(t.claude_soft));
        assert_eq!(at(8, "sonnet").fg, Some(t.ink_muted));
        assert_eq!(at(9, "context").fg, Some(t.ink_muted));
        assert_eq!(at(9, "71%").fg, Some(t.ink));
        assert_eq!(at(9, "█").fg, Some(t.claude_soft));
        assert_eq!(at(9, "░").fg, Some(t.subtle));
        assert_eq!(at(9, "142k").fg, Some(t.ink_muted));
        assert_eq!(at(12, "351k").fg, Some(t.ink));
        assert_eq!(at(13, "19k").fg, Some(t.ink));
        // hijas: en curso, estrella y nombre claude-soft negrita; terminada, ink
        assert_eq!(at(17, "✦").fg, Some(t.claude));
        assert_eq!(at(17, "Explore").fg, Some(t.claude_soft));
        assert!(at(17, "Explore").add_modifier.contains(Modifier::BOLD));
        assert_eq!(at(18, "general").fg, Some(t.ink));
        assert_eq!(at(18, "[claude]").fg, Some(t.ink_muted));
        assert_eq!(at(18, "4 min ago").fg, Some(t.ink_muted));
        assert_eq!(at(18, "Check").fg, Some(t.ink));
        // desde el 85 %, la barra y la cifra en ámbar
        app.run_detail.as_mut().unwrap().context = Some(Context {
            used: 180_000,
            window: 200_000,
        });
        let lines = draw(&mut app, 100, 40);
        let buf = draw_buf(&mut app, 100, 40);
        let at = |y: usize, pat: &str| buf[(col(&lines[y], pat), y as u16)].style();
        assert_eq!(at(9, "90%").fg, Some(t.warning));
        assert_eq!(at(9, "█").fg, Some(t.warning));
    }

    #[test]
    fn the_selected_launched_run_carries_the_cursor_and_the_footer_says_open() {
        let t = Theme::claude_256();
        let mut app = rich_run();
        let lines = draw(&mut app, 140, 40);
        assert_eq!(
            bare(&lines[39]),
            " esc back · ↑↓ other agent · tab launched · pgup/pgdn steps · a agent · q quit"
        );
        app.launched_sel = Some(1);
        let lines = draw(&mut app, 140, 40);
        assert!(
            lines[18].starts_with(" ❯ 2.   general-purpose"),
            "{:?}",
            lines[18]
        );
        assert!(lines[17].starts_with("   1. ✦ Explore"), "{:?}", lines[17]);
        let buf = draw_buf(&mut app, 140, 40);
        assert_eq!(buf[(1, 18)].style().fg, Some(t.claude));
        assert_eq!(buf[(3, 18)].style().fg, Some(t.claude_soft));
        let name = buf[(col(&lines[18], "general"), 18)].style();
        assert_eq!(name.fg, Some(t.claude_soft));
        assert!(name.add_modifier.contains(Modifier::BOLD));
        assert_eq!(
            bare(&lines[39]),
            " esc back · ↑↓ other agent · tab launched · enter open · pgup/pgdn steps · a agent · q quit"
        );
        // sin hijas, ni sección ni tab
        let d = app.run_detail.as_mut().unwrap();
        d.launched.clear();
        app.launched_sel = None;
        let lines = draw(&mut app, 140, 40);
        assert!(!lines.iter().any(|l| l.contains("launched ·")));
        assert!(!lines[39].contains("tab"), "{:?}", lines[39]);
    }

    #[test]
    fn launched_by_a_run_names_it_and_the_footer_offers_p() {
        let mut app = rich_run();
        let parent = link("backend", true, 10, 7, 0, "");
        app.run_detail.as_mut().unwrap().launched_by = Some(LaunchedBy::Run(RunLink {
            kind: RunKind::Defined,
            ..parent
        }));
        assert!(app.can_open_parent());
        let lines = draw(&mut app, 140, 40);
        assert_eq!(lines[3], "   launched by  backend [own]");
        assert!(lines[39].contains(" · p parent · "), "{:?}", lines[39]);
        app.run_detail.as_mut().unwrap().launched_by = Some(LaunchedBy::GoneRun { id: "x".into() });
        let lines = draw(&mut app, 140, 40);
        assert_eq!(lines[3], "   launched by  a run no longer on disk");
        assert!(!lines[39].contains("parent"), "{:?}", lines[39]);
        // sin quién, ni línea: la sección del encargo sube
        app.run_detail.as_mut().unwrap().launched_by = None;
        let lines = draw(&mut app, 140, 40);
        assert_eq!(lines[3], "");
        assert!(lines[4].starts_with(" task"), "{:?}", lines[4]);
        // sesión sin prompt: solo el id corto
        app.run_detail.as_mut().unwrap().launched_by = Some(LaunchedBy::Session {
            id: "abc".into(),
            prompt: None,
        });
        assert_eq!(draw(&mut app, 140, 40)[3], "   launched by  session abc");
    }

    #[test]
    fn usage_has_no_cost_and_goes_without_a_known_model() {
        let mut app = rich_run();
        // en una terminal estrecha tampoco aparece el coste en su propia fila
        let lines = draw(&mut app, 40, 40);
        assert_eq!(lines[13], "   cache write   19k");
        assert!(lines[14].starts_with(" tools ─"), "{:?}", lines[14]);
        assert!(!lines
            .iter()
            .any(|l| l.contains('$') || l.contains("API prices")));
        // cifras pequeñas y sin modelo conocido
        let d = app.run_detail.as_mut().unwrap();
        d.usage.tokens = crate::runs::usage::Tokens {
            input: 10,
            output: 10,
            cache_read: 0,
            cache_creation: 0,
        };
        d.usage.model = None;
        let lines = draw(&mut app, 100, 40);
        assert!(!lines
            .iter()
            .any(|l| l.contains('$') || l.contains("API prices")));
        assert!(lines[8].ends_with("─ —"), "{:?}", lines[8]);
        // sin contexto, un guion
        app.run_detail.as_mut().unwrap().context = None;
        assert_eq!(draw(&mut app, 100, 40)[9], "   context       —");
    }

    #[test]
    fn a_run_from_the_history_cache_says_steps_are_not_kept() {
        let t = Theme::claude_256();
        let mut app = rich_run();
        let d = app.run_detail.as_mut().unwrap();
        d.from_cache = true;
        d.steps.clear();
        let lines = draw(&mut app, 100, 40);
        let i = pos(&lines, " steps ─");
        assert_eq!(
            lines[i + 1],
            "   from the history cache · steps are not kept"
        );
        let buf = draw_buf(&mut app, 100, 40);
        assert_eq!(buf[(3, i as u16 + 1)].style().fg, Some(t.ink_muted));
        assert!(!lines[39].contains("pgup"), "{:?}", lines[39]);
    }

    #[test]
    fn a_short_run_screen_drops_usage_and_launched_before_the_task_and_steps() {
        let mut app = rich_run();
        for h in 1u16..=40 {
            for w in [30u16, 60, 100] {
                let lines = draw(&mut app, w, h);
                assert_eq!(lines.len(), h as usize);
                assert!(
                    lines.iter().all(|l| width(l) <= w as usize),
                    "{w}x{h}: {lines:#?}"
                );
            }
        }
        // 24 filas: los pasos se quedan con tres, el encargo entra y la
        // sección de uso, que no cabe entera, se va
        let lines = draw(&mut app, 100, 24);
        assert!(lines.iter().any(|l| l.starts_with(" task")), "{lines:#?}");
        assert!(
            lines.iter().any(|l| l.starts_with(" launched")),
            "{lines:#?}"
        );
        let steps = pos(&lines, " steps · 5");
        assert!(lines[steps + 3].contains("ago"), "{lines:#?}");
        // con alto de sobra, todo
        let lines = draw(&mut app, 100, 40);
        for s in [" task", " usage", " tools", " launched", " steps"] {
            assert!(lines.iter().any(|l| l.starts_with(s)), "{s}: {lines:#?}");
        }
    }

    #[test]
    fn run_header_has_the_name_by_state_the_origin_tag_and_the_project() {
        let t = Theme::claude_256();
        let mut app = run_app();
        let lines = draw(&mut app, 100, 24);
        let head = &lines[2];
        assert!(
            head.starts_with(" ✦ visual [own] via workflow · station"),
            "{head:?}"
        );
        assert!(head.ends_with("running 4 min · 5 tools"), "{head:?}");
        let buf = draw_buf(&mut app, 100, 24);
        // en curso: estrella en claude; nombre claude-soft negrita; etiqueta gris
        assert_eq!(buf[(1, 2)].symbol(), "✦");
        assert_eq!(buf[(1, 2)].style().fg, Some(t.claude));
        assert_eq!(buf[(3, 2)].style().fg, Some(t.claude_soft));
        assert!(buf[(3, 2)].style().add_modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(col(head, "[own]"), 2)].style().fg, Some(t.ink_muted));
        assert_eq!(buf[(col(head, "via"), 2)].style().fg, Some(t.ink_muted));
        // un built-in en curso: [claude], mismo nombre claude-soft negrita
        let d = app.run_detail.as_mut().unwrap();
        d.kind = RunKind::BuiltIn;
        d.name = "Explore".into();
        let lines = draw(&mut app, 100, 24);
        assert!(lines[2].starts_with(" ✦ Explore [claude] · station"));
        let buf = draw_buf(&mut app, 100, 24);
        assert_eq!(buf[(3, 2)].style().fg, Some(t.claude_soft));
        assert!(buf[(3, 2)].style().add_modifier.contains(Modifier::BOLD));
        // la estrella parpadea cada medio segundo
        app.now += TimeDelta::milliseconds(500);
        assert!(draw(&mut app, 100, 24)[2].starts_with(" ✧ Explore"));
        app.now -= TimeDelta::milliseconds(500);
        // un agente definido: [propio] sin «vía workflow»
        let d = app.run_detail.as_mut().unwrap();
        d.kind = RunKind::Defined;
        d.name = "backend".into();
        let lines = draw(&mut app, 100, 24);
        assert!(
            lines[2].starts_with(" ✦ backend [own] · station"),
            "{:?}",
            lines[2]
        );
    }

    #[test]
    fn an_ended_run_has_an_ink_name_and_says_when_it_ended_in_muted() {
        let t = Theme::claude_256();
        let mut app = run_app();
        app.run_detail.as_mut().unwrap().ended = Some("2026-11-05T11:58:00Z".parse().unwrap());
        let lines = draw(&mut app, 100, 24);
        let head = &lines[2];
        assert!(
            head.starts_with("   visual [own] via workflow · station"),
            "{head:?}"
        );
        assert!(head.ends_with("ended 2 min ago"), "{head:?}");
        assert!(!head.contains("running") && !head.contains(['✦', '✧', '●']));
        let buf = draw_buf(&mut app, 100, 24);
        assert_eq!(buf[(3, 2)].style().fg, Some(t.ink));
        assert!(!buf[(3, 2)].style().add_modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(col(head, "ended"), 2)].style().fg, Some(t.ink_muted));
        // un definido terminado: ink, ya no naranja; un built-in, igual
        for (kind, name) in [(RunKind::Defined, "backend"), (RunKind::BuiltIn, "Explore")] {
            let d = app.run_detail.as_mut().unwrap();
            d.kind = kind;
            d.name = name.into();
            let buf = draw_buf(&mut app, 100, 24);
            assert_eq!(buf[(3, 2)].style().fg, Some(t.ink), "{name}");
        }
    }

    #[test]
    fn run_header_drops_running_under_70_and_the_tag_under_50() {
        let mut app = run_app();
        let head = draw(&mut app, 69, 24)[2].clone();
        assert!(
            head.contains("via workflow") && !head.contains("running"),
            "{head:?}"
        );
        assert!(head.ends_with("5 tools"), "{head:?}");
        let head = draw(&mut app, 49, 24)[2].clone();
        assert!(
            !head.contains("via workflow") && head.contains("visual"),
            "{head:?}"
        );
        assert!(head.ends_with("5 tools"), "{head:?}");
        // el nombre se recorta antes que perder las cifras
        app.run_detail.as_mut().unwrap().name = "un-nombre-larguisimo-de-agente".into();
        let head = draw(&mut app, 30, 24)[2].clone();
        assert!(head.contains('…') && head.ends_with("5 tools"), "{head:?}");
    }

    #[test]
    fn run_sections_come_in_order_with_their_rules() {
        let mut app = run_app();
        let lines = draw(&mut app, 80, 24);
        assert_eq!(lines[3], "", "una línea en blanco tras la cabecera");
        assert!(lines[4].starts_with(" task ─"), "{:?}", lines[4]);
        assert_eq!(lines[5], "   pintar la pantalla de activos");
        assert_eq!(lines[6], "   con el diseño B");
        assert!(lines[7].starts_with(" tools ─"), "{:?}", lines[7]);
        assert_eq!(lines[8], "   Read 3 · Bash 1 · Edit 1");
        assert!(lines[9].starts_with(" steps · 5 ─"), "{:?}", lines[9]);
        assert_eq!(lines[10], "   5 s ago    Bash · git diff --stat");
        assert_eq!(lines[11], "   1 min ago  Read · src/d.rs");
        assert_eq!(lines[14], "   4 min ago  Read · src/a.rs");
        assert_eq!(lines[15], "", "el resto queda en blanco");
        assert_eq!(
            bare(&lines[23]),
            " esc back · ↑↓ other agent · pgup/pgdn steps · a agent · q quit"
        );
    }

    #[test]
    fn run_section_rules_are_soft_with_a_subtle_line_and_the_last_step_is_ink() {
        let t = Theme::claude_256();
        let mut app = run_app();
        let lines = draw(&mut app, 80, 24);
        let buf = draw_buf(&mut app, 80, 24);
        assert_eq!(buf[(1, 4)].style().fg, Some(t.claude_soft));
        assert_eq!(buf[(40, 4)].style().fg, Some(t.subtle));
        let first = &lines[10];
        assert_eq!(buf[(col(first, "Bash"), 10)].style().fg, Some(t.ink));
        assert_eq!(buf[(col(first, "ago"), 10)].style().fg, Some(t.ink_muted));
        assert_eq!(
            buf[(col(&lines[11], "Read"), 11)].style().fg,
            Some(t.ink_muted)
        );
        let tools = &lines[8];
        assert_eq!(buf[(col(tools, "Read"), 8)].style().fg, Some(t.ink));
        assert_eq!(buf[(col(tools, "3"), 8)].style().fg, Some(t.claude_soft));
    }

    #[test]
    fn a_missing_task_says_dash_and_a_long_one_is_cut_with_an_ellipsis() {
        let mut app = run_app();
        app.run_detail.as_mut().unwrap().task_full = None;
        let lines = draw(&mut app, 80, 24);
        assert_eq!(lines[5], "   —");
        let long: Vec<String> = (1..=12).map(|i| format!("línea {i}")).collect();
        app.run_detail.as_mut().unwrap().task_full = Some(long.join("\n"));
        let lines = draw(&mut app, 80, 24);
        // ocho líneas como mucho; la última lleva `…`
        assert_eq!(lines[5], "   línea 1");
        assert_eq!(lines[12], "   línea 8…");
        assert!(lines[13].starts_with(" tools"), "{:?}", lines[13]);
        // una línea larga se ajusta al ancho
        app.run_detail.as_mut().unwrap().task_full =
            Some("una dos tres cuatro cinco seis siete ocho".into());
        let lines = draw(&mut app, 24, 24);
        assert_eq!(lines[5], "   una dos tres cuatro");
        assert_eq!(lines[6], "   cinco seis siete");
    }

    #[test]
    fn tools_wrap_to_the_next_line_when_they_do_not_fit() {
        let mut app = run_app();
        app.run_detail.as_mut().unwrap().tool_counts = vec![
            ("Read".into(), 17),
            ("Bash".into(), 12),
            ("Edit".into(), 8),
            ("Grep".into(), 1),
        ];
        let lines = draw(&mut app, 30, 24);
        let i = lines.iter().position(|l| l.starts_with(" tools")).unwrap();
        assert_eq!(lines[i + 1], "   Read 17 · Bash 12 · Edit 8");
        assert_eq!(lines[i + 2], "   Grep 1");
    }

    #[test]
    fn steps_scroll_and_say_the_range_and_the_last_n_of_m() {
        let mut app = run_app();
        let d = app.run_detail.as_mut().unwrap();
        d.tools = 38;
        d.steps = (0..20)
            .map(|i| crate::runs::ToolStep {
                at: "2026-11-05T11:59:00Z".parse().unwrap(),
                name: "Read".into(),
                arg: Some(format!("f{i}")),
            })
            .collect();
        let lines = draw(&mut app, 80, 24);
        let rule = lines.iter().position(|l| l.contains("steps ·")).unwrap();
        assert!(
            lines[rule].starts_with(" steps · last 20 of 38 ─"),
            "{:?}",
            lines[rule]
        );
        let room = 23 - rule - 1;
        assert!(
            lines[rule].ends_with(&format!("1-{room}/20")),
            "{:?}",
            lines[rule]
        );
        assert_eq!(lines[rule + 1], "   1 min ago  Read · f19");
        press(&mut app, KeyCode::PageDown);
        let lines = draw(&mut app, 80, 24);
        // una página más abajo, acotada a la última página llena
        let off = room.min(20 - room);
        assert!(lines[rule].ends_with(&format!("{}-{}/20", off + 1, off + room)));
        assert_eq!(
            lines[rule + 1],
            format!("   1 min ago  Read · f{}", 19 - off)
        );
        // desplazado, ninguno va destacado
        let t = Theme::claude_256();
        let buf = draw_buf(&mut app, 80, 24);
        assert_eq!(buf[(18, rule as u16 + 1)].style().fg, Some(t.ink_muted));
    }

    #[test]
    fn a_short_screen_gives_the_steps_before_the_task_and_never_overflows() {
        let mut app = run_app();
        let sizes = [
            (80u16, 12u16),
            (80, 9),
            (80, 7),
            (80, 5),
            (80, 4),
            (80, 3),
            (20, 2),
            (10, 1),
        ];
        for (w, h) in sizes {
            let lines = draw(&mut app, w, h);
            assert_eq!(lines.len(), h as usize);
        }
        // 9 filas: 6 de cuerpo = cabecera, regla de pasos y 3 pasos; sin encargo
        let lines = draw(&mut app, 80, 9);
        assert!(lines[2].starts_with(" ✦ visual"));
        assert!(lines[3].starts_with(" steps · 5"), "{lines:?}");
        assert_eq!(lines[6], "   2 min ago  Edit · src/c.rs");
        assert!(!lines.iter().any(|l| l.starts_with(" task")));
        // con 12 filas caben el encargo y las herramientas
        let lines = draw(&mut app, 80, 12);
        assert!(lines.iter().any(|l| l.starts_with(" task")));
        assert!(lines.iter().any(|l| l.starts_with(" tools")));
        // con 10 filas, las herramientas ganan al encargo
        let lines = draw(&mut app, 80, 10);
        assert!(!lines.iter().any(|l| l.starts_with(" task")));
        assert!(lines.iter().any(|l| l.starts_with(" tools")));
    }

    #[test]
    fn the_run_footer_announces_a_only_when_the_agent_can_be_opened() {
        let t = Theme::claude_256();
        let mut app = run_app();
        assert!(app.can_open_agent());
        let lines = draw(&mut app, 100, 24);
        assert_eq!(
            bare(&lines[23]),
            " esc back · ↑↓ other agent · pgup/pgdn steps · a agent · q quit"
        );
        let buf = draw_buf(&mut app, 100, 24);
        let x = col(&lines[23], "pgup/pgdn");
        assert_eq!(buf[(x, 23)].style().fg, Some(t.claude));
        assert_eq!(buf[(x + 10, 23)].style().fg, Some(t.ink));
        // sin agente que abrir, `a` no sale
        app.run_detail.as_mut().unwrap().owner = None;
        let lines = draw(&mut app, 100, 24);
        assert_eq!(
            bare(&lines[23]),
            " esc back · ↑↓ other agent · pgup/pgdn steps · q quit"
        );
        // estrecho: se quitan teclas menores antes que `q`
        let lines = draw(&mut app, 30, 24);
        assert!(bare(&lines[23]).ends_with("q quit"), "{:?}", lines[23]);
        assert!(width(&lines[23]) <= 30);
    }

    #[test]
    fn an_empty_monitor_says_the_last_use_and_points_to_tab() {
        let t = Theme::claude_256();
        let mut app = fixture();
        app.now = "2026-11-06T12:00:00Z".parse().unwrap();
        press(&mut app, KeyCode::Char('a'));
        let lines = draw(&mut app, 100, 8);
        assert_eq!(lines[2], " nothing in the last 10 min");
        assert_eq!(
            lines[3],
            " last use: visual from taskflow 1 d ago · tab to see all"
        );
        let buf = draw_buf(&mut app, 100, 8);
        assert_eq!(buf[(1, 2)].style().fg, Some(t.ink));
        assert_eq!(buf[(1, 3)].style().fg, Some(t.ink_muted));
    }

    #[test]
    fn an_empty_monitor_without_any_use_says_so() {
        let mut app = App::new(10, PathBuf::from("/Users/x"), "0.1.0", app_now());
        let agents = (0..fixture().agent_count())
            .map(|i| fixture().agent(i).clone())
            .collect();
        app.update(Action::Msg(Msg::Agents(agents)));
        app.update(Action::Msg(Msg::Activity {
            calls: vec![],
            runs: vec![],
            sessions: 0,
        }));
        // arranca en activos: no hace falta pulsar `a`
        let lines = draw(&mut app, 100, 8);
        assert_eq!(lines[2], " nothing in the last 10 min");
        assert_eq!(lines[3], " no use recorded · tab to see all");
    }

    fn app_now() -> DateTime<Utc> {
        "2026-11-05T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn all_mode_has_no_strip_and_the_run_shows_by_its_star() {
        let mut app = with_runs(
            fixture(),
            vec![run(
                "backend",
                STATION,
                "2026-11-05T11:50:00Z",
                8,
                None,
                false,
            )],
        );
        let lines = draw(&mut app, 110, 22);
        assert!(lines[2].contains("worked"), "{:?}", lines[2]);
        assert!(!lines.iter().any(|l| l.contains("Running")));
        assert!(line_of(&lines, "backend").contains("backend ✦"));
    }

    #[test]
    fn unused_groups_fold_into_one_numbered_line_that_enter_opens() {
        let mut app = with_unused();
        let lines = draw(&mut app, 110, 20);
        let folded = lines
            .iter()
            .find(|l| l.contains("_argibox"))
            .expect("grupo plegado");
        assert!(folded.starts_with("    6. ▸ _argibox · "), "{folded:?}");
        assert!(folded.contains(" · 1 agent unused ──"), "{folded:?}");
        assert!(folded.ends_with('─'), "{folded:?}");
        app.cursor = 5;
        let lines = draw(&mut app, 110, 20);
        let folded = lines.iter().find(|l| l.contains("_argibox")).unwrap();
        assert!(folded.starts_with(" ❯  6. ▸ _argibox"), "{folded:?}");
        press(&mut app, KeyCode::Enter);
        let lines = draw(&mut app, 110, 20);
        let y = lines.iter().position(|l| l.contains("_argibox")).unwrap();
        assert!(lines[y].starts_with(" ❯  6. ▾ _argibox"), "{:?}", lines[y]);
        let row = &lines[y + 1];
        assert!(row.starts_with("    7. ai-engineer"), "{row:?}");
        assert!(row.contains(&"─".repeat(14)), "{row:?}");
        assert!(row.ends_with("never"), "{row:?}");
    }

    #[test]
    fn folded_lines_are_muted_and_their_rule_subtle() {
        let t = Theme::claude_256();
        let mut app = with_unused();
        let lines = draw(&mut app, 110, 20);
        let folded = lines.iter().find(|l| l.contains("_argibox")).unwrap();
        let y = y_of(&lines, folded);
        let buf = draw_buf(&mut app, 110, 20);
        assert_eq!(
            buf[(col(folded, "_argibox"), y)].style().fg,
            Some(t.ink_muted)
        );
        assert_eq!(buf[(col(folded, "─"), y)].style().fg, Some(t.subtle));
    }

    #[test]
    fn scrollbar_appears_only_when_the_list_does_not_fit() {
        let t = Theme::claude_256();
        // 11 líneas de lista en 11 de cuerpo: cabe, sin barra
        let buf = draw_buf(&mut fixture(), 100, 14);
        for y in 2..13 {
            assert_eq!(buf[(99, y)].symbol(), " ", "fila {y}");
        }
        // 11 líneas en 5: barra en la última columna, del alto del cuerpo
        let buf = draw_buf(&mut fixture(), 100, 8);
        assert_eq!(buf[(99, 2)].symbol(), "█");
        assert_eq!(buf[(99, 2)].style().fg, Some(t.claude_soft));
        assert_eq!(buf[(99, 6)].symbol(), "│");
        assert_eq!(buf[(99, 6)].style().fg, Some(t.subtle));
        assert_eq!(buf[(99, 1)].symbol(), " ", "nada sobre el cuerpo");
        assert_eq!(buf[(99, 7)].symbol(), " ", "nada en el pie");
        // la fila no pisa la columna de la barra
        let row = cells(&buf, 0, 4, 99);
        assert!(row.contains("visual "), "{row:?}");
        assert!(!row.contains('●'), "{row:?}");
        assert!(row.trim_end().ends_with("5 s ago"), "{row:?}");
    }

    #[test]
    fn footer_lists_the_keys_of_the_list() {
        let lines = draw(&mut fixture(), 100, 14);
        assert_eq!(
            bare(&lines[13]),
            " ↑↓ move · enter details · tab mode · / filter · o sort · q quit"
        );
    }

    #[test]
    fn footer_shows_the_position_when_the_list_does_not_fit() {
        let lines = draw(&mut fixture(), 100, 8);
        assert!(lines[7].ends_with("1-5/11  v0.1.0"), "{:?}", lines[7]);
        assert!(lines[7].contains("o sort"), "{:?}", lines[7]);
    }

    #[test]
    fn narrow_footer_drops_minor_keys_before_the_position() {
        let lines = draw(&mut fixture(), 60, 8);
        assert!(lines[7].ends_with(" 1-5/11  v0.1.0"), "{:?}", lines[7]);
        assert!(lines[7].starts_with(" ↑↓ move"), "{:?}", lines[7]);
        assert!(lines[7].contains("tab mode"), "{:?}", lines[7]);
        assert!(lines[7].contains("q quit"), "{:?}", lines[7]);
        assert!(!lines[7].contains("o sort"), "{:?}", lines[7]);
    }

    #[test]
    fn before_the_first_scan_it_says_so() {
        let mut app = App::new(
            10,
            PathBuf::from("/Users/x"),
            "0.1.0",
            "2026-11-05T12:00:00Z".parse().unwrap(),
        );
        let lines = draw(&mut app, 100, 8);
        assert!(lines[3].ends_with("✦ loading agents"), "{lines:#?}");
        app.update(Action::Msg(Msg::Agents(vec![])));
        let lines = draw(&mut app, 100, 8);
        assert!(
            lines[3].ends_with("✦ loading agents"),
            "sin la primera pasada del historial todo saldría como nunca: {lines:#?}"
        );
    }

    /// Una pantalla recién abierta que aún lee `~/.claude/projects`, con el
    /// reloj `ms` milisegundos después del arranque.
    fn loading(ms: i64) -> App {
        let start: chrono::DateTime<chrono::Utc> = "2026-11-05T12:00:00Z".parse().unwrap();
        let mut app = App::new(10, PathBuf::from("/Users/x"), "0.1.0", start);
        app.projects = PathBuf::from("/Users/x/.claude/projects");
        app.update(Action::Tick(start + TimeDelta::milliseconds(ms)));
        app
    }

    #[test]
    fn loading_is_centered_in_the_body() {
        let mut app = loading(1_200);
        // 13 filas: cabecera, segunda línea, 10 de cuerpo y el pie
        let lines = draw(&mut app, 60, 13);
        let star = "✦ loading agents";
        let path = "~/.claude/projects · 1.2 s";
        assert_eq!(
            lines[6],
            format!("{}{star}", " ".repeat((60 - width(star)) / 2))
        );
        assert_eq!(
            lines[7],
            format!("{}{path}", " ".repeat((60 - width(path)) / 2))
        );
        for (i, l) in lines.iter().enumerate().take(12).skip(2) {
            if i != 6 && i != 7 {
                assert_eq!(l, "", "fila {i}: {lines:#?}");
            }
        }
    }

    #[test]
    fn loading_star_blinks_every_half_second() {
        let lines = draw(&mut loading(1_200), 60, 13);
        assert!(lines[6].ends_with("✦ loading agents"), "{:?}", lines[6]);
        let lines = draw(&mut loading(1_700), 60, 13);
        assert!(lines[6].ends_with("✧ loading agents"), "{:?}", lines[6]);
        assert!(
            lines[7].ends_with("~/.claude/projects · 1.7 s"),
            "{:?}",
            lines[7]
        );
    }

    #[test]
    fn loading_time_from_a_minute_on_is_like_worked() {
        let lines = draw(&mut loading(125_000), 60, 13);
        assert!(
            lines[7].ends_with("~/.claude/projects · 2 min"),
            "{:?}",
            lines[7]
        );
    }

    #[test]
    fn loading_colors() {
        let t = Theme::claude_256();
        let mut app = loading(1_200);
        let buf = draw_buf(&mut app, 60, 13);
        let lines = draw(&mut app, 60, 13);
        let star = col(&lines[6], "✦");
        assert_eq!(buf[(star, 6)].style().fg, Some(t.claude));
        let text = col(&lines[6], "loading");
        for x in text..text + "loading agents".len() as u16 {
            assert_eq!(buf[(x, 6)].style().fg, Some(t.ink), "columna {x}");
        }
        let path = col(&lines[7], "~");
        let end = path + width("~/.claude/projects · 1.2 s") as u16;
        for x in path..end {
            assert_eq!(buf[(x, 7)].style().fg, Some(t.ink_muted), "columna {x}");
        }
    }

    #[test]
    fn loading_footer_only_quits() {
        let lines = draw(&mut loading(1_200), 60, 13);
        assert!(lines[12].starts_with(" q quit"), "{:?}", lines[12]);
        assert!(!lines[12].contains("move"), "{:?}", lines[12]);
        assert!(!lines[12].contains("tab"), "{:?}", lines[12]);
    }

    #[test]
    fn loading_narrow_truncates_without_overflowing() {
        // 20 columnas: el texto cabe, la ruta se corta y el tiempo se queda
        let lines = draw(&mut loading(1_200), 20, 13);
        assert!(lines[6].ends_with("✦ loading agents"), "{lines:#?}");
        assert!(lines[7].ends_with("… · 1.2 s"), "{lines:#?}");
        assert!(lines[7].contains("~/"), "{lines:#?}");
        // la cabecera no es de la carga: se miran las dos líneas del cuerpo
        for l in &lines[6..8] {
            assert!(width(l) <= 19, "{l:?} desborda");
        }
        // 12 columnas: el texto se corta con `…`; sin sitio para la ruta,
        // solo el tiempo
        let lines = draw(&mut loading(1_200), 12, 13);
        assert!(lines[6].trim_start().starts_with("✦ load"), "{lines:#?}");
        assert!(lines[6].ends_with('…'), "{lines:#?}");
        assert_eq!(lines[7].trim_start(), "1.2 s", "{lines:#?}");
        // 4 columnas: solo la estrella
        let lines = draw(&mut loading(1_200), 4, 13);
        assert_eq!(lines[6].trim(), "✦", "{lines:#?}");
    }

    #[test]
    fn loading_low_keeps_only_the_first_line() {
        // 4 filas: una sola de cuerpo
        let lines = draw(&mut loading(1_200), 60, 4);
        assert!(lines[2].ends_with("✦ loading agents"), "{lines:#?}");
        assert!(!lines.join("\n").contains("projects"), "{lines:#?}");
    }

    // --- ranking ---

    /// Columna (exclusiva) donde acaba el texto de `line`.
    fn end_of(line: &str) -> usize {
        width(line.trim_end())
    }

    /// Columna (exclusiva) donde acaba `pat` dentro de `line`.
    fn end_col(line: &str, pat: &str) -> usize {
        col(line, pat) as usize + width(pat)
    }

    #[test]
    fn ranking_lists_agents_by_worked_with_a_proportional_bar() {
        let mut app = worked();
        press(&mut app, KeyCode::Tab);
        let lines = draw(&mut app, 110, 14);
        assert!(lines[2].starts_with(" By time worked · "), "{:?}", lines[2]);
        assert!(
            lines[2].contains(" · 2 agents with work ─"),
            "{:?}",
            lines[2]
        );
        let docs = &lines[3];
        // prefijo 7 + frontend 8 + estrella 2 + 2 + station 7 + 2 = 28;
        // derecha 10 + 3 + 6 + 3 + 4 + 1 = 27 (sin tokens: «—» en tokens)
        let bar = 110 - 28 - 2 - 27;
        assert_eq!(
            *docs,
            format!(
                " ❯  1. docs        station  {}  1 h 12 min        —      2",
                "━".repeat(bar)
            )
        );
        let frontend = &lines[4];
        assert!(
            frontend.starts_with("    2. frontend    station  ━"),
            "{frontend:?}"
        );
        assert!(
            frontend.ends_with("       5 min        —      1"),
            "{frontend:?}"
        );
        assert_eq!(width(docs), width(frontend));
    }

    /// A 170, 110 y 80 columnas: el total acaba donde el texto de la regla
    /// en todas las filas y la barra del máximo llena el hueco entre el
    /// proyecto y el total.
    #[test]
    fn ranking_fills_the_width_and_ends_the_total_at_the_rule() {
        for w in [170u16, 110, 80] {
            let mut app = worked();
            press(&mut app, KeyCode::Tab);
            let lines = draw(&mut app, w, 14);
            let edge = end_of(&lines[2]);
            assert_eq!(edge, w as usize - 1, "{w}: {:?}", lines[2]);
            let (docs, frontend) = (&lines[3], &lines[4]);
            assert_eq!(
                end_col(docs, "1 h 12 min"),
                end_col(frontend, "5 min"),
                "{w}: {docs:?}"
            );
            for row in [docs, frontend] {
                assert_eq!(end_of(row), edge, "{w}: {row:?}");
                for gone in ["ago", "running"] {
                    assert!(!row.contains(gone), "{w}: {row:?}");
                }
            }
            // la barra del máximo: de tras el proyecto hasta dos antes del total
            let from = end_col(docs, "station") + 2;
            let to = col(docs, "1 h 12 min") as usize - 2;
            assert_eq!(col(docs, "━") as usize, from, "{w}: {docs:?}");
            assert_eq!(
                docs.chars().filter(|&c| c == RANK_FULL).count(),
                to - from,
                "{w}: {docs:?}"
            );
            assert!(to - from >= BAR_MIN, "{w}: {docs:?}");
        }
    }

    /// `worked()` con tokens, en el ranking: docs 72 min en 2 ejecuciones con
    /// 4.4M; frontend 5 min en 1 con 6.2M.
    fn rank_tokens() -> App {
        let mut d1 = run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true);
        d1.usage = usage(3_000_000, None);
        let mut d2 = run("docs", STATION, "2026-11-03T09:00:00Z", 22, None, true);
        d2.usage = usage(1_400_000, None);
        let mut f = run("frontend", STATION, "2026-11-04T10:00:00Z", 5, None, true);
        f.usage = usage(6_200_000, None);
        let mut app = with_runs(fixture(), vec![d1, d2, f]);
        press(&mut app, KeyCode::Tab);
        app
    }

    #[test]
    fn ranking_rows_carry_worked_tokens_and_runs_under_headers_in_the_rule() {
        let t = Theme::claude_256();
        let mut app = rank_tokens();
        let lines = draw(&mut app, 110, 14);
        let rule = &lines[2];
        assert!(
            rule.starts_with(" By time worked · 2 agents with work ─"),
            "{rule:?}"
        );
        assert!(rule.ends_with("─ worked   tokens   runs"), "{rule:?}");
        let (docs, fe) = (&lines[3], &lines[4]);
        assert!(
            docs.starts_with(" ❯  1. docs        station  ━"),
            "{docs:?}"
        );
        assert!(docs.ends_with("━  1 h 12 min     4.4M      2"), "{docs:?}");
        assert!(fe.ends_with("     5 min     6.2M      1"), "{fe:?}");
        // la cabecera «worked», no la del título
        let worked_end = rule.rfind("worked").map(|i| width(&rule[..i]) + 6).unwrap();
        for (row, worked) in [(docs, "1 h 12 min"), (fe, "5 min")] {
            assert_eq!(end_col(row, worked), worked_end, "{row:?}");
            assert_eq!(end_of(row), end_col(rule, "runs"), "{row:?}");
        }
        assert_eq!(end_col(docs, "4.4M"), end_col(rule, "tokens"));
        // la columna de la clave en ink y su cabecera en claude-soft; las otras, grises
        let buf = draw_buf(&mut app, 110, 14);
        let at = |y: usize, pat: &str| buf[(col(&lines[y], pat), y as u16)].style().fg;
        assert_eq!(at(3, "1 h 12 min"), Some(t.ink));
        assert_eq!(at(3, "4.4M"), Some(t.ink_muted));
        let worked_x = (worked_end - 6) as u16;
        assert_eq!(buf[(worked_x, 2)].style().fg, Some(t.claude_soft));
        assert_eq!(at(2, "tokens"), Some(t.ink_muted));
        assert_eq!(at(2, "By"), Some(t.claude));
        // la cabecera suma los tokens del periodo
        assert!(
            lines[0].ends_with("all time · 1 h 17 min in 3 runs · 10.6M tok"),
            "{:?}",
            lines[0]
        );
        // por tokens: frontend primero, con la barra entera, y su columna en ink
        press(&mut app, KeyCode::Char('s'));
        let lines = draw(&mut app, 110, 14);
        assert!(
            lines[2].starts_with(" By tokens · 2 agents"),
            "{:?}",
            lines[2]
        );
        assert!(lines[3].contains(" frontend "), "{:?}", lines[3]);
        let bars: Vec<usize> = [&lines[3], &lines[4]]
            .iter()
            .map(|l| {
                l.chars()
                    .filter(|&c| c == RANK_FULL || c == RANK_HALF)
                    .count()
            })
            .collect();
        assert!(bars[0] > bars[1] && bars[1] > 0, "{lines:#?}");
        let buf = draw_buf(&mut app, 110, 14);
        let at = |y: usize, pat: &str| buf[(col(&lines[y], pat), y as u16)].style().fg;
        assert_eq!(at(3, "6.2M"), Some(t.ink));
        assert_eq!(at(3, "5 min"), Some(t.ink_muted));
        let tokens_x = width(&lines[2][..lines[2].rfind("tokens").unwrap()]) as u16;
        assert_eq!(buf[(tokens_x, 2)].style().fg, Some(t.claude_soft));
        // por ejecuciones
        press(&mut app, KeyCode::Char('s'));
        let lines = draw(&mut app, 110, 14);
        assert!(
            lines[2].starts_with(" By runs · 2 agents"),
            "{:?}",
            lines[2]
        );
        assert!(lines[3].contains(" docs "), "{:?}", lines[3]);
    }

    #[test]
    fn narrow_ranking_drops_runs_then_tokens_before_the_bar() {
        let mut app = rank_tokens();
        let has = |lines: &[String], pat: &str| lines[2].contains(pat);
        let bar = |lines: &[String]| lines[3].contains(RANK_FULL);
        let lines = draw(&mut app, 65, 14);
        assert!(has(&lines, "runs") && has(&lines, "tokens") && bar(&lines));
        let lines = draw(&mut app, 64, 14);
        assert!(!has(&lines, "runs") && has(&lines, "tokens") && bar(&lines));
        assert!(lines[3].ends_with("1 h 12 min     4.4M"), "{:?}", lines[3]);
        let lines = draw(&mut app, 57, 14);
        assert!(!has(&lines, "tokens") && bar(&lines), "{lines:#?}");
        assert!(lines[3].ends_with("━  1 h 12 min"), "{:?}", lines[3]);
        let lines = draw(&mut app, 48, 14);
        assert!(!bar(&lines) && lines[3].ends_with("1 h 12 min"));
        // la columna de la clave no se va nunca: por tokens, se va «worked»
        press(&mut app, KeyCode::Char('s'));
        let lines = draw(&mut app, 57, 14);
        assert!(
            has(&lines, "tokens") && !has(&lines, "worked"),
            "{lines:#?}"
        );
        assert!(lines[3].ends_with("6.2M"), "{:?}", lines[3]);
        // por debajo de 40, nombre y proyecto ya no caben (como antes)
        for w in 40u16..=170 {
            let lines = draw(&mut app, w, 14);
            assert!(
                lines[2..5].iter().all(|l| width(l) < w as usize),
                "{w}: {lines:#?}"
            );
        }
    }

    #[test]
    fn the_ranking_says_how_many_runs_only_the_history_keeps() {
        let t = Theme::claude_256();
        let mut runs = vec![
            run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true),
            run("docs", STATION, "2026-10-01T09:00:00Z", 22, None, true),
            run("frontend", STATION, "2026-10-02T10:00:00Z", 5, None, true),
        ];
        runs[1].from_cache = true;
        runs[2].from_cache = true;
        let mut app = with_runs(fixture(), runs);
        press(&mut app, KeyCode::Tab);
        let lines = draw(&mut app, 110, 14);
        // tras las filas, una línea en blanco y la nota en gris
        assert_eq!(lines[5], "");
        assert_eq!(
            lines[6],
            " history kept: 2 runs older than Claude's cleanup"
        );
        let buf = draw_buf(&mut app, 110, 14);
        assert_eq!(buf[(2, 6)].style().fg, Some(t.ink_muted));
        // con la lista llena, la nota se queda en la última línea del cuerpo
        let lines = draw(&mut app, 110, 6);
        assert_eq!(
            lines[4],
            " history kept: 2 runs older than Claude's cleanup"
        );
        assert!(lines[3].contains("docs"), "{lines:#?}");
        // sin historial guardado, nada
        let lines = draw(&mut rank_tokens(), 110, 14);
        assert!(!lines.iter().any(|l| l.contains("history kept")));
    }

    /// docs terminado y backend en curso desde las 11:50.
    fn ranking_running() -> App {
        let mut app = with_runs(
            fixture(),
            vec![
                run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true),
                run("backend", STATION, "2026-11-05T11:50:00Z", 8, None, false),
            ],
        );
        press(&mut app, KeyCode::Tab);
        app
    }

    #[test]
    fn a_running_agent_in_the_ranking_carries_the_star_after_its_name() {
        let t = Theme::claude_256();
        let mut app = ranking_running();
        let lines = draw(&mut app, 110, 14);
        let text = lines.join("\n");
        let backend = find(&lines, "backend");
        let docs = find(&lines, "docs");
        assert!(
            backend.contains("backend ✦") || backend.contains("backend ✧"),
            "{text}"
        );
        assert!(!docs.contains('✦') && !docs.contains('✧'), "{docs:?}");
        assert!(!text.contains('●'), "{text}");
        for row in [backend, docs] {
            for gone in ["ago", "running"] {
                assert!(!row.contains(gone), "{row:?}");
            }
            assert_eq!(end_of(row), end_of(&lines[2]), "{row:?}");
        }
        assert_eq!(col(backend, "station"), col(docs, "station"), "{text}");
        assert_eq!(col(backend, "━"), col(docs, "━"), "{text}");
        assert!(backend.ends_with("10 min        —      1"), "{backend:?}");
        let buf = draw_buf(&mut app, 110, 14);
        let y = y_of(&lines, backend);
        assert_eq!(buf[(col(backend, "✦"), y)].style().fg, Some(t.claude));
    }

    #[test]
    fn ranking_total_respects_the_scrollbar_column() {
        let mut app = worked();
        press(&mut app, KeyCode::Tab);
        // cuerpo de 2 filas para 3 líneas: sale la barra de scroll en la 109
        let buf = draw_buf(&mut app, 110, 5);
        assert_ne!(buf[(109, 2)].symbol(), " ", "sin barra de scroll");
        let lines: Vec<String> = (0..5).map(|y| cells(&buf, 0, y, 109)).collect();
        let edge = end_of(&lines[2]);
        assert_eq!(edge, 108, "{:?}", lines[2]);
        assert!(
            lines[3].trim_end().ends_with("1 h 12 min        —      2"),
            "{:?}",
            lines[3]
        );
        assert_eq!(end_of(&lines[3]), edge, "{:?}", lines[3]);
    }

    #[test]
    fn narrow_ranking_drops_the_bar_and_keeps_the_total_at_the_edge() {
        let mut app = worked();
        press(&mut app, KeyCode::Tab);
        // 28 + 2 + 11 = 41 no deja las 8 de la barra a 46: se va
        let lines = draw(&mut app, 46, 14);
        let docs = &lines[3];
        assert!(!docs.contains([RANK_FULL, RANK_HALF]), "{docs:?}");
        assert!(docs.starts_with(" ❯  1. docs        station  "), "{docs:?}");
        assert!(docs.ends_with("1 h 12 min"), "{docs:?}");
        assert_eq!(end_of(docs), end_of(&lines[2]), "{docs:?}");
        assert_eq!(end_of(&lines[4]), end_of(&lines[2]), "{:?}", lines[4]);
    }

    #[test]
    fn ranking_names_are_only_cut_when_the_terminal_is_narrow() {
        let long = || {
            with_runs(
                fixture(),
                vec![
                    run(
                        "react-structure-checker",
                        "/Users/x/Offing/taskflow",
                        "2026-11-04T09:00:00Z",
                        8,
                        None,
                        true,
                    ),
                    run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true),
                ],
            )
        };
        let mut app = long();
        press(&mut app, KeyCode::Tab);
        let lines = draw(&mut app, 80, 14);
        assert!(
            lines[4].contains("react-structure-checker    global  "),
            "{:?}",
            lines[4]
        );
        assert!(lines[3].contains(RANK_FULL), "{:?}", lines[3]);
        let lines = draw(&mut app, 44, 14);
        assert!(lines[4].contains("react-str"), "{:?}", lines[4]);
        assert!(lines[4].contains("…"), "{:?}", lines[4]);
        assert_eq!(end_of(&lines[4]), end_of(&lines[2]), "{}", lines.join("\n"));
    }

    #[test]
    fn rank_bar_uses_half_cells_and_never_full_blocks() {
        let m = |v: i64| v as f64;
        assert_eq!(rank_bar(m(69), m(69), 24), "━".repeat(24));
        // lo que tenga algo se ve, aunque sea media celda
        assert_eq!(rank_bar(m(1), m(1000), 24), "╸");
        assert_eq!(rank_bar(m(0), m(10), 24), "");
        assert_eq!(rank_bar(m(5), m(10), 0), "");
        // 5/72 de 48 medias celdas: 3,33 → 3, una y media
        assert_eq!(rank_bar(m(5), m(72), 24), "━╸");
        // 55/69 de 48: 38,26 → 38, diecinueve enteras
        assert_eq!(rank_bar(m(55), m(69), 24), "━".repeat(19));
        for v in 1..=69 {
            let bar = rank_bar(m(v), m(69), 24);
            assert!(width(&bar) <= 24, "{v}: {bar:?}");
            assert!(
                !bar.chars().any(|c| c == '█' || EIGHTHS.contains(&c)),
                "{v}: {bar:?}"
            );
        }
    }

    /// Celdas que llenan el alto entero y se pegan a la fila de abajo.
    fn full_height(c: char) -> bool {
        c == '█' || EIGHTHS.contains(&c)
    }

    #[test]
    fn ranking_bars_do_not_touch_the_row_below() {
        let mut app = rank_tokens();
        for _ in 0..3 {
            let lines = draw(&mut app, 100, 14);
            for row in &lines[3..5] {
                assert!(row.contains(RANK_FULL), "{row:?}");
                assert!(!row.chars().any(full_height), "{row:?}");
            }
            press(&mut app, KeyCode::Char('s'));
        }
    }

    #[test]
    fn context_gauge_keeps_eighths() {
        let t = Theme::claude_256();
        let ctx = Context {
            used: 142_000,
            window: 200_000,
        };
        let gauge: String = context_gauge(ctx, 40, &t)
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(gauge.starts_with(&"█".repeat(28)), "{gauge:?}");
        assert!(!gauge.contains(RANK_FULL) && !gauge.contains(RANK_HALF));
    }

    const LONG_PROJECT: &str = "/Users/x/Offing/a-really-long-project-folder-name-for-tests";

    /// Un agente de un proyecto de nombre largo con 1 h de trabajo y docs
    /// con 50 min, en el ranking.
    fn long_project() -> App {
        let mut app = fixture();
        let mut agents: Vec<_> = (0..app.agent_count())
            .map(|i| app.agent(i).clone())
            .collect();
        agents.push(crate::agents::Agent {
            name: "kiosk".into(),
            path: PathBuf::from(format!("{LONG_PROJECT}/.claude/agents/kiosk.md")),
            scope: crate::agents::Scope::Project(PathBuf::from(LONG_PROJECT)),
            description: None,
        });
        app.update(Action::Msg(Msg::Agents(agents)));
        let mut app = with_runs(
            app,
            vec![
                run(
                    "kiosk",
                    LONG_PROJECT,
                    "2026-11-04T09:00:00Z",
                    60,
                    None,
                    true,
                ),
                run("docs", STATION, "2026-11-04T11:00:00Z", 50, None, true),
            ],
        );
        press(&mut app, KeyCode::Tab);
        app
    }

    /// Del 60 al 200: la fila no pasa del borde, las cifras acaban donde el
    /// texto de la regla, el proyecto se corta con «…» a 18 como mucho y el
    /// nombre sale entero.
    #[test]
    fn long_project_never_pushes_the_numbers_off() {
        let mut app = long_project();
        for w in [60u16, 80, 100, 140, 200] {
            let lines = draw(&mut app, w, 14);
            let text = lines.join("\n");
            let edge = end_of(&lines[2]);
            assert_eq!(edge, w as usize - 1, "{w}: {text}");
            let kiosk = line_of(&lines, "kiosk");
            assert!(width(kiosk) < w as usize, "{w}: {text}");
            assert_eq!(end_of(kiosk), edge, "{w}: {text}");
            // la cifra de la clave, bajo su cabecera
            let head = lines[2].rfind("worked").unwrap();
            assert_eq!(
                end_col(kiosk, "1 h 0 min"),
                width(&lines[2][..head]) + 6,
                "{w}: {text}"
            );
            assert!(kiosk.contains(". kiosk "), "{w}: {text}");
            if let Some(i) = kiosk.find("a-really") {
                let project = kiosk[i..].split(' ').next().unwrap();
                assert!(width(project) <= RANK_PROJECT_MAX, "{w}: {text}");
                assert!(project.ends_with('…'), "{w}: {text}");
                // station sale entero en la fila de docs, en la misma columna
                let docs = line_of(&lines, "docs");
                assert_eq!(col(docs, "station"), col(kiosk, "a-"), "{w}: {text}");
            } else {
                assert!(w <= 60, "{w}: el proyecto no sale: {text}");
            }
        }
        // estrecha, solo la clave: la línea acaba en ella
        let lines = draw(&mut app, 60, 14);
        assert!(
            line_of(&lines, "kiosk").ends_with("1 h 0 min"),
            "{lines:#?}"
        );
    }

    /// Del 60 al 200 y con cada clave: la barra va entera entre el proyecto y
    /// la primera cifra, con dos de aire a cada lado, en la misma columna en
    /// todas las filas.
    #[test]
    fn ranking_bars_stay_inside_their_slot_at_any_width() {
        for mut app in [long_project(), rank_tokens()] {
            for _ in 0..3 {
                for w in [60u16, 80, 100, 140, 200] {
                    let lines = draw(&mut app, w, 14);
                    let text = lines.join("\n");
                    let mut from = None;
                    for row in lines[3..5].iter() {
                        let Some(i) = row.find([RANK_FULL, RANK_HALF]) else {
                            continue;
                        };
                        let start = width(&row[..i]);
                        let bar: String = row[i..]
                            .chars()
                            .take_while(|&c| c == RANK_FULL || c == RANK_HALF)
                            .collect();
                        let rest = &row[i + bar.len()..];
                        // aire antes y después, y la primera cifra pegada a la derecha
                        assert!(row[..i].ends_with("  "), "{w}: {text}");
                        assert!(rest.starts_with("  "), "{w}: {text}");
                        assert!(!rest.contains([RANK_FULL, RANK_HALF]), "{w}: {text}");
                        assert_eq!(*from.get_or_insert(start), start, "{w}: {text}");
                        assert_eq!(end_of(row), end_of(&lines[2]), "{w}: {text}");
                    }
                }
                press(&mut app, KeyCode::Char('s'));
            }
        }
    }

    #[test]
    fn ranking_key_column_sits_next_to_the_bar() {
        let mut app = rank_tokens();
        let first = |lines: &[String], y: usize| -> String {
            let row = &lines[y];
            let i = row.rfind([RANK_FULL, RANK_HALF]).unwrap();
            let after = &row[i + RANK_FULL.len_utf8()..];
            after
                .split("   ")
                .map(str::trim)
                .find(|s| !s.is_empty())
                .unwrap()
                .to_string()
        };
        let first_head = |lines: &[String]| -> String {
            let rule = &lines[2];
            let i = rule.rfind('─').unwrap();
            rule[i + '─'.len_utf8()..]
                .split_whitespace()
                .next()
                .unwrap()
                .to_string()
        };
        let lines = draw(&mut app, 110, 14);
        assert_eq!(first_head(&lines), "worked", "{:?}", lines[2]);
        assert_eq!(first(&lines, 3), "1 h 12 min", "{:?}", lines[3]);
        press(&mut app, KeyCode::Char('s'));
        let lines = draw(&mut app, 110, 14);
        assert!(
            lines[2].ends_with("─ tokens       worked   runs"),
            "{:?}",
            lines[2]
        );
        assert_eq!(first_head(&lines), "tokens", "{:?}", lines[2]);
        assert_eq!(first(&lines, 3), "6.2M", "{:?}", lines[3]);
        assert!(
            lines[3].ends_with("  6.2M        5 min      1"),
            "{:?}",
            lines[3]
        );
        press(&mut app, KeyCode::Char('s'));
        let lines = draw(&mut app, 110, 14);
        assert!(
            lines[2].ends_with("─ runs       worked   tokens"),
            "{:?}",
            lines[2]
        );
        assert_eq!(first(&lines, 3), "2", "{:?}", lines[3]);
    }

    #[test]
    fn global_agents_say_global_in_the_ranking() {
        let mut app = with_runs(
            fixture(),
            vec![run(
                "react-structure-checker",
                "/Users/x/Offing/taskflow",
                "2026-11-04T09:00:00Z",
                8,
                None,
                true,
            )],
        );
        press(&mut app, KeyCode::Tab);
        let lines = draw(&mut app, 110, 14);
        assert!(
            lines[3].starts_with(" ❯  1. react-structure-checker    global  "),
            "{:?}",
            lines[3]
        );
    }

    #[test]
    fn ranking_footer_shows_the_period_keys() {
        let t = Theme::claude_256();
        let mut app = worked();
        press(&mut app, KeyCode::Tab);
        let lines = draw(&mut app, 140, 14);
        assert_eq!(
            bare(&lines[13]),
            " ↑↓ move · enter details · tab mode · p period: 7 days · 30 days · all time · s by: time · tokens · runs · q quit"
        );
        let buf = draw_buf(&mut app, 140, 14);
        // la clave abierta resalta como el periodo
        let x = col(&lines[13], "time ·");
        assert_eq!(buf[(x, 13)].style().fg, Some(t.claude_soft));
        assert_eq!(
            buf[(col(&lines[13], "tokens"), 13)].style().fg,
            Some(t.ink_muted)
        );
        // a 110 se van primero los periodos; luego las claves
        let lines = draw(&mut app, 110, 14);
        assert_eq!(
            bare(&lines[13]),
            " ↑↓ move · enter details · tab mode · p period · s by: time · tokens · runs · q quit"
        );
        let lines = draw(&mut app, 80, 14);
        assert_eq!(
            bare(&lines[13]),
            " ↑↓ move · enter details · tab mode · p period · s by · q quit"
        );
        // el periodo abierto resalta
        let lines = draw(&mut app, 140, 14);
        let buf = draw_buf(&mut app, 140, 14);
        let x = col(&lines[13], "all time");
        assert_eq!(buf[(x, 13)].style().fg, Some(t.claude_soft));
        assert_eq!(
            buf[(col(&lines[13], "7 days"), 13)].style().fg,
            Some(t.ink_muted)
        );
    }

    #[test]
    fn empty_ranking_points_to_the_period_key() {
        let mut app = fixture();
        press(&mut app, KeyCode::Tab);
        let lines = draw(&mut app, 110, 8);
        assert_eq!(
            lines[2],
            " nothing to show · no agent worked in this period · p changes the period"
        );
    }

    // --- ficha ---

    #[test]
    fn detail_takes_the_whole_screen_with_figures_chart_and_runs() {
        let mut app = worked();
        open(&mut app, "docs");
        let lines = draw(&mut app, 100, 26);
        let text = lines.join("\n");
        assert!(lines[0].starts_with("  active "), "{text}");
        assert!(lines[25].ends_with(&format!(" v{}", app.version)), "{text}");
        assert!(lines[2].starts_with(" docs ─"), "{text}");
        assert!(lines[2].ends_with("─ ~/Offing/argi/station"), "{text}");
        assert_eq!(lines[3], " ~/Offing/argi/station/.claude/agents/docs.md");
        assert_eq!(
            lines[5],
            " 1 h 12 min worked   ·   2 runs   ·   36 min avg   ·   last used 8 min ago"
        );
        assert_eq!(lines[7], " Agente docs");
        assert!(lines[9].starts_with(" Work per day ─"), "{text}");
        assert!(lines[9].ends_with("─ last 14 days"), "{text}");
        assert_eq!(lines[10], format!(" 50 min ┤{} ██", " ".repeat(36)));
        assert_eq!(lines[11], format!("        │{} ██", " ".repeat(36)));
        assert_eq!(lines[12], format!("        │{} ▆▆ ██", " ".repeat(33)));
        assert_eq!(lines[13], format!("        │{} ██ ██", " ".repeat(33)));
        assert_eq!(lines[14], format!("       0└{}", "─".repeat(43)));
        assert_eq!(
            lines[15],
            "          23 24 25 26 27 28 29 30 31  1  2  3  4  5   Oct → Nov"
        );
        assert!(lines[17].starts_with(" Runs ─"), "{text}");
        assert!(lines[17].ends_with("─ 2"), "{text}");
        assert_eq!(lines[18], "   Nov 4 09:00  50 min  7 tools  docs: tarea");
        assert_eq!(lines[19], "   Nov 3 09:00  22 min  7 tools  docs: tarea");
        assert_eq!(lines[20], "   + 1 old call with no recorded duration");
        assert!(!text.contains("react-structure-checker"), "{text}");
        assert_eq!(bare(&lines[25]), " esc back · ↑↓ other agent · q quit");
    }

    #[test]
    fn detail_figures_are_bold_ink_and_the_name_bold_moon() {
        let t = Theme::claude_256();
        let mut app = worked();
        open(&mut app, "docs");
        let buf = draw_buf(&mut app, 100, 26);
        assert_eq!(buf[(1, 2)].style().fg, Some(t.claude));
        assert!(buf[(1, 2)].style().add_modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(1, 5)].style().fg, Some(t.ink));
        assert!(buf[(1, 5)].style().add_modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(12, 5)].style().fg, Some(t.ink_muted), "«worked»");
        assert_eq!(buf[(24, 5)].style().fg, Some(t.subtle), "el ·");
    }

    #[test]
    fn detail_runs_are_cut_to_the_room_left() {
        let mut app = with_runs(
            fixture(),
            vec![
                run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true),
                run("docs", STATION, "2026-11-03T09:00:00Z", 22, None, true),
                run("docs", STATION, "2026-11-02T09:00:00Z", 10, None, true),
            ],
        );
        open(&mut app, "docs");
        // 18 líneas hasta la regla de ejecuciones, 2 debajo y el pie
        let lines = draw(&mut app, 100, 21);
        let text = lines.join("\n");
        assert!(lines[17].ends_with("─ 3 · most recent"), "{text}");
        assert!(lines[18].starts_with("   Nov 4 09:00"), "{text}");
        assert_eq!(lines[19], "   … 2 more", "{text}");
    }

    #[test]
    fn detail_of_a_running_agent_says_how_long_it_has_been_working() {
        let mut app = with_runs(
            fixture(),
            vec![
                run("docs", STATION, "2026-11-05T09:00:00Z", 12, None, true),
                run(
                    "docs",
                    "/Users/x/Offing/argi/station/front",
                    "2026-11-05T11:55:00Z",
                    4,
                    None,
                    false,
                ),
            ],
        );
        open(&mut app, "docs");
        let t = Theme::claude_256();
        let lines = draw(&mut app, 120, 26);
        let text = lines.join("\n");
        assert!(
            lines[5].starts_with(" ✦ running, 5 min   ·   17 min worked"),
            "{text}"
        );
        let now = lines
            .iter()
            .find(|l| l.starts_with("   Nov 5 11:55"))
            .expect(&text);
        assert!(
            now.contains("running 5 min  7 tools  docs: tarea"),
            "{now:?}"
        );
        assert!(now.ends_with("docs: tarea  ./front"), "{now:?}");
        let done = lines
            .iter()
            .find(|l| l.starts_with("   Nov 5 09:00"))
            .expect(&text);
        assert!(
            done.ends_with("docs: tarea"),
            "en la raíz no sale: {done:?}"
        );
        let buf = draw_buf(&mut app, 120, 26);
        let y = y_of(&lines, now);
        assert_eq!(buf[(col(now, "running"), y)].style().fg, Some(t.claude));
    }

    #[test]
    fn narrow_detail_figures_use_short_separators() {
        let mut app = worked();
        open(&mut app, "docs");
        let lines = draw(&mut app, 70, 26);
        assert_eq!(
            lines[5],
            " 1 h 12 min worked · 2 runs · 36 min avg · last used 8 min ago"
        );
    }

    #[test]
    fn detail_without_runs_says_so() {
        let mut app = fixture();
        open(&mut app, "visual");
        let lines = draw(&mut app, 100, 26);
        let text = lines.join("\n");
        assert_eq!(lines[5], " no runs recorded   ·   last used 5 s ago");
        assert!(text.contains("\n no work in the last 14 days\n"), "{text}");
        assert!(text.contains(" Runs ─"), "{text}");
        assert!(
            text.contains("\n   + 1 old call with no recorded duration"),
            "{text}"
        );
    }

    #[test]
    fn detail_of_a_never_used_agent_says_none() {
        let mut app = with_unused();
        app.cursor = 5;
        press(&mut app, KeyCode::Enter); // despliega
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter); // ficha
        let text = draw(&mut app, 100, 26).join("\n");
        assert!(text.contains(" ai-engineer ─"), "{text}");
        assert!(text.contains("last used never"), "{text}");
        assert!(text.contains("\n   none\n"), "{text}");
    }

    /// `backend` ejecutado `n` veces en station (la última en curso) y un
    /// `visual` de workflow una vez, en activos.
    fn fused_app(n: usize) -> App {
        let mut runs: Vec<_> = (0..n)
            .map(|i| {
                let start = format!("2026-11-05T11:{:02}:00Z", 50 + i * 2);
                run(
                    "backend",
                    STATION,
                    &start,
                    1,
                    Some(&format!("toolu_b{i}")),
                    i + 1 < n,
                )
            })
            .collect();
        let mut wf = run(
            "workflow-subagent",
            STATION,
            "2026-11-05T11:59:00Z",
            1,
            None,
            false,
        );
        wf.description = Some("visual".into());
        runs.push(wf);
        let mut app = with_runs(fixture(), runs);
        press(&mut app, KeyCode::Char('a'));
        app
    }

    /// Las terminadas de un agente se funden en una fila con `×N` (N > 1)
    /// tras la etiqueta, en `subtle`; la que sigue en curso va en su fila,
    /// sin contador.
    #[test]
    fn the_count_sits_after_the_tags_in_subtle_and_only_when_runs_exceed_one() {
        let t = Theme::claude_256();
        let mut app = fused_app(5);
        let lines = draw(&mut app, 100, 30);
        let be = find(&lines, "backend [own] ×4");
        assert!(be.contains("   backend [own] ×4  backend: tarea"), "{be:?}");
        let buf = draw_buf(&mut app, 100, 30);
        let y = y_of(&lines, be);
        assert_eq!(buf[(col(be, "×4"), y)].style().fg, Some(t.subtle));
        assert_eq!(buf[(col(be, "[own]"), y)].style().fg, Some(t.ink_muted));
        // en curso y sola: su agente encima, sin ×1
        assert!(
            lines.iter().any(|l| l == "         backend [own]"),
            "{lines:#?}"
        );
        // una sola ejecución: sin contador
        let mut one = fused_app(1);
        let lines = draw(&mut one, 100, 30);
        assert!(!lines.iter().any(|l| l.contains('×')), "{lines:#?}");
    }

    #[test]
    fn the_count_follows_via_workflow() {
        assert_eq!(live_tag(RunKind::Workflow, 3, 40), " [own] via workflow ×3");
        assert_eq!(live_tag(RunKind::Defined, 5, 40), " [own] ×5");
        assert_eq!(live_tag(RunKind::BuiltIn, 2, 40), " [claude] ×2");
        assert_eq!(live_tag(RunKind::Workflow, 1, 40), " [own] via workflow");
    }

    /// Un paso de workflow sin agente definido no es propio: `[workflow]`, sin
    /// `[own]` ni `via workflow`, y con su ×N detrás.
    #[test]
    fn a_workflow_step_without_agent_is_tagged_workflow_not_own() {
        assert_eq!(live_tag(RunKind::WorkflowStep, 1, 40), " [workflow]");
        assert_eq!(live_tag(RunKind::WorkflowStep, 3, 40), " [workflow] ×3");
        assert_eq!(kind_tag(RunKind::WorkflowStep, 40), " [workflow]");
        assert_eq!(kind_tag(RunKind::Workflow, 40), " [own] via workflow");
        // al estrecharse: nombre > origen > ×N
        // 14 = " [workflow] ×3"
        assert_eq!(live_tag(RunKind::WorkflowStep, 3, 14), " [workflow] ×3");
        assert_eq!(live_tag(RunKind::WorkflowStep, 3, 13), " [workflow]");
        assert_eq!(live_tag(RunKind::WorkflowStep, 3, 11), " [workflow]");
        assert_eq!(live_tag(RunKind::WorkflowStep, 3, 10), "");
        assert_eq!(kind_tag(RunKind::WorkflowStep, 11), " [workflow]");
        assert_eq!(kind_tag(RunKind::WorkflowStep, 10), "");
    }

    #[test]
    fn narrow_rows_keep_name_then_origin_then_count_then_via_workflow() {
        // 22 = " [own] via workflow ×3"
        assert_eq!(live_tag(RunKind::Workflow, 3, 22), " [own] via workflow ×3");
        assert_eq!(live_tag(RunKind::Workflow, 3, 21), " [own] ×3");
        assert_eq!(live_tag(RunKind::Workflow, 3, 9), " [own] ×3");
        assert_eq!(live_tag(RunKind::Workflow, 3, 8), " [own]");
        assert_eq!(live_tag(RunKind::Workflow, 3, 6), " [own]");
        assert_eq!(live_tag(RunKind::Workflow, 3, 5), "");
    }

    #[test]
    fn the_count_goes_with_the_tags_under_50_columns_and_never_overflows() {
        let mut app = fused_app(5);
        let lines = draw(&mut app, 49, 60);
        assert!(!lines.iter().any(|l| l.contains('×')), "{lines:#?}");
        for w in 30u16..=160 {
            let lines = draw(&mut app, w, 60);
            assert!(
                lines[2..59].iter().all(|l| width(l) < w as usize),
                "{w}: {lines:#?}"
            );
            // las cifras de la derecha siguen pegadas al borde
            let be = lines
                .iter()
                .find(|l| l.contains("1. ") || l.contains(" 1."));
            assert!(be.is_some(), "{w}");
        }
    }

    /// La ficha de la cabeza de un racimo de dos `backend` en curso.
    fn nav_app() -> App {
        let runs = [("2026-11-05T11:57:00Z", 2), ("2026-11-05T11:58:00Z", 1)]
            .into_iter()
            .enumerate()
            .map(|(i, (start, mins))| {
                run(
                    "backend",
                    STATION,
                    start,
                    mins,
                    Some(&format!("toolu_n{i}")),
                    false,
                )
            })
            .collect();
        let mut app = with_runs(fixture(), runs);
        press(&mut app, KeyCode::Char('a'));
        open_backend(&mut app);
        app
    }

    /// Abre la ficha de la fila de `backend`.
    fn open_backend(app: &mut App) {
        for n in ['1', '2', '3', '4', '5', '6'] {
            press(app, KeyCode::Char(n));
            press(app, KeyCode::Enter);
            if app.run_detail.as_ref().is_some_and(|d| d.name == "backend") {
                return;
            }
            press(app, KeyCode::Esc);
        }
        panic!("sin fila de backend");
    }

    #[test]
    fn the_run_header_says_which_run_it_is_when_there_are_several() {
        let t = Theme::claude_256();
        let mut app = nav_app();
        let lines = draw(&mut app, 100, 24);
        assert!(
            lines[2].contains("run 1/2 · running ") && lines[2].ends_with(" · 7 tools"),
            "{:?}",
            lines[2]
        );
        let buf = draw_buf(&mut app, 100, 24);
        assert_eq!(
            buf[(col(&lines[2], "run"), 2)].style().fg,
            Some(t.ink_muted)
        );
        press(&mut app, KeyCode::Left);
        let lines = draw(&mut app, 100, 24);
        assert!(lines[2].contains("run 2/2 · "), "{:?}", lines[2]);
        // el resto de la cabecera sigue donde estaba
        assert!(
            lines[2].starts_with(" ✧ backend [own] · station")
                || lines[2].starts_with(" ✦ backend [own] · station")
                || lines[2].starts_with("   backend [own] · station"),
            "{:?}",
            lines[2]
        );
        // estrecha: cede antes que `lleva …`
        let lines = draw(&mut app, 69, 24);
        assert!(!lines[2].contains("run"), "{:?}", lines[2]);
        for w in 30u16..=160 {
            let lines = draw(&mut app, w, 24);
            assert!(lines.iter().all(|l| width(l) <= w as usize), "{w}");
        }
    }

    #[test]
    fn the_run_header_has_no_counter_with_a_single_run() {
        let mut app = fused_app(1);
        open_backend(&mut app);
        let lines = draw(&mut app, 100, 24);
        assert!(!lines[2].contains("run "), "{:?}", lines[2]);
        assert!(lines[2].ends_with("tools"), "{:?}", lines[2]);
    }

    #[test]
    fn a_temp_folder_run_says_so_under_the_header_in_muted() {
        let t = Theme::claude_256();
        let mut app = run_app();
        let before = draw(&mut app, 110, 24);
        assert!(!before.iter().any(|l| l.contains("temp folder")));
        {
            let d = app.run_detail.as_mut().unwrap();
            d.temp_cwd = Some(PathBuf::from("/Users/x/tmp/claude-abc"));
            d.cwd_resolved = true;
        }
        let lines = draw(&mut app, 120, 24);
        assert_eq!(
            lines[3],
            "   launched from Claude's temp folder · project taken from the session · ~/tmp/claude-abc"
        );
        let buf = draw_buf(&mut app, 120, 24);
        assert_eq!(buf[(5, 3)].style().fg, Some(t.ink_muted));
        assert_eq!(
            buf[(col(&lines[3], "~/tmp"), 3)].style().fg,
            Some(t.ink_muted)
        );
        // sin resolver
        app.run_detail.as_mut().unwrap().cwd_resolved = false;
        let lines = draw(&mut app, 120, 24);
        assert!(
            lines[3].starts_with("   launched from Claude's temp folder · project unresolved"),
            "{:?}",
            lines[3]
        );
        // la ruta se recorta con … y sin sitio desaparece
        let lines = draw(&mut app, 66, 24);
        assert!(lines[3].ends_with("unresolved"), "{:?}", lines[3]);
        let lines = draw(&mut app, 90, 24);
        assert!(
            lines[3].contains("·") && width(&lines[3]) <= 90,
            "{:?}",
            lines[3]
        );
        // el resto de la ficha cuelga una línea más abajo, sin desbordes
        for w in 30u16..=160 {
            for h in [1u16, 3, 5, 7, 9, 10, 12, 14, 24] {
                let lines = draw(&mut app, w, h);
                assert_eq!(lines.len(), h as usize);
                assert!(lines.iter().all(|l| width(l) <= w as usize), "{w}x{h}");
            }
        }
    }

    #[test]
    fn the_temp_notice_gives_way_before_the_task_and_the_tools() {
        let mut app = run_app();
        let d = app.run_detail.as_mut().unwrap();
        d.temp_cwd = Some(PathBuf::from("/tmp/x"));
        d.cwd_resolved = true;
        // 9 filas: cuerpo de 6 = cabecera, regla, 3 pasos y una libre: el
        // aviso entra en esa, antes que herramientas y encargo
        let lines = draw(&mut app, 100, 9);
        assert!(lines[3].contains("temp folder"), "{lines:#?}");
        assert!(!lines.iter().any(|l| l.starts_with(" task")));
        assert!(!lines.iter().any(|l| l.starts_with(" tools")));
        // justo con cabecera, regla y pasos mínimos: sin aviso
        let lines = draw(&mut app, 100, 8);
        assert!(!lines.iter().any(|l| l.contains("temp folder")));
        // holgado: el aviso y todo lo demás
        let lines = draw(&mut app, 100, 24);
        assert!(lines.iter().any(|l| l.starts_with(" task")));
        assert!(lines.iter().any(|l| l.starts_with(" tools")));
        assert!(lines.iter().any(|l| l.contains("temp folder")));
    }

    /// La lista ya no dice dónde trabaja cada uno (eso queda en la ficha):
    /// ni con un cwd fuera del home ni sin cwd sale ninguna ruta, y nada
    /// desborda a ningún ancho.
    #[test]
    fn the_list_never_shows_a_cwd_and_never_overflows() {
        let mut runs = blocks_runs();
        runs[2].cwd = None;
        runs[2].launch_dir = Some(PathBuf::from(STATION));
        runs[1].cwd = Some(PathBuf::from("/opt/fuera/del/home"));
        let mut app = with_runs(fixture(), runs);
        press(&mut app, KeyCode::Char('a'));
        let lines = draw(&mut app, 110, 60);
        assert!(
            !lines
                .iter()
                .any(|l| l.contains("cwd") || l.contains("/opt/") || l.contains("~/")),
            "{lines:#?}"
        );
        for w in 10u16..=110 {
            let lines = draw(&mut app, w, 60);
            assert!(lines.iter().all(|l| width(l) <= w as usize), "{w}");
        }
    }

    #[test]
    fn the_run_header_takes_the_project_from_the_launch_dir_and_tells_the_cwd() {
        let t = Theme::claude_256();
        let mut app = run_app();
        // mismo sitio: sin línea de cwd
        app.run_detail.as_mut().unwrap().launch_dir = Some(PathBuf::from(STATION));
        let lines = draw(&mut app, 100, 24);
        assert!(
            !lines.iter().any(|l| l.starts_with("   cwd ")),
            "{lines:#?}"
        );
        // el subagente hizo cd: el proyecto es el de lanzamiento, el cwd debajo
        {
            let d = app.run_detail.as_mut().unwrap();
            d.launch_dir = Some(PathBuf::from("/Users/x/lanzadera"));
            d.cwd = Some(PathBuf::from(STATION));
        }
        let lines = draw(&mut app, 100, 24);
        assert!(
            lines[2].starts_with(" ✦ visual [own] via workflow · lanzadera"),
            "{:?}",
            lines[2]
        );
        // el valor en la columna de «launched by»
        assert_eq!(lines[3], "   cwd          ~/Offing/argi/station");
        let buf = draw_buf(&mut app, 100, 24);
        assert_eq!(buf[(3, 3)].style().fg, Some(t.ink_muted));
        assert_eq!(buf[(16, 3)].style().fg, Some(t.ink_muted));
        // estrecho: la ruta se recorta con … y nada desborda
        let lines = draw(&mut app, 30, 24);
        assert_eq!(lines[3], "   cwd          ~/Offing/arg…");
        for w in 10u16..=100 {
            let lines = draw(&mut app, w, 24);
            assert!(lines.iter().all(|l| width(l) <= w as usize), "{w}");
        }
        // sin launch_dir el proyecto sale del cwd, sin línea extra
        app.run_detail.as_mut().unwrap().launch_dir = None;
        let lines = draw(&mut app, 100, 24);
        assert!(lines[2].contains("· station"), "{:?}", lines[2]);
        assert!(!lines.iter().any(|l| l.starts_with("   cwd ")));
        // sin cwd, con launch_dir: proyecto de lanzamiento, sin línea extra
        {
            let d = app.run_detail.as_mut().unwrap();
            d.launch_dir = Some(PathBuf::from("/Users/x/lanzadera"));
            d.cwd = None;
        }
        let lines = draw(&mut app, 100, 24);
        assert!(lines[2].contains("· lanzadera"), "{:?}", lines[2]);
        assert!(!lines.iter().any(|l| l.starts_with("   cwd ")));
    }

    #[test]
    fn the_run_footer_tells_up_down_from_left_right() {
        let mut app = nav_app();
        let lines = draw(&mut app, 120, 24);
        assert_eq!(
            bare(&lines[23]),
            " esc back · ↑↓ other agent · ←→ other run · pgup/pgdn steps · a agent · q quit"
        );
        // una sola ejecución: sin ←→
        let mut one = fused_app(1);
        open_backend(&mut one);
        let lines = draw(&mut one, 120, 24);
        assert!(bare(&lines[23]).contains("↑↓ other agent"));
        assert!(!bare(&lines[23]).contains("←→"));
        // estrecho: cede primero ←→, luego a, pgup/pgdn y ↑↓
        let full = bare(&draw(&mut app, 120, 24)[23]);
        let wd = width(&full);
        let lines = draw(&mut app, (wd - 1) as u16, 24);
        let f = bare(&lines[23]);
        assert!(
            !f.contains("←→") && f.contains("a agent") && f.contains("pgup/pgdn"),
            "{f}"
        );
        assert!(f.contains("↑↓ other agent"));
        let lines = draw(&mut app, 30, 24);
        let f = bare(&lines[23]);
        assert!(f.contains("esc back") && f.ends_with("q quit"), "{f}");
    }

    #[test]
    fn tools_count_uses_the_english_plural() {
        assert_eq!(tools(0), "0 tools");
        assert_eq!(tools(1), "1 tool");
        assert_eq!(tools(38), "38 tools");
    }

    // --- zonas clicables: `App::hits` sale de lo que de verdad se pinta ---

    /// Cada pestaña del mapa cae sobre su texto en la fila de arriba.
    fn assert_tabs_on_text(app: &mut App, w: u16) {
        let buf = draw_buf(app, w, 20);
        let tabs = app.hits.tabs.clone();
        let modes: Vec<Mode> = tabs.iter().map(|h| h.mode).collect();
        assert_eq!(modes, [Mode::Active, Mode::All, Mode::Ranking]);
        for (hit, label) in tabs.iter().zip([" active ", " all ", " ranking "]) {
            assert_eq!(hit.row, 0);
            let n = hit.cols.end - hit.cols.start;
            let text = cells(&buf, hit.cols.start, 0, n);
            assert!(
                label.starts_with(&text),
                "{w}: {:?} sobre {text:?}",
                hit.cols
            );
            assert_eq!(
                n as usize,
                width(label).min(w as usize - hit.cols.start as usize)
            );
            // antes de la pestaña, el hueco que la separa
            assert_eq!(cells(&buf, hit.cols.start - 1, 0, 1), " ");
        }
    }

    #[test]
    fn tab_hits_sit_exactly_on_the_painted_tabs() {
        for mode in [Mode::Active, Mode::All, Mode::Ranking] {
            let mut app = fixture();
            app.mode = mode;
            // ancha, con los contadores enteros, y estrecha, recortados
            assert_tabs_on_text(&mut app, 110);
            assert_tabs_on_text(&mut app, 30);
            let tabs = app.hits.tabs.clone();
            assert_eq!(tabs[0].cols, 1..9);
            assert_eq!(tabs[1].cols, 10..15);
            assert_eq!(tabs[2].cols, 16..25);
        }
    }

    #[test]
    fn tab_hits_are_clipped_to_a_terminal_narrower_than_the_tabs() {
        let mut app = fixture();
        assert_tabs_on_text(&mut app, 20);
        assert_eq!(app.hits.tabs[2].cols, 16..20);
    }

    /// Las filas de pantalla que llevan número `N. ` y el `cursor` que les toca.
    fn numbered(lines: &[String]) -> Vec<(u16, usize)> {
        lines
            .iter()
            .enumerate()
            .filter_map(|(y, l)| {
                let head = l.trim_start().trim_start_matches('❯').trim_start();
                let digits: String = head.chars().take_while(|c| c.is_ascii_digit()).collect();
                let n: usize = digits.parse().ok()?;
                head[digits.len()..]
                    .starts_with(". ")
                    .then_some((y as u16, n - 1))
            })
            .collect()
    }

    fn row_hits(app: &App) -> Vec<(u16, usize)> {
        app.hits.rows.iter().map(|h| (h.row, h.cursor)).collect()
    }

    #[test]
    fn row_hits_point_at_the_numbered_rows_of_the_list() {
        let mut app = fixture();
        let lines = draw(&mut app, 110, 20);
        let want = numbered(&lines[..19]);
        assert!(want.len() >= 3, "{lines:#?}");
        assert_eq!(row_hits(&app), want, "{lines:#?}");
        // ni las reglas, ni los blancos, ni la cabecera llevan zona
        assert!(app.hits.rows.iter().all(|h| h.row >= 2 && h.row < 19));
    }

    #[test]
    fn row_hits_follow_the_scroll() {
        let mut app = fixture();
        app.cursor = app.count() - 1;
        let lines = draw(&mut app, 110, 8);
        assert!(app.scroll > 0, "sin scroll: {lines:#?}");
        let want = numbered(&lines[..7]);
        assert!(want.iter().any(|&(_, c)| c == app.cursor), "{lines:#?}");
        assert_eq!(row_hits(&app), want, "{lines:#?}");
    }

    #[test]
    fn every_line_of_an_active_row_clicks_its_run() {
        let mut app = blocks();
        let lines = draw(&mut app, 110, 60);
        let g = pos(&lines, " station · 2 running · 1 recent ──");
        let hits = row_hits(&app);
        let at = |y: usize| hits.iter().find(|h| h.0 as usize == y).map(|h| h.1);
        assert_eq!(at(g), None, "la regla del grupo no se pulsa");
        // la fila con skill: agente, fila, skill y now
        for y in g + 1..=g + 4 {
            assert_eq!(at(y), Some(2), "línea {y}: {lines:#?}");
        }
        // la fila sin skill, tres líneas (agente, fila y now); la terminada, una
        for y in g + 5..=g + 7 {
            assert_eq!(at(y), Some(3), "línea {y}: {lines:#?}");
        }
        assert_eq!(at(g + 8), Some(4), "{lines:#?}");
        assert_eq!(at(g + 9), None, "el blanco antes de la regla no se pulsa");
        assert_eq!(at(2), None, "la cabecera de columnas no se pulsa");
    }

    #[test]
    fn a_row_cut_by_the_scroll_keeps_its_run_on_the_visible_lines() {
        let mut app = blocks();
        let items = app.items();
        let live = |n: usize| {
            items
                .iter()
                .position(|i| matches!(i, Item::Live { n: m, .. } if *m == n))
                .unwrap()
        };
        // el `Live` de la fila 3 queda justo encima de la vista
        app.scroll = live(3) + 1;
        app.cursor = 3;
        let lines = draw(&mut app, 110, 14);
        assert_eq!(app.scroll, live(3) + 1, "{lines:#?}");
        assert!(lines[2].starts_with("           skill"), "{lines:#?}");
        let hits = row_hits(&app);
        // skill y now de la 3; agente, fila y now de la 4; la terminada
        assert_eq!(
            &hits[..6],
            &[(2, 2), (3, 2), (4, 3), (5, 3), (6, 3), (7, 4)],
            "{lines:#?}"
        );
    }

    #[test]
    fn an_open_card_keeps_the_tabs_but_no_rows() {
        let mut app = fixture();
        draw(&mut app, 110, 20);
        assert!(!app.hits.rows.is_empty());
        press(&mut app, KeyCode::Enter);
        assert!(app.detail);
        draw(&mut app, 110, 20);
        assert!(app.hits.rows.is_empty());
        assert_eq!(app.hits.tabs.len(), 3);

        let mut app = blocks();
        press(&mut app, KeyCode::Enter);
        assert!(app.run_detail.is_some());
        draw(&mut app, 110, 30);
        assert!(app.hits.rows.is_empty());
        assert_eq!(app.hits.tabs.len(), 3);
    }
}
