//! Pantalla de terminal al estilo de moon: lista viva de agentes por proyecto,
//! que se relee sola mientras está abierta.

pub mod app;
pub mod scan;
pub mod theme;
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
    let result = event_loop(&mut app, &mut terminal, &theme, &msgs, &rescan_tx);
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
    rescan: &mpsc::Sender<()>,
) -> Result<()> {
    loop {
        while let Ok(msg) = msgs.try_recv() {
            app.update(Action::Msg(msg));
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
