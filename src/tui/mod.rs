//! Pantalla de terminal al estilo de moon: lista viva de agentes por proyecto,
//! que se relee sola mientras está abierta.

pub mod app;
pub mod scan;
pub mod theme;
pub mod update;
pub mod view;

use anyhow::Result;
use app::{Action, App};
use chrono::Utc;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::DefaultTerminal;
use std::ffi::OsStr;
use std::io::stdout;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use theme::Theme;

/// Lo que la pantalla necesita saber del arranque.
pub struct Config {
    pub roots: Vec<PathBuf>,
    pub home: PathBuf,
    pub projects: PathBuf,
    /// Historial guardado de ejecuciones (`runs::history`); `None`, sin él.
    pub history: Option<PathBuf>,
    pub minutes: i64,
    pub version: &'static str,
    /// Carpeta de la nota diaria de versiones (`update::cache::default_dir`).
    /// `None` no comprueba nada: `CLAUDE_AGENTS_NO_UPDATE_CHECK=1` o los tests.
    pub update_cache: Option<PathBuf>,
}

/// La variable que apaga el aviso de versión nueva.
pub const NO_UPDATE_CHECK: &str = "CLAUDE_AGENTS_NO_UPDATE_CHECK";

/// Si se comprueba la versión al abrir la pantalla, según el valor de
/// `CLAUDE_AGENTS_NO_UPDATE_CHECK`: sin ella, vacía o `0`, sí.
pub fn update_check_enabled(value: Option<&OsStr>) -> bool {
    value.is_none_or(|v| v.is_empty() || v == "0")
}

/// Pregunta en otro hilo si hay versión nueva (`check`, que no debe fallar
/// nunca) y la deja en el canal sin la `v`. El bucle no espera a nadie: si no
/// hay red, el hilo tarda lo que tarde y no manda nada.
fn spawn_update_check(
    check: impl FnOnce() -> Option<crate::update::Version> + Send + 'static,
) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if let Some(v) = check() {
            let _ = tx.send(v.to_string());
        }
    });
    rx
}

/// Cada cuánto se repinta aunque no pase nada: los «hace N s» avanzan solos.
const FRAME: Duration = Duration::from_millis(250);

pub fn run(cfg: Config) -> Result<()> {
    let theme = Theme::detect();
    let (rescan_tx, rescan_rx) = mpsc::channel();
    let msgs = scan::spawn(
        scan::Sources {
            roots: cfg.roots,
            home: cfg.home.clone(),
            projects: cfg.projects.clone(),
            history: cfg.history,
        },
        rescan_rx,
    );
    // Sin la versión en marcha bien escrita no hay con qué comparar.
    let updates = match (cfg.update_cache, cfg.version.parse()) {
        (Some(dir), Ok(current)) => Some(spawn_update_check(move || {
            crate::update::cache::newer_than(&current, Some(&dir), crate::update::cache::TTL)
        })),
        _ => None,
    };
    let mut app = App::new(cfg.minutes, cfg.home, cfg.version, Utc::now());
    // la carpeta que la pantalla de carga dice estar leyendo
    app.projects = cfg.projects;

    let mut terminal = ratatui::init();
    let mouse = Mouse::enable();
    // ratatui::init ya restaura el terminal en un pánico; la captura del ratón
    // es cosa nuestra y hay que soltarla antes.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        mouse.disable();
        prev(info);
    }));
    let result = event_loop(
        &mut app,
        &mut terminal,
        &theme,
        &msgs,
        updates.as_ref(),
        &rescan_tx,
    );
    mouse.disable();
    ratatui::restore();
    result
}

/// Líneas que mueve cada paso de la rueda, como en moon.
const WHEEL: i32 = 3;

/// Captura del ratón para la rueda y el clic, como `Extras` en moon: si el terminal no
/// la acepta, la pantalla funciona igual con el teclado.
#[derive(Clone, Copy)]
struct Mouse(bool);

impl Mouse {
    fn enable() -> Self {
        Mouse(execute!(stdout(), EnableMouseCapture).is_ok())
    }

    fn disable(self) {
        if self.0 {
            let _ = execute!(stdout(), DisableMouseCapture);
        }
    }
}

fn event_loop(
    app: &mut App,
    terminal: &mut DefaultTerminal,
    theme: &Theme,
    msgs: &mpsc::Receiver<app::Msg>,
    updates: Option<&mpsc::Receiver<String>>,
    rescan: &mpsc::Sender<()>,
) -> Result<()> {
    loop {
        while let Ok(msg) = msgs.try_recv() {
            app.update(Action::Msg(msg));
        }
        if let Some(v) = updates.and_then(|rx| rx.try_recv().ok()) {
            app.update(Action::UpdateAvailable(v));
        }
        app.update(Action::Tick(Utc::now()));
        if app.take_rescan() {
            // Si el hilo ya no está, la pantalla sigue con lo que tiene.
            let _ = rescan.send(());
        }
        terminal.draw(|frame| view::render(app, frame, theme))?;
        if app.quit {
            return Ok(());
        }
        if event::poll(FRAME)? {
            match event::read()? {
                // Sin este filtro, en Windows cada tecla llegaría dos veces.
                Event::Key(k) if k.kind != KeyEventKind::Release => app.update(Action::Key(k)),
                Event::Mouse(m) => {
                    if let Some(action) = mouse_action(m) {
                        app.update(action);
                    }
                }
                _ => {}
            }
            // Tras cualquier evento se vuelve arriba del bucle y se repinta
            // en el acto, sin esperar a `FRAME`.
        }
    }
}

/// Qué hace cada evento del ratón: la rueda desplaza y el clic izquierdo
/// selecciona. Lo demás (movimiento, arrastre, soltar, otros botones) se
/// ignora; con la captura activa el terminal manda movimiento a cada paso.
fn mouse_action(m: MouseEvent) -> Option<Action> {
    match m.kind {
        MouseEventKind::ScrollUp => Some(Action::Scroll(-WHEEL)),
        MouseEventKind::ScrollDown => Some(Action::Scroll(WHEEL)),
        MouseEventKind::Down(MouseButton::Left) => Some(Action::Click {
            column: m.column,
            row: m.row,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn the_update_check_is_on_unless_the_variable_says_otherwise() {
        assert!(update_check_enabled(None));
        assert!(update_check_enabled(Some(OsStr::new(""))));
        assert!(update_check_enabled(Some(OsStr::new("0"))));
        assert!(!update_check_enabled(Some(OsStr::new("1"))));
        assert!(!update_check_enabled(Some(OsStr::new("yes"))));
    }

    #[test]
    fn the_update_check_hands_over_the_version_without_the_v() {
        let rx = spawn_update_check(|| "v1.1.0".parse().ok());
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).as_deref(),
            Ok("1.1.0")
        );
        // sin versión nueva (o sin red) no llega nada y el canal se cierra
        let none = spawn_update_check(|| None);
        assert!(none.recv_timeout(Duration::from_secs(5)).is_err());
    }

    #[test]
    fn left_click_becomes_click_on_the_same_cell() {
        let action = mouse_action(mouse(MouseEventKind::Down(MouseButton::Left), 7, 3));
        assert!(matches!(action, Some(Action::Click { column: 7, row: 3 })));
    }

    #[test]
    fn wheel_scrolls_three_lines() {
        let up = mouse_action(mouse(MouseEventKind::ScrollUp, 0, 0));
        let down = mouse_action(mouse(MouseEventKind::ScrollDown, 0, 0));
        assert!(matches!(up, Some(Action::Scroll(-3))));
        assert!(matches!(down, Some(Action::Scroll(3))));
    }

    #[test]
    fn other_mouse_events_do_nothing() {
        for kind in [
            MouseEventKind::Moved,
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::Down(MouseButton::Right),
            MouseEventKind::Down(MouseButton::Middle),
            MouseEventKind::ScrollLeft,
            MouseEventKind::ScrollRight,
        ] {
            assert!(mouse_action(mouse(kind, 1, 1)).is_none(), "{kind:?}");
        }
    }
}
