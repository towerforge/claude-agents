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
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseEventKind,
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

/// Captura del ratón para la rueda, como `Extras` en moon: si el terminal no
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
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollUp => app.update(Action::Scroll(-WHEEL)),
                    MouseEventKind::ScrollDown => app.update(Action::Scroll(WHEEL)),
                    _ => {}
                },
                _ => {}
            }
        }
    }
}
