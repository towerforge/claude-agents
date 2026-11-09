//! El panel de `claude-agents update` con terminal, como el de `moon update`:
//! unas líneas en línea (sin pantalla completa) que dicen de qué versión a
//! cuál se pasa, qué trae, preguntan antes de tocar nada y animan la descarga.
//! La lógica es la de `update::flow`; aquí solo el bucle y el pintado.

use super::theme::Theme;
use super::view::{section_line, star, truncate, width, wrap};
use crate::report;
use crate::update::flow::{self, Msg, Next, Options, Outcome, Phase, State, Step};
use crate::update::{Client, Source};
use chrono::{DateTime, Utc};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui::{DefaultTerminal, TerminalOptions, Viewport};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

/// Cada cuánto se repinta: la estrella parpadea cada medio segundo y la barra
/// avanza con lo que llega.
const TICK: Duration = Duration::from_millis(100);

/// El panel entero: pregunta a GitHub en un hilo, pinta, pregunta y descarga
/// en otro hilo; el último fotograma queda en la terminal, encima del prompt.
/// `None` si la terminal no deja abrir el panel (no dice dónde está el
/// cursor): quien llama sigue con la vista de texto, sin nada tocado.
pub fn run(opts: &Options, state: &mut State) -> Option<Outcome> {
    let inline = TerminalOptions {
        viewport: Viewport::Inline(HEIGHT),
    };
    let Ok(mut term) = ratatui::try_init_with_options(inline) else {
        ratatui::restore();
        return None;
    };
    let theme = Theme::detect();
    let outcome = drive(&mut term, opts, state, &theme);
    // el último fotograma se queda por encima del prompt; el vivo se va
    let _ = term.insert_before(HEIGHT, |buf| {
        render(state, &theme, Utc::now(), buf.area, buf)
    });
    let _ = term.clear();
    ratatui::restore();
    Some(outcome)
}

fn drive(term: &mut DefaultTerminal, opts: &Options, state: &mut State, t: &Theme) -> Outcome {
    let client = Arc::new(Client::new());
    let (tx, rx) = mpsc::channel::<Msg>();
    {
        let (client, tx, to) = (client.clone(), tx.clone(), opts.to.clone());
        thread::spawn(move || {
            let release = flow::fetch(&*client, to.as_ref()).map_err(|e| e.to_string());
            let _ = tx.send(Msg::Release(release));
        });
    }
    loop {
        if term
            .draw(|f| render(state, t, Utc::now(), f.area(), f.buffer_mut()))
            .is_err()
        {
            return state.outcome();
        }
        while let Ok(msg) = rx.try_recv() {
            match flow::apply(msg, state, opts) {
                Next::Install => spawn_install(&client, state, &tx),
                Next::Done(outcome) => return outcome,
                Next::Wait | Next::Ask => {}
            }
        }
        match event::poll(TICK) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => return state.outcome(),
        }
        let Ok(Event::Key(k)) = event::read() else {
            continue;
        };
        if k.kind == KeyEventKind::Release {
            continue;
        }
        match on_key(&state.phase, k) {
            Some(Answer::Yes) => {
                if state.confirm() == Next::Install {
                    spawn_install(&client, state, &tx);
                }
            }
            // al salir, el hilo de la descarga muere con el proceso: nada
            // se ha escrito todavía
            Some(Answer::No) => return state.cancel(),
            None => {}
        }
    }
}

/// Descarga, comprueba e instala en otro hilo; cada paso llega por el canal.
fn spawn_install(client: &Arc<Client>, state: &State, tx: &mpsc::Sender<Msg>) {
    let Some(release) = state.release.clone() else {
        let _ = tx.send(Msg::Finished(Err("no release to install".into())));
        return;
    };
    let (client, tx) = (client.clone(), tx.clone());
    let (target, dest) = (state.target, state.dest.clone());
    thread::spawn(move || {
        let source: &dyn Source = &*client;
        let res = flow::install_release(source, &release, &target, &dest, &mut |m| {
            let _ = tx.send(m);
        });
        let _ = tx.send(Msg::Finished(res.map_err(|e| e.to_string())));
    });
}

/// Lo que dice una tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Yes,
    No,
}

/// Enter o `y` en la pregunta; esc, `q`, `n` o ctrl-c mientras no se ha
/// tocado nada. Comprobando o instalando no se puede parar a medias.
fn on_key(phase: &Phase, k: KeyEvent) -> Option<Answer> {
    let ctrl_c = k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
    let cancel = ctrl_c || matches!(k.code, KeyCode::Esc | KeyCode::Char('q' | 'n'));
    match phase {
        Phase::Confirm if matches!(k.code, KeyCode::Enter | KeyCode::Char('y')) => {
            Some(Answer::Yes)
        }
        Phase::Confirm | Phase::Checking | Phase::Working(Step::Download { .. }) if cancel => {
            Some(Answer::No)
        }
        _ => None,
    }
}

/// Filas del panel: un blanco arriba, la regla, un blanco, cuatro de datos,
/// un blanco, el estado, un blanco y las teclas.
pub const HEIGHT: u16 = 11;
/// Donde empiezan los datos: bajo el texto del estado, tras `✗ `.
const INDENT: usize = 3;
/// `install to  `: la etiqueta más larga y su hueco.
const LABEL_W: usize = 12;
/// Hueco entre los datos y las notas.
const NOTES_GAP: usize = 4;
/// Por debajo de esto las notas no caben con sentido y se van.
const NOTES_MIN: usize = 24;
const NOTES_MAX: usize = 3;
/// Lo que se le deja como mínimo a la ruta antes de que se vayan las notas.
const PATH_MIN: usize = 28;
const BAR_W: usize = 24;
/// Por debajo de esto la barra no dice nada: quedan solo las cifras.
const BAR_MIN: usize = 4;

const ROW_RULE: u16 = 1;
const ROW_HEAD: u16 = 3;
const ROW_STATUS: u16 = 8;
const ROW_KEYS: u16 = 10;

/// ```text
///
///  claude-agents update ──────────────────────────────── released 2 d ago
///
///    1.1.0  →  1.2.0                         what's new in 1.2.0
///    package     macos · aarch64 · 2.1 MB    · feat: update command
///    install to  ~/.local/bin/claude-agents  · fix: narrow terminals
///
///  ████████████░░░░░░░░░░░░   50%  1.1 MB / 2.1 MB
///
///  esc cancel
/// ```
///
/// Como la pantalla: nada pinta fondo, la regla al estilo de las secciones y
/// las teclas en `claude` con la acción en `ink`. Se pinta pegado abajo del
/// área; la última columna queda libre.
pub fn render(state: &State, t: &Theme, now: DateTime<Utc>, area: Rect, buf: &mut Buffer) {
    if area.height < HEIGHT || area.width < 12 {
        return;
    }
    let w = area.width as usize;
    let top = area.y + area.height - HEIGHT;
    let put = |buf: &mut Buffer, row: u16, x: usize, line: Line| {
        let room = (w - 1).saturating_sub(x) as u16;
        line.render(Rect::new(area.x + x as u16, top + row, room, 1), buf);
    };

    // la regla: lo que se actualiza y, si cabe, cuándo salió la release
    let title = "claude-agents update";
    let info = state
        .release
        .as_ref()
        .and_then(|r| r.published_at)
        .map(|at| format!("released {}", report::relative(at, now)))
        .filter(|i| width(title) + width(i) + 8 <= w)
        .unwrap_or_default();
    put(
        buf,
        ROW_RULE,
        0,
        section_line(t, title, t.accent_bold(), &info, w),
    );

    // los datos: versiones, paquete y destino
    let left_room = (w - 1).saturating_sub(INDENT);
    // con una release que trae notas, los valores ceden hasta `PATH_MIN` para
    // que quepan a la derecha
    let beside_notes = (w - 1).saturating_sub(INDENT + LABEL_W + NOTES_GAP + NOTES_MIN);
    let value_room = match state.next_version() {
        Some(_) => left_room
            .saturating_sub(LABEL_W)
            .min(beside_notes.max(PATH_MIN)),
        None => left_room.saturating_sub(LABEL_W),
    };
    let versions = match state.next_version() {
        Some(next) => vec![
            Span::styled(state.current.to_string(), t.muted()),
            Span::styled("  →  ", t.line()),
            Span::styled(next.to_string(), t.accent_bold()),
        ],
        None => vec![
            Span::styled("claude-agents ", t.text()),
            Span::styled(state.current.to_string(), t.muted()),
        ],
    };
    let package = match state.asset_size() {
        Some(size) => format!("{} · {}", state.target, mb(size)),
        None => state.target.to_string(),
    };
    let rows = [
        ("package", truncate(&package, value_room)),
        (
            "install to",
            truncate_start(&state.dest_display(), value_room),
        ),
    ];
    let left_w = rows
        .iter()
        .map(|(_, v)| LABEL_W + width(v))
        .chain([versions.iter().map(|s| width(&s.content)).sum()])
        .max()
        .unwrap_or(0);
    put(buf, ROW_HEAD, INDENT, Line::from(versions));
    for (i, (label, value)) in rows.into_iter().enumerate() {
        let spans = vec![
            Span::styled(format!("{label:<LABEL_W$}"), t.muted()),
            Span::styled(value, t.text()),
        ];
        put(buf, ROW_HEAD + 1 + i as u16, INDENT, Line::from(spans));
    }

    // las notas de la versión que llega, a la derecha y solo si caben
    let notes_x = INDENT + left_w + NOTES_GAP;
    let notes_room = (w - 1).saturating_sub(notes_x);
    let incoming = state.next_version().and(state.release.as_ref());
    if let (Some(release), true) = (incoming, notes_room >= NOTES_MIN) {
        let head = format!("what's new in {}", release.version);
        put(
            buf,
            ROW_HEAD,
            notes_x,
            Line::styled(truncate(&head, notes_room), t.soft()),
        );
        let notes = release.highlights(NOTES_MAX);
        let notes = if notes.is_empty() {
            vec![Line::styled("no release notes", t.muted())]
        } else {
            notes
                .iter()
                .map(|n| {
                    Line::from(vec![
                        Span::styled("· ", t.line()),
                        Span::styled(truncate(n, notes_room - 2), t.muted()),
                    ])
                })
                .collect()
        };
        for (i, line) in notes.into_iter().enumerate() {
            put(buf, ROW_HEAD + 1 + i as u16, notes_x, line);
        }
    }

    for (i, line) in status_lines(state, t, now, w).into_iter().enumerate() {
        put(buf, ROW_STATUS + i as u16, 0, line);
    }
    put(buf, ROW_KEYS, 0, keys_line(state, t));
}

/// Lo que está pasando, en la fila del estado. Un error ocupa hasta las tres
/// filas de abajo (las teclas no tienen nada que ofrecer entonces).
fn status_lines(state: &State, t: &Theme, now: DateTime<Utc>, w: usize) -> Vec<Line<'static>> {
    let room = (w - 1).saturating_sub(INDENT);
    let next = state
        .next_version()
        .map(ToString::to_string)
        .unwrap_or_default();
    let mark = |m: &str, style| vec![Span::raw(" "), Span::styled(format!("{m} "), style)];
    let spin = |text: String| {
        let mut spans = mark(star(now), t.accent());
        spans.push(Span::styled(truncate(&text, room), t.muted()));
        vec![Line::from(spans)]
    };
    let said = |m: &str, style, text: String| {
        let mut spans = mark(m, style);
        spans.push(Span::styled(truncate(&text, room), t.text()));
        spans
    };
    match &state.phase {
        Phase::Checking => spin("checking GitHub for a new release…".into()),
        Phase::UpToDate => vec![Line::from(said(
            "✓",
            t.ok(),
            format!("claude-agents {} is the latest version", state.current),
        ))],
        Phase::Available => vec![Line::from(said(
            "↑",
            t.accent(),
            format!("claude-agents {next} is available"),
        ))],
        Phase::Confirm => vec![Line::from(said(
            "↑",
            t.accent(),
            format!("install claude-agents {next}?"),
        ))],
        Phase::Working(Step::Download { got, total }) => vec![download_line(*got, *total, t, w)],
        Phase::Working(Step::Verify) => spin("verifying sha-256".into()),
        Phase::Working(Step::Install) => spin(format!("installing to {}", state.dest_display())),
        Phase::Done => {
            let text = format!("claude-agents {next} installed");
            let mut spans = said("✓", t.ok(), text.clone());
            if !state.verified {
                let rest = room.saturating_sub(width(&text));
                spans.push(Span::styled(
                    truncate(" · checksum not published: not verified", rest),
                    t.warn(),
                ));
            }
            vec![Line::from(spans)]
        }
        Phase::Cancelled => vec![Line::from(vec![
            Span::raw(" "),
            Span::styled(
                truncate("✗ cancelled · nothing was touched", room + 2),
                t.muted(),
            ),
        ])],
        Phase::Refused(r) => {
            let mut spans = said("✗", t.alert(), r.why.to_string());
            let rest = room.saturating_sub(width(r.why));
            spans.push(Span::styled(
                truncate(" · --force overwrites it", rest),
                t.muted(),
            ));
            vec![Line::from(spans)]
        }
        Phase::Failed(e) => {
            let mut lines = wrap(e, room);
            if lines.len() > 3 {
                lines.truncate(3);
                lines[2] = truncate(&format!("{} …", lines[2]), room);
            }
            lines
                .into_iter()
                .enumerate()
                .map(|(i, l)| {
                    Line::from(vec![
                        Span::styled(if i == 0 { " ✗ " } else { "   " }, t.alert()),
                        Span::styled(l, t.alert()),
                    ])
                })
                .collect()
        }
    }
}

/// `████████░░░░   50%  1.1 MB / 2.1 MB`: la barra en `claude`, las cifras en
/// `ink-muted`. Sin tamaño, la barra no se llena y solo cuentan los MB;
/// estrecho, la barra se acorta y luego se va, y las cifras se quedan.
fn download_line(got: u64, total: Option<u64>, t: &Theme, w: usize) -> Line<'static> {
    let total = total.filter(|t| *t > 0);
    let frac = total.map_or(0.0, |t| (got as f64 / t as f64).clamp(0.0, 1.0));
    let figures = match total {
        Some(t) => format!("  {:>3.0}%  {} / {}", frac * 100.0, mb(got), mb(t)),
        None => format!("  {}", mb(got)),
    };
    let bar_w = BAR_W.min((w - 1).saturating_sub(1 + width(&figures)));
    let mut spans = vec![Span::raw(" ")];
    if bar_w >= BAR_MIN {
        let filled = (frac * bar_w as f64).round() as usize;
        spans.push(Span::styled(
            format!("{}{}", "█".repeat(filled), "░".repeat(bar_w - filled)),
            t.accent(),
        ));
        spans.push(Span::styled(figures, t.muted()));
    } else {
        spans.push(Span::styled(figures.trim_start().to_string(), t.muted()));
    }
    Line::from(spans)
}

/// Las teclas de la fase, como el pie de la pantalla; o, cuando ya no hay
/// ninguna, lo que se escribe a continuación.
fn keys_line(state: &State, t: &Theme) -> Line<'static> {
    let keys = |keys: &[(&str, &str)]| {
        let mut spans = vec![Span::raw(" ")];
        for (i, (k, label)) in keys.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", t.muted()));
            }
            spans.push(Span::styled(k.to_string(), t.accent()));
            spans.push(Span::styled(format!(" {label}"), t.text()));
        }
        Line::from(spans)
    };
    match &state.phase {
        Phase::Confirm => keys(&[("enter", "update"), ("esc", "cancel")]),
        Phase::Checking | Phase::Working(Step::Download { .. }) => keys(&[("esc", "cancel")]),
        Phase::Available => keys(&[("claude-agents update", "installs it")]),
        Phase::Done => keys(&[("claude-agents", "opens the new version")]),
        // el comando que actualiza esta copia, entero: es lo que toca escribir
        Phase::Refused(r) => Line::from(vec![
            Span::raw(" "),
            Span::styled(r.fix.to_string(), t.accent()),
        ]),
        _ => Line::default(),
    }
}

/// Corta por delante: de una ruta, lo que importa es el final (`…/bin/claude-agents`).
fn truncate_start(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut w = 1;
    for ch in s.chars().rev() {
        let cw = width(ch.encode_utf8(&mut [0; 4]));
        if w + cw > max {
            break;
        }
        tail.push(ch);
        w += cw;
    }
    if max == 0 {
        return String::new();
    }
    std::iter::once('…').chain(tail.into_iter().rev()).collect()
}

/// `2.1 MB`, con un decimal.
fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Las teclas valen lo que el panel enseña: enter o y solo en la pregunta,
    /// esc mientras nada se ha tocado, y nada mientras se comprueba o instala.
    #[test]
    fn keys_do_what_the_panel_offers() {
        let ask = Phase::Confirm;
        assert_eq!(on_key(&ask, key(KeyCode::Enter)), Some(Answer::Yes));
        assert_eq!(on_key(&ask, key(KeyCode::Char('y'))), Some(Answer::Yes));
        assert_eq!(on_key(&ask, key(KeyCode::Char('n'))), Some(Answer::No));
        assert_eq!(on_key(&ask, key(KeyCode::Esc)), Some(Answer::No));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(on_key(&Phase::Checking, ctrl_c), Some(Answer::No));
        assert_eq!(on_key(&Phase::Checking, key(KeyCode::Enter)), None);
        let download = Phase::Working(Step::Download {
            got: 0,
            total: None,
        });
        assert_eq!(on_key(&download, key(KeyCode::Char('q'))), Some(Answer::No));
        assert_eq!(
            on_key(&Phase::Working(Step::Verify), key(KeyCode::Esc)),
            None
        );
        assert_eq!(on_key(&Phase::Working(Step::Install), ctrl_c), None);
    }
    use crate::update::{Asset, Refusal, Release, Target};
    use ratatui::style::Color;
    use std::path::PathBuf;

    fn state(phase: Phase) -> State {
        let mut s = State::new(
            "1.1.0".parse().unwrap(),
            Target::new("macos", "aarch64", false),
            PathBuf::from("/usr/local/bin/claude-agents"),
        );
        s.release = Some(Release {
            version: "1.2.0".parse().unwrap(),
            tag: "v1.2.0".into(),
            notes: "* feat: update command by @towerforge in http://x\n* fix: narrow terminals\n"
                .into(),
            url: String::new(),
            published_at: Some(now() - chrono::TimeDelta::days(2)),
            assets: vec![Asset {
                name: "claude-agents-macos-aarch64.tar.gz".into(),
                url: String::new(),
                size: 2_100_000,
            }],
        });
        s.phase = phase;
        s
    }

    fn now() -> DateTime<Utc> {
        "2026-11-09T12:00:00Z".parse().unwrap()
    }

    fn draw_buf(state: &State, w: u16) -> Buffer {
        let area = Rect::new(0, 0, w, HEIGHT);
        let mut buf = Buffer::empty(area);
        render(state, &Theme::claude_256(), now(), area, &mut buf);
        buf
    }

    fn draw(state: &State, w: u16) -> Vec<String> {
        let buf = draw_buf(state, w);
        (0..HEIGHT)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// La pregunta: regla con la fecha de la release, versiones, paquete y
    /// destino a la izquierda, las notas a la derecha y las teclas abajo.
    #[test]
    fn the_question_shows_versions_destination_and_notes() {
        let lines = draw(&state(Phase::Confirm), 90);
        let text = lines.join("\n");
        assert_eq!(lines[0], "", "{text}");
        assert!(lines[1].starts_with(" claude-agents update ───"), "{text}");
        assert!(lines[1].ends_with("─ released 2 d ago"), "{text}");
        assert!(lines[3].starts_with("   1.1.0  →  1.2.0"), "{text}");
        assert!(lines[3].contains("what's new in 1.2.0"), "{text}");
        assert!(
            lines[4].starts_with("   package     macos · aarch64 · 2.1 MB"),
            "{text}"
        );
        assert!(lines[4].ends_with("· feat: update command"), "{text}");
        // el autor y la pull request son ruido en un panel así
        assert!(!text.contains("@towerforge"), "{text}");
        assert!(
            lines[5].starts_with("   install to  /usr/local/bin/claude-agents"),
            "{text}"
        );
        assert!(lines[5].ends_with("· fix: narrow terminals"), "{text}");
        assert_eq!(lines[8], " ↑ install claude-agents 1.2.0?", "{text}");
        assert_eq!(lines[10], " enter update · esc cancel", "{text}");
        // las notas empiezan en la misma columna en las tres filas
        let col = |l: &str, pat: &str| l.find(pat).map(|i| width(&l[..i]));
        assert_eq!(col(&lines[3], "what's"), col(&lines[4], "· feat"));
        assert_eq!(col(&lines[4], "· feat"), col(&lines[5], "· fix"));
    }

    #[test]
    fn the_question_paints_the_new_version_and_the_keys_in_claude() {
        let t = Theme::claude_256();
        let buf = draw_buf(&state(Phase::Confirm), 90);
        // `1.2.0` en la fila 3, tras `   1.1.0  →  `
        assert_eq!(buf[(13, 3)].symbol(), "1");
        assert_eq!(buf[(13, 3)].style().fg, Some(t.claude));
        assert_eq!(buf[(3, 3)].style().fg, Some(t.ink_muted));
        // `enter` en `claude`, `update` en `ink`
        assert_eq!(buf[(1, 10)].style().fg, Some(t.claude));
        assert_eq!(buf[(7, 10)].style().fg, Some(t.ink));
        // nada pinta fondo
        for y in 0..HEIGHT {
            for x in 0..90 {
                let bg = buf[(x, y)].style().bg;
                assert!(matches!(bg, None | Some(Color::Reset)), "({x}, {y})");
            }
        }
    }

    /// Una ruta larga se corta por delante para dejar sitio a las notas.
    #[test]
    fn a_long_destination_leaves_room_for_the_notes() {
        let mut s = state(Phase::Confirm);
        s.dest =
            PathBuf::from("/private/tmp/some/very/long/scratch/folder/with/a/build/claude-agents");
        let lines = draw(&s, 90);
        assert!(lines[3].contains("what's new in 1.2.0"), "{lines:?}");
        assert!(lines[5].starts_with("   install to  …"), "{lines:?}");
        assert!(lines[5].contains("/build/claude-agents  "), "{lines:?}");
    }

    /// Estrecho, las notas se van y lo de la izquierda se queda.
    #[test]
    fn a_narrow_panel_drops_the_notes() {
        let lines = draw(&state(Phase::Confirm), 50);
        let text = lines.join("\n");
        assert!(!text.contains("what's new"), "{text}");
        assert_eq!(lines[3], "   1.1.0  →  1.2.0", "{text}");
        assert_eq!(
            lines[5], "   install to  /usr/local/bin/claude-agents",
            "{text}"
        );
        assert!(lines.iter().all(|l| width(l) <= 50), "{text}");
        let tiny = draw(&state(Phase::Confirm), 30);
        assert!(tiny.iter().all(|l| width(l) <= 30), "{tiny:?}");
        // de la ruta se queda el final, que es lo que dice qué se sustituye
        assert_eq!(tiny[5], "   install to  …claude-agents", "{tiny:?}");
    }

    /// Mientras pregunta a GitHub: la versión en marcha y la estrella que
    /// parpadea, como en la pantalla.
    #[test]
    fn checking_blinks_the_star() {
        let mut s = state(Phase::Checking);
        s.release = None;
        let lines = draw(&s, 90);
        assert_eq!(lines[1].find("released"), None, "{lines:?}");
        assert_eq!(lines[3], "   claude-agents 1.1.0", "{lines:?}");
        assert_eq!(lines[4], "   package     macos · aarch64", "{lines:?}");
        assert_eq!(
            lines[8], " ✦ checking GitHub for a new release…",
            "{lines:?}"
        );
        assert_eq!(lines[10], " esc cancel", "{lines:?}");
    }

    #[test]
    fn the_bar_advances_with_the_download() {
        let lines = draw(
            &state(Phase::Working(Step::Download {
                got: 1_050_000,
                total: Some(2_100_000),
            })),
            90,
        );
        assert_eq!(
            lines[8], " ████████████░░░░░░░░░░░░   50%  1.1 MB / 2.1 MB",
            "{lines:?}"
        );
        assert_eq!(lines[10], " esc cancel", "{lines:?}");
        // sin tamaño, la barra no avanza pero los MB sí
        let unknown = draw(
            &state(Phase::Working(Step::Download {
                got: 700_000,
                total: None,
            })),
            90,
        );
        assert_eq!(
            unknown[8], " ░░░░░░░░░░░░░░░░░░░░░░░░  0.7 MB",
            "{unknown:?}"
        );
        // estrecho, la barra se acorta antes que las cifras
        let narrow = draw(
            &state(Phase::Working(Step::Download {
                got: 1_050_000,
                total: Some(2_100_000),
            })),
            40,
        );
        assert!(narrow[8].ends_with("50%  1.1 MB / 2.1 MB"), "{narrow:?}");
        assert!(width(&narrow[8]) <= 40, "{narrow:?}");
    }

    /// Comprobar e instalar no se pueden cancelar: no se ofrece la tecla.
    #[test]
    fn verifying_and_installing_offer_no_cancel() {
        let verify = draw(&state(Phase::Working(Step::Verify)), 90);
        assert_eq!(verify[8], " ✦ verifying sha-256", "{verify:?}");
        assert_eq!(verify[10], "", "{verify:?}");
        let install = draw(&state(Phase::Working(Step::Install)), 90);
        assert_eq!(
            install[8], " ✦ installing to /usr/local/bin/claude-agents",
            "{install:?}"
        );
    }

    #[test]
    fn done_says_so_in_green_and_warns_when_not_verified() {
        let t = Theme::claude_256();
        let mut s = state(Phase::Done);
        let lines = draw(&s, 90);
        assert_eq!(lines[8], " ✓ claude-agents 1.2.0 installed", "{lines:?}");
        assert_eq!(
            lines[10], " claude-agents opens the new version",
            "{lines:?}"
        );
        assert_eq!(draw_buf(&s, 90)[(1, 8)].style().fg, Some(t.success));
        s.verified = false;
        let lines = draw(&s, 90);
        assert_eq!(
            lines[8], " ✓ claude-agents 1.2.0 installed · checksum not published: not verified",
            "{lines:?}"
        );
        assert_eq!(draw_buf(&s, 90)[(35, 8)].style().fg, Some(t.warning));
    }

    #[test]
    fn up_to_date_available_and_cancelled() {
        let mut s = state(Phase::UpToDate);
        s.release.as_mut().unwrap().version = "1.1.0".parse().unwrap();
        let lines = draw(&s, 90);
        assert_eq!(lines[3], "   claude-agents 1.1.0", "{lines:?}");
        assert_eq!(
            lines[8], " ✓ claude-agents 1.1.0 is the latest version",
            "{lines:?}"
        );
        assert_eq!(lines[10], "", "{lines:?}");
        let lines = draw(&state(Phase::Available), 90);
        assert_eq!(lines[8], " ↑ claude-agents 1.2.0 is available", "{lines:?}");
        assert_eq!(lines[10], " claude-agents update installs it", "{lines:?}");
        let lines = draw(&state(Phase::Cancelled), 90);
        assert_eq!(lines[8], " ✗ cancelled · nothing was touched", "{lines:?}");
    }

    /// Rechazado: el motivo en el estado y el comando que lo actualiza, entero
    /// y en su propia línea.
    #[test]
    fn refused_puts_the_fix_on_its_own_line() {
        let t = Theme::claude_256();
        let s = state(Phase::Refused(Refusal {
            why: "installed with cargo",
            fix: "cargo install --git https://github.com/towerforge/claude-agents",
        }));
        let lines = draw(&s, 90);
        assert_eq!(
            lines[8], " ✗ installed with cargo · --force overwrites it",
            "{lines:?}"
        );
        assert_eq!(
            lines[10], " cargo install --git https://github.com/towerforge/claude-agents",
            "{lines:?}"
        );
        let buf = draw_buf(&s, 90);
        assert_eq!(buf[(1, 8)].style().fg, Some(t.alert));
        assert_eq!(buf[(1, 10)].style().fg, Some(t.claude));
    }

    /// El error en `alert`, partido en hasta tres líneas en lugar de cortado.
    #[test]
    fn failed_is_in_alert_and_wraps() {
        let t = Theme::claude_256();
        let mut s = state(Phase::Failed(
            "github: HTTP 502 while asking for the latest release (no connection?)".into(),
        ));
        s.release = None;
        let lines = draw(&s, 50);
        assert_eq!(
            lines[8], " ✗ github: HTTP 502 while asking for the latest",
            "{lines:?}"
        );
        assert_eq!(lines[9], "   release (no connection?)", "{lines:?}");
        let buf = draw_buf(&s, 50);
        assert_eq!(buf[(1, 8)].style().fg, Some(t.alert));
        assert_eq!(buf[(3, 9)].style().fg, Some(t.alert));
    }
}
