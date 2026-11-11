//! La pantalla del agente: lo que abre Enter sobre un bloque de activos. Las
//! ejecuciones del bloque, una por fila y numeradas de corrido, con un cursor
//! que elige cuál abrir en la pantalla de la ejecución. Aquí van su estado,
//! sus teclas y los datos ya resueltos que pinta la vista (`AgentView`); sin
//! terminal, como el resto de `app`.

use super::{now_line, App, NowLine};
use crate::status::{self, RunKind, RunState};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;

/// Estado de la pantalla del agente. No guarda datos que pintar: se rehacen
/// en cada refresco con `status::agent_block` (`App::agent_view`).
pub(super) struct AgentScreen {
    /// La ejecución bajo el cursor (`status::run_id`): un id y no un índice,
    /// así el cursor va con ella si cambia de sitio (termina y pasa a
    /// `finished`, o entra otra por encima).
    run: String,
    /// Los ids de todas las ejecuciones del bloque en el último refresco, en
    /// el orden de la pantalla. Sirven para mover el cursor y para volver a
    /// encontrar el bloque si la del cursor sale de la ventana.
    known: Vec<String>,
    /// Primera línea de `AgentView::items` que se pinta.
    scroll: usize,
    /// Alto del cuerpo en la última vuelta: lo que avanza una página.
    page: usize,
}

/// La pantalla del agente resuelta para pintar: la cabecera del bloque y sus
/// líneas. Datos propios, sin préstamos de `App`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentView {
    /// Nombre resuelto del agente.
    pub name: String,
    /// Origen que rotula el bloque en activos (el de su primera fila); cada
    /// ejecución lleva el suyo en `AgentRun::line`.
    pub kind: RunKind,
    /// Índice del agente definido (`App::agent`); `None` en built-in y pasos
    /// de workflow sin agente definido.
    pub owner: Option<usize>,
    /// Ruta absoluta del `.md` que lo define (`defined in`); `None` sin `owner`.
    pub definition: Option<PathBuf>,
    /// Clave del grupo: «global» o la carpeta acortada con `~`.
    pub section: String,
    /// Carpeta del grupo sin acortar (`running in`); `None` en «global».
    pub dir: Option<PathBuf>,
    /// Ejecuciones en curso (`N working now`).
    pub running: usize,
    /// Terminadas en la ventana (`N finished`); con `running` suman las de
    /// la pantalla (`N runs`).
    pub finished: usize,
    /// Tokens de todo el bloque.
    pub tokens: u64,
    /// Las líneas bajo la cabecera, en orden (`AgentItem`).
    pub items: Vec<AgentItem>,
    /// Número (`AgentRun::n`) de la ejecución bajo el cursor.
    pub cursor: usize,
}

/// Una línea de la pantalla del agente tal como se pinta. Solo `Run` es
/// seleccionable; `Now` y `Skill` son parte de su ejecución (`n`) para el
/// cursor, el scroll y el clic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentItem {
    /// Regla de sección: `working now · count` (`running`) o
    /// `finished · count`. Cada una solo si tiene filas, y primero la de en
    /// curso: la primera que salga lleva las cabeceras de las columnas.
    Section { running: bool, count: usize },
    /// Una ejecución: una por fila, también las terminadas. En caja: es
    /// mucho mayor que las demás líneas.
    Run(Box<AgentRun>),
    /// `now …`: la última herramienta de la ejecución `n` (la vista pinta
    /// `—` si falta). Siempre tras una fila en curso; nunca tras una terminada.
    Now { n: usize, last_tool: Option<String> },
    /// `skills …`: la última skill de la ejecución `n`, tras su `Now`; solo
    /// en curso y si usó alguna.
    Skill { n: usize, skill: String },
    /// Línea en blanco entre dos ejecuciones en curso.
    Blank,
}

impl AgentItem {
    /// La ejecución a la que pertenece la línea, si es de alguna.
    pub fn n(&self) -> Option<usize> {
        match self {
            AgentItem::Run(r) => Some(r.n),
            AgentItem::Now { n, .. } | AgentItem::Skill { n, .. } => Some(*n),
            AgentItem::Section { .. } | AgentItem::Blank => None,
        }
    }
}

/// Una ejecución de la pantalla del agente.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRun {
    /// Su número, 1..N de corrido por toda la pantalla: el de `run i of n`
    /// en la pantalla de la ejecución.
    pub n: usize,
    /// Sus datos, como una fila de activos: encargo, estado, tiempo, tokens,
    /// contexto, skill y última herramienta. Siempre una sola ejecución
    /// (`runs == 1`).
    pub line: NowLine,
    /// Es una llamada `Agent` sin ejecución enlazada: terminada y sin cifras
    /// que enseñar (0 trabajado, 0 tokens).
    pub lone_call: bool,
}

/// Primera línea que tiene que verse para que la ejecución de la línea
/// `line` se vea: todo lo de encima si es la primera, o su regla si abre
/// sección.
fn run_top(items: &[AgentItem], line: usize) -> usize {
    if !items[..line].iter().any(|i| matches!(i, AgentItem::Run(_))) {
        0
    } else if line >= 1 && matches!(items[line - 1], AgentItem::Section { .. }) {
        line - 1
    } else {
        line
    }
}

/// Última línea de la ejecución de la línea `line`: sus `now` y `skills`.
fn run_end(items: &[AgentItem], line: usize) -> usize {
    let mut end = line;
    while matches!(
        items.get(end + 1),
        Some(AgentItem::Now { .. } | AgentItem::Skill { .. })
    ) {
        end += 1;
    }
    end
}

/// Las líneas de las ejecuciones de `items`, en el orden de su número.
fn run_lines(items: &[AgentItem]) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter_map(|(l, i)| matches!(i, AgentItem::Run(_)).then_some(l))
        .collect()
}

impl App {
    /// El bloque desplegado del agente al que pertenece `id`, sobre los
    /// grupos sin filtrar: la pantalla del agente no depende de lo que se
    /// teclease en `/`.
    fn block_of(&self, id: &str) -> Option<status::AgentBlock<'_>> {
        status::agent_block(
            &self.agents,
            &self.calls,
            &self.runs,
            &self.groups(),
            id,
            self.now,
        )
        .filter(|b| !b.runs.is_empty())
    }

    /// Abre la pantalla del agente del bloque que contiene `id`, con el
    /// cursor en su primera ejecución; si el bloque no está en la ventana no
    /// hace nada. Devuelve si la abrió.
    pub(super) fn open_agent(&mut self, id: &str) -> bool {
        let Some(known) = self
            .block_of(id)
            .map(|b| b.runs.iter().map(|r| r.id.clone()).collect::<Vec<_>>())
        else {
            return false;
        };
        self.agent_screen = Some(AgentScreen {
            run: known[0].clone(),
            known,
            scroll: 0,
            page: self.agent_screen.as_ref().map_or(0, |s| s.page),
        });
        self.pending = None;
        true
    }

    /// Pone al día la pantalla del agente con los datos actuales (cada
    /// `Tick`, cada `Msg::Activity` y tras cada tecla).
    ///
    /// El bloque se busca por la ejecución del cursor; si ya no está en la
    /// ventana, por cualquiera de las otras que se le conocían. Si el cursor
    /// perdió su ejecución, pasa a la que ocupa su puesto (o a la última).
    /// Si ninguna da con el bloque, este salió entero de la ventana: no
    /// queda nada veraz que enseñar y la pantalla se cierra, de vuelta a
    /// activos (con la pantalla de la ejecución abierta encima, esta sigue
    /// con su última foto y su Esc ya vuelve a activos).
    ///
    /// De paso deja el cursor de la lista en el bloque, si el filtro lo
    /// deja ver: es donde vuelve Esc y el agente de la ficha de `a`.
    pub(super) fn refresh_agent(&mut self) {
        let Some(s) = &self.agent_screen else { return };
        let found = std::iter::once(&s.run)
            .chain(&s.known)
            .find_map(|id| self.block_of(id))
            .map(|b| b.runs.iter().map(|r| r.id.clone()).collect::<Vec<_>>());
        let Some(ids) = found else {
            self.agent_screen = None;
            return;
        };
        let run = if ids.contains(&s.run) {
            s.run.clone()
        } else {
            let was = s.known.iter().position(|i| *i == s.run).unwrap_or(0);
            ids[was.min(ids.len() - 1)].clone()
        };
        let block = self.live_line_of(&run).map(|(n, _, _)| n);
        if let Some(s) = &mut self.agent_screen {
            s.run = run;
            s.known = ids;
        }
        if let Some(n) = block {
            self.cursor = n - 1;
        }
    }

    /// Lo que pinta la pantalla del agente, o `None` si no está abierta.
    /// Primero `working now` y luego `finished`, cada sección solo si tiene
    /// filas; una ejecución por fila en el orden de `status::agent_block`
    /// (las en curso como en activos, las terminadas de la más reciente a
    /// la más antigua), numeradas de corrido. Cada en curso lleva debajo su
    /// `Now` y, si usó alguna skill, su `Skill`; entre dos en curso, un
    /// `Blank`. Las terminadas son una sola línea.
    pub fn agent_view(&self) -> Option<AgentView> {
        let s = self.agent_screen.as_ref()?;
        let b = self.block_of(&s.run)?;
        let mut items = Vec::new();
        let mut cursor = 1;
        for (i, r) in b.runs.iter().enumerate() {
            let n = i + 1;
            let running = r.state == RunState::Running;
            if r.id == s.run {
                cursor = n;
            }
            if i == 0 || (!running && i == b.running) {
                items.push(AgentItem::Section {
                    running,
                    count: if running { b.running } else { b.finished },
                });
            } else if running {
                items.push(AgentItem::Blank);
            }
            let line = now_line(r);
            items.push(AgentItem::Run(Box::new(AgentRun {
                n,
                lone_call: r.lone_call,
                line: line.clone(),
            })));
            if running {
                items.push(AgentItem::Now {
                    n,
                    last_tool: line.last_tool,
                });
                if let Some(skill) = line.skill {
                    items.push(AgentItem::Skill { n, skill });
                }
            }
        }
        Some(AgentView {
            name: b.name.to_string(),
            kind: b.kind,
            owner: b.owner,
            definition: b.definition.map(PathBuf::from),
            section: b.section,
            dir: b.dir.map(PathBuf::from),
            running: b.running,
            finished: b.finished,
            tokens: b.tokens,
            items,
            cursor,
        })
    }

    /// Las dos pantallas de activos (la del agente y la de la ejecución) van
    /// a pantalla completa: la vista no pinta la fila de pestañas ni la
    /// segunda línea. La ficha del agente (`a`) sí las lleva, aunque se
    /// abriese desde la pantalla del agente.
    pub fn hides_tabs(&self) -> bool {
        self.run_detail.is_some() || (self.agent_screen.is_some() && !self.detail)
    }

    /// Desde la pantalla del agente se puede abrir su ficha de siempre: es
    /// un agente definido (`owner`) y su bloque se ve en la lista (la ficha
    /// es la de la fila del cursor). El pie solo anuncia `a agent` si es cierto.
    pub fn can_open_agent(&self) -> bool {
        self.agent_screen.as_ref().is_some_and(|s| {
            self.live_line_of(&s.run)
                .is_some_and(|(_, _, row)| row.is_some())
        })
    }

    /// Posición del bloque abierto entre los que se ven en activos (los que
    /// deja el filtro), en el orden de pantalla: (número, total). `None` si
    /// la pantalla no está abierta o el filtro ya no deja ver su bloque. El
    /// pie anuncia `←→ other agent` solo si el total pasa de 1.
    pub fn agent_nav(&self) -> Option<(usize, usize)> {
        let s = self.agent_screen.as_ref()?;
        let n = self.live_line_of(&s.run)?.0;
        let lines = self.live_lines();
        let pos = lines.iter().position(|(m, _, _)| *m == n)?;
        Some((pos + 1, lines.len()))
    }

    /// A dónde vuelve Esc desde la pantalla de la ejecución, para el pie
    /// (`esc back to <agente>`): el agente de la ejecución desde la que se
    /// abrió esta (una hija o la madre) o, si no hay, el de la pantalla del
    /// agente. `None` si Esc vuelve a activos (el bloque salió de la
    /// ventana) o no hay ejecución abierta.
    pub fn esc_target(&self) -> Option<String> {
        self.run_detail.as_ref()?;
        if let Some(back) = self.run_back.last() {
            return Some(back.detail.name.clone());
        }
        let s = self.agent_screen.as_ref()?;
        self.block_of(&s.run).map(|b| b.name.to_string())
    }

    /// El cursor de la pantalla del agente pasa a `id` si es una de sus
    /// ejecuciones (←→ en la pantalla de la ejecución).
    pub(super) fn agent_follow(&mut self, id: &str) {
        if let Some(s) = &mut self.agent_screen {
            if s.known.iter().any(|i| i == id) {
                s.run = id.to_string();
            }
        }
    }

    /// Mueve el cursor a la ejecución de índice `i` (desde 0), acotado.
    fn agent_select(&mut self, i: usize) {
        if let Some(s) = &mut self.agent_screen {
            if let Some(id) = s.known.get(i.min(s.known.len().saturating_sub(1))) {
                s.run = id.clone();
            }
        }
    }

    /// Índice (desde 0) de la ejecución bajo el cursor y cuántas hay.
    fn agent_cursor(&self) -> (usize, usize) {
        self.agent_screen.as_ref().map_or((0, 0), |s| {
            (
                s.known.iter().position(|i| *i == s.run).unwrap_or(0),
                s.known.len(),
            )
        })
    }

    /// ←→ en la pantalla del agente: el bloque anterior o siguiente de los
    /// que se ven en activos (con filtro, solo los que deja), en el orden de
    /// pantalla, sin salir. Se abre en su primera ejecución y el cursor de
    /// la lista lo sigue; en los extremos no se mueve.
    fn step_block(&mut self, dir: i64) {
        let Some(s) = &self.agent_screen else { return };
        let Some((n, _, _)) = self.live_line_of(&s.run) else {
            return;
        };
        let lines = self.live_lines();
        let Some(pos) = lines.iter().position(|(m, _, _)| *m == n) else {
            return;
        };
        let next = if dir > 0 {
            lines.get(pos + 1)
        } else {
            pos.checked_sub(1).and_then(|i| lines.get(i))
        };
        if let Some((n, line, _)) = next {
            let (n, id) = (*n, line.id.clone());
            if self.open_agent(&id) {
                self.cursor = n - 1;
            }
        }
    }

    /// Teclas de la pantalla del agente. Se come todo lo que no es suyo (el
    /// filtro, la pestaña…): la lista de debajo no cambia mientras se mira.
    pub(super) fn agent_key(&mut self, k: KeyEvent) {
        let (at, total) = self.agent_cursor();
        if let Some(d) = match k.code {
            KeyCode::Char(c) => c.to_digit(10),
            _ => None,
        } {
            if let Some(n) = self.type_digit(d as usize, total) {
                self.agent_select(n - 1);
            }
            return;
        }
        self.pending = None;
        let page = self.agent_screen.as_ref().map_or(1, |s| s.page.max(1)) as i64;
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => {
                // de vuelta a activos, con el cursor de la lista en el bloque
                self.refresh_agent();
                self.agent_screen = None;
            }
            KeyCode::Enter => {
                let id = self.agent_screen.as_ref().map(|s| s.run.clone());
                if let Some(id) = id {
                    self.run_back.clear();
                    self.open_run(&id);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.agent_select(at + 1),
            KeyCode::Up | KeyCode::Char('k') => self.agent_select(at.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => self.agent_select(0),
            KeyCode::End | KeyCode::Char('G') => self.agent_select(total.saturating_sub(1)),
            KeyCode::Left | KeyCode::Char('h') => self.step_block(-1),
            KeyCode::Right | KeyCode::Char('l') => self.step_block(1),
            KeyCode::PageDown | KeyCode::Char(' ') => self.agent_scroll_by(page),
            KeyCode::PageUp => self.agent_scroll_by(-page),
            KeyCode::Char('a') if self.can_open_agent() => self.detail = true,
            KeyCode::Char('r') => self.rescan = true,
            _ => {}
        }
    }

    /// Clic en la pantalla del agente: sobre una línea de una ejecución
    /// (`Hits::rows`, con `cursor` = su número menos 1) la elige y, si ya
    /// era la del cursor, la abre como Enter. Fuera de ellas, nada.
    pub(super) fn agent_click(&mut self, row: u16) {
        let Some(i) = self
            .hits
            .rows
            .iter()
            .find(|h| h.row == row)
            .map(|h| h.cursor)
        else {
            return;
        };
        let (at, total) = self.agent_cursor();
        if i >= total {
            return;
        }
        if i == at {
            let id = self.agent_screen.as_ref().map(|s| s.run.clone());
            if let Some(id) = id {
                self.run_back.clear();
                self.open_run(&id);
            }
        } else {
            self.agent_select(i);
        }
    }

    /// Para la vista: guarda `height` (líneas de `items` que caben bajo la
    /// cabecera) como alto de página, mueve el desplazamiento lo justo para
    /// que la ejecución número `cursor` se vea entera (con su regla si abre
    /// sección; si es más alta que la vista, su principio) y devuelve la
    /// primera línea de `items` a pintar.
    pub fn agent_window(&mut self, items: &[AgentItem], cursor: usize, height: usize) -> usize {
        let Some(s) = &mut self.agent_screen else {
            return 0;
        };
        s.page = height;
        let line = items
            .iter()
            .position(|i| matches!(i, AgentItem::Run(r) if r.n == cursor));
        let Some(line) = line else {
            s.scroll = 0;
            return 0;
        };
        let (top, end) = (run_top(items, line), run_end(items, line));
        if top < s.scroll {
            s.scroll = top;
        } else if height > 0 && end >= s.scroll + height {
            s.scroll = (end + 1 - height).min(top);
        }
        s.scroll = s.scroll.min(items.len().saturating_sub(height));
        s.scroll
    }

    /// Mueve la vista de la pantalla del agente `delta` líneas (la rueda y
    /// las páginas) y arrastra el cursor a una ejecución que se vea entera
    /// si la suya se queda fuera.
    pub(super) fn agent_scroll_by(&mut self, delta: i64) {
        let Some(view) = self.agent_view() else {
            return;
        };
        let Some(s) = &mut self.agent_screen else {
            return;
        };
        let items = &view.items;
        let page = s.page.max(1);
        let max = items.len().saturating_sub(page) as i64;
        s.scroll = (s.scroll as i64 + delta).clamp(0, max) as usize;
        let (top, bottom) = (s.scroll, s.scroll + page);
        let lines = run_lines(items);
        let Some(&line) = lines.get(view.cursor - 1) else {
            return;
        };
        let fits = |l: usize| run_top(items, l) >= top && run_end(items, l) < bottom;
        let dragged = if run_top(items, line) < top {
            lines.iter().position(|&l| fits(l))
        } else if run_end(items, line) >= bottom {
            lines.iter().rposition(|&l| fits(l))
        } else {
            None
        };
        if let Some(i) = dragged {
            self.agent_select(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{
        active, busy, family, fixture, fused, key, open_id, type_filter, with_runs,
    };
    use super::super::{Action, App, Mode, RowHit};
    use super::*;
    use crate::runs::Run;
    use ratatui::crossterm::event::KeyCode;
    use std::path::Path;

    /// (número, id) de cada ejecución de la pantalla del agente, en orden.
    fn runs(app: &App) -> Vec<(usize, String)> {
        app.agent_view()
            .expect("la pantalla del agente está abierta")
            .items
            .into_iter()
            .filter_map(|i| match i {
                AgentItem::Run(r) => Some((r.n, r.line.id)),
                _ => None,
            })
            .collect()
    }

    /// El id de la ejecución bajo el cursor de la pantalla del agente.
    fn at_cursor(app: &App) -> String {
        let v = app.agent_view().expect("abierta");
        runs(app)
            .into_iter()
            .find(|(n, _)| *n == v.cursor)
            .expect("el cursor está en una ejecución")
            .1
    }

    /// Las ejecuciones de `app` con `change` aplicado a la de `tool`.
    fn changed(app: &App, tool: &str, change: impl Fn(&mut Run)) -> Vec<Run> {
        let mut runs = app.runs.clone();
        for r in &mut runs {
            if r.tool_use_id.as_deref() == Some(tool) {
                change(r);
            }
        }
        runs
    }

    /// Las ejecuciones de `app` sin las de `tools`.
    fn without(app: &App, tools: &[&str]) -> Vec<Run> {
        app.runs
            .iter()
            .filter(|r| !tools.contains(&r.tool_use_id.as_deref().unwrap_or("")))
            .cloned()
            .collect()
    }

    #[test]
    fn enter_on_a_block_opens_the_agent_screen_on_its_first_run() {
        let mut app = fused();
        assert!(app.agent_view().is_none() && !app.hides_tabs());
        key(&mut app, KeyCode::Enter);
        assert!(app.run_detail.is_none(), "ya no abre la ejecución");
        assert!(!app.detail);
        assert!(app.hides_tabs(), "sin pestañas ni segunda línea");
        let v = app.agent_view().expect("abre la pantalla del agente");
        assert_eq!(
            (v.name.as_str(), v.kind, v.owner.is_some()),
            ("backend", RunKind::Defined, true)
        );
        assert_eq!(
            v.definition.as_deref(),
            Some(Path::new(
                "/Users/x/Offing/argi/station/.claude/agents/backend.md"
            ))
        );
        assert_eq!(v.section, "~/Offing/argi/station");
        assert_eq!(
            v.dir.as_deref(),
            Some(Path::new("/Users/x/Offing/argi/station"))
        );
        assert_eq!((v.running, v.finished, v.cursor), (2, 2, 1));
        // una por ejecución, numeradas de corrido: las terminadas no se funden
        assert_eq!(
            runs(&app),
            vec![
                (1, "toolu_b3".to_string()),
                (2, "toolu_b4".to_string()),
                (3, "toolu_b2".to_string()),
                (4, "toolu_b1".to_string()),
            ]
        );
    }

    /// Secciones solo con filas; cada en curso con su `now` (y su `skill` si
    /// la tiene), una línea en blanco entre las en curso y las terminadas en
    /// una línea.
    #[test]
    fn the_screen_lists_working_now_and_finished_with_their_sub_lines() {
        let mut app = fused();
        let runs = changed(&app, "toolu_b4", |r| {
            r.skill = Some("frontend-design".into());
            r.last_tool = Some("Bash · git diff --stat".into());
        });
        app = with_runs(app, runs);
        key(&mut app, KeyCode::Enter);
        let shape: Vec<String> = app
            .agent_view()
            .unwrap()
            .items
            .iter()
            .map(|i| match i {
                AgentItem::Section { running, count } => format!("section {running} {count}"),
                AgentItem::Run(r) => format!("run {} {}", r.n, r.line.id),
                AgentItem::Now { n, last_tool } => {
                    format!("now {n} {}", last_tool.as_deref().unwrap_or("-"))
                }
                AgentItem::Skill { n, skill } => format!("skill {n} {skill}"),
                AgentItem::Blank => "blank".to_string(),
            })
            .collect();
        assert_eq!(
            shape,
            [
                "section true 2",
                "run 1 toolu_b3",
                "now 1 -",
                "blank",
                "run 2 toolu_b4",
                "now 2 Bash · git diff --stat",
                "skill 2 frontend-design",
                "section false 2",
                "run 3 toolu_b2",
                "run 4 toolu_b1",
            ]
        );
        // un bloque solo terminado no lleva sección de en curso
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('2'));
        key(&mut app, KeyCode::Enter);
        let v = app.agent_view().unwrap();
        assert_eq!((v.name.as_str(), v.running, v.finished), ("frontend", 0, 1));
        assert!(matches!(
            v.items[..],
            [
                AgentItem::Section {
                    running: false,
                    count: 1
                },
                AgentItem::Run(_)
            ]
        ));
    }

    #[test]
    fn the_header_sums_the_tokens_of_the_block() {
        let mut app = fused();
        let runs = changed(&app, "toolu_b1", |r| r.usage.tokens.output = 500);
        app = with_runs(app, runs);
        let runs = changed(&app, "toolu_b3", |r| r.usage.tokens.output = 1500);
        app = with_runs(app, runs);
        key(&mut app, KeyCode::Enter);
        let v = app.agent_view().unwrap();
        assert_eq!(v.tokens, 2000);
        let tokens: Vec<u64> = v
            .items
            .iter()
            .filter_map(|i| match i {
                AgentItem::Run(r) => Some(r.line.tokens),
                _ => None,
            })
            .collect();
        assert_eq!(tokens, [1500, 0, 0, 500], "cada fila, las suyas");
    }

    #[test]
    fn arrows_digits_home_and_end_choose_a_run() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Up);
        assert_eq!(at_cursor(&app), "toolu_b3", "en el extremo no se mueve");
        key(&mut app, KeyCode::Down);
        assert_eq!(at_cursor(&app), "toolu_b4");
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Down);
        assert_eq!(at_cursor(&app), "toolu_b1", "tampoco por abajo");
        key(&mut app, KeyCode::Char('k'));
        assert_eq!(at_cursor(&app), "toolu_b2");
        key(&mut app, KeyCode::Home);
        assert_eq!(at_cursor(&app), "toolu_b3");
        key(&mut app, KeyCode::End);
        assert_eq!(at_cursor(&app), "toolu_b1");
        key(&mut app, KeyCode::Char('2'));
        assert_eq!(at_cursor(&app), "toolu_b4");
        key(&mut app, KeyCode::Char('7'));
        assert_eq!(at_cursor(&app), "toolu_b4", "fuera de rango no mueve");
        // lo demás se lo come: ni filtro ni pestaña, y el cursor de la lista no se mueve
        key(&mut app, KeyCode::Char('/'));
        key(&mut app, KeyCode::Tab);
        assert!(!app.filtering && app.mode == Mode::Active && app.cursor == 0);
        assert!(app.agent_view().is_some());
    }

    #[test]
    fn enter_opens_the_run_under_the_cursor_and_esc_goes_back_a_level_each_time() {
        let mut app = fused();
        key(&mut app, KeyCode::Char('2'));
        key(&mut app, KeyCode::Char('1'));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('3'));
        key(&mut app, KeyCode::Enter);
        assert_eq!(open_id(&app), "toolu_b2");
        assert_eq!(app.run_nav(), Some((3, 4)), "run 3 of 4");
        assert!(app.hides_tabs());
        assert_eq!(app.esc_target().as_deref(), Some("backend"));
        key(&mut app, KeyCode::Esc);
        assert!(app.run_detail.is_none());
        assert_eq!(
            at_cursor(&app),
            "toolu_b2",
            "vuelve a la pantalla del agente"
        );
        assert!(app.hides_tabs());
        app.cursor = 1; // fuera de sitio a propósito
        key(&mut app, KeyCode::Esc);
        assert!(app.agent_view().is_none() && !app.hides_tabs());
        assert_eq!(app.cursor, 0, "a activos, con el cursor en su bloque");
        assert_eq!(app.esc_target(), None);
    }

    /// → siguiente y ← anterior, según la numeración de la pantalla del
    /// agente; paran en los extremos y el cursor de esa pantalla las sigue.
    #[test]
    fn left_and_right_in_the_run_walk_the_runs_of_the_agent_in_screen_order() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Left);
        assert_eq!(
            (open_id(&app).as_str(), app.run_nav()),
            ("toolu_b3", Some((1, 4))),
            "en el extremo no se mueve"
        );
        for (id, pos) in [("toolu_b4", 2), ("toolu_b2", 3), ("toolu_b1", 4)] {
            key(&mut app, KeyCode::Right);
            assert_eq!(
                (open_id(&app).as_str(), app.run_nav(), app.cursor),
                (id, Some((pos, 4)), 0)
            );
        }
        key(&mut app, KeyCode::Char('l'));
        assert_eq!(open_id(&app), "toolu_b1", "en el extremo no se mueve");
        key(&mut app, KeyCode::Char('h'));
        assert_eq!(open_id(&app), "toolu_b2");
        key(&mut app, KeyCode::Esc);
        assert_eq!(at_cursor(&app), "toolu_b2", "el cursor siguió a la abierta");
    }

    #[test]
    fn up_and_down_in_the_run_no_longer_change_agent() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        for code in [KeyCode::Down, KeyCode::Up, KeyCode::Char('j')] {
            key(&mut app, code);
            assert_eq!((open_id(&app).as_str(), app.cursor), ("toolu_b3", 0));
        }
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.agent_view().unwrap().name, "backend");
    }

    #[test]
    fn up_and_down_in_the_run_move_the_steps_a_line() {
        let mut app = fused();
        let runs = changed(&app, "toolu_b3", |r| {
            r.steps = (0..5)
                .map(|i| crate::runs::ToolStep {
                    at: "2026-11-05T11:58:00Z".parse().unwrap(),
                    name: "Bash".into(),
                    arg: Some(format!("cmd {i}")),
                })
                .collect::<Vec<_>>()
                .into();
        });
        app = with_runs(app, runs);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.steps_window(2), 0);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Down);
        assert_eq!(app.steps_scroll, 2);
        key(&mut app, KeyCode::Up);
        assert_eq!(app.steps_scroll, 1);
    }

    /// ←→ en la pantalla del agente: el bloque anterior o siguiente de
    /// activos, en el orden de pantalla; el cursor de la lista lo sigue.
    #[test]
    fn left_and_right_in_the_agent_screen_walk_the_blocks_of_active() {
        let mut app = active(busy());
        key(&mut app, KeyCode::Enter);
        let name = |a: &App| a.agent_view().unwrap().name;
        assert_eq!(
            (name(&app).as_str(), app.agent_nav()),
            ("visual", Some((1, 8)))
        );
        key(&mut app, KeyCode::Left);
        assert_eq!(name(&app), "visual", "en el extremo no se mueve");
        for (i, want) in [
            "react-structure-checker",
            "Explore",
            "workflow-subagent: tarea",
            "backend",
            "frontend",
            "general-purpose",
            "docs",
        ]
        .into_iter()
        .enumerate()
        {
            key(&mut app, KeyCode::Right);
            assert_eq!((name(&app).as_str(), app.cursor), (want, i + 1));
            assert_eq!(app.agent_view().unwrap().cursor, 1, "en su primera");
        }
        key(&mut app, KeyCode::Char('l'));
        assert_eq!(name(&app), "docs", "tampoco por el final");
        key(&mut app, KeyCode::Char('h'));
        assert_eq!((name(&app).as_str(), app.cursor), ("general-purpose", 6));
        key(&mut app, KeyCode::Esc);
        assert_eq!(
            app.cursor, 6,
            "Esc deja el cursor en el último bloque visto"
        );
    }

    /// Con filtro, ←→ recorren solo los bloques que se ven en activos; la
    /// pantalla del agente enseña todas sus ejecuciones, encajen o no.
    #[test]
    fn with_a_filter_left_and_right_walk_only_the_blocks_it_leaves() {
        let mut app = active(busy());
        type_filter(&mut app, "station");
        // station: 1 Explore, 2 workflow, 3 backend, 4 frontend; station/front: 5 docs
        key(&mut app, KeyCode::Char('4'));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.agent_nav(), Some((4, 5)));
        key(&mut app, KeyCode::Right);
        assert_eq!(app.agent_view().unwrap().name, "docs");
        key(&mut app, KeyCode::Right);
        assert_eq!(app.agent_view().unwrap().name, "docs");
        assert_eq!(app.filter, "station", "el filtro sigue");
        let mut app = fused();
        type_filter(&mut app, "tests");
        key(&mut app, KeyCode::Enter);
        assert_eq!(runs(&app).len(), 4, "todas las del bloque, sin filtrar");
        assert_eq!(at_cursor(&app), "toolu_b3", "en la primera del bloque");
        assert_eq!(app.agent_nav(), Some((1, 1)));
    }

    #[test]
    fn a_opens_the_agent_card_only_with_an_owner_and_esc_returns_to_the_screen() {
        let mut app = active(busy());
        key(&mut app, KeyCode::Char('4'));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.agent_view().unwrap().name, "workflow-subagent: tarea");
        assert!(!app.can_open_agent());
        key(&mut app, KeyCode::Char('a'));
        assert!(!app.detail && app.mode == Mode::Active && app.agent_view().is_some());
        key(&mut app, KeyCode::Right);
        assert_eq!(app.agent_view().unwrap().name, "backend");
        assert!(app.can_open_agent());
        key(&mut app, KeyCode::Char('a'));
        assert!(app.detail && app.mode == Mode::Active);
        assert!(!app.hides_tabs(), "la ficha de siempre lleva sus pestañas");
        assert_eq!(app.agent(app.current().unwrap().agent).name, "backend");
        key(&mut app, KeyCode::Esc);
        assert!(!app.detail && app.hides_tabs());
        assert_eq!(
            app.agent_view().unwrap().name,
            "backend",
            "vuelve a su pantalla"
        );
        // ↑↓ en la ficha cambian de agente: la pantalla del agente se deja
        key(&mut app, KeyCode::Char('a'));
        key(&mut app, KeyCode::Down);
        assert!(app.detail);
        assert_eq!(app.agent(app.current().unwrap().agent).name, "frontend");
        key(&mut app, KeyCode::Esc);
        assert!(!app.detail && app.agent_view().is_none() && !app.hides_tabs());
        assert_eq!(app.cursor, 5);
    }

    /// Una llamada sin ejecución enlazada es una terminada más, marcada, y
    /// abre su ficha mínima.
    #[test]
    fn a_call_without_a_run_is_a_finished_row_that_opens_its_minimal_detail() {
        let mut app = active(fixture());
        key(&mut app, KeyCode::Char('3'));
        key(&mut app, KeyCode::Enter);
        let v = app.agent_view().unwrap();
        assert_eq!((v.name.as_str(), v.running, v.finished), ("frontend", 0, 1));
        let AgentItem::Run(r) = &v.items[1] else {
            panic!("tras la regla, su fila")
        };
        assert!(r.lone_call);
        assert!(app.can_open_agent());
        key(&mut app, KeyCode::Enter);
        let d = app.run_detail.as_ref().expect("abre su ficha mínima");
        assert_eq!(d.name, "frontend");
        assert_eq!((d.tools, d.steps.len()), (0, 0));
        key(&mut app, KeyCode::Esc);
        assert!(app.run_detail.is_none() && app.agent_view().is_some());
    }

    /// Se refresca en vivo: la ejecución del cursor termina y pasa a
    /// `finished`; el cursor va con ella (guarda su id, no su posición).
    #[test]
    fn a_run_that_finishes_changes_section_and_keeps_the_cursor() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        assert_eq!(at_cursor(&app), "toolu_b3");
        let runs = changed(&app, "toolu_b3", |r| r.finished = Some(true));
        app = with_runs(app, runs);
        app.update(Action::Tick("2026-11-05T12:00:30Z".parse().unwrap()));
        let v = app.agent_view().expect("sigue abierta");
        assert_eq!((v.running, v.finished, v.cursor), (1, 3, 2));
        assert_eq!(
            runs_ids(&app),
            ["toolu_b4", "toolu_b3", "toolu_b2", "toolu_b1"]
        );
        assert_eq!(at_cursor(&app), "toolu_b3");
    }

    fn runs_ids(app: &App) -> Vec<String> {
        runs(app).into_iter().map(|(_, id)| id).collect()
    }

    /// La ejecución del cursor sale de la ventana pero el bloque sigue: se
    /// reintenta con las otras conocidas y el cursor queda en su sitio.
    #[test]
    fn when_the_run_under_the_cursor_leaves_the_block_is_found_by_another_one() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::End);
        assert_eq!(at_cursor(&app), "toolu_b1");
        let left = without(&app, &["toolu_b1"]);
        app = with_runs(app, left);
        assert_eq!(runs_ids(&app), ["toolu_b3", "toolu_b4", "toolu_b2"]);
        assert_eq!(at_cursor(&app), "toolu_b2", "la última que queda");
        key(&mut app, KeyCode::Home);
        let left = without(&app, &["toolu_b3"]);
        app = with_runs(app, left);
        assert_eq!(at_cursor(&app), "toolu_b4", "la que ocupa su puesto");
    }

    /// El bloque entero sale de la ventana: no queda nada veraz que enseñar
    /// y se vuelve a activos.
    #[test]
    fn when_the_block_leaves_the_window_the_screen_goes_back_to_active() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        let left = without(&app, &["toolu_b1", "toolu_b2", "toolu_b3", "toolu_b4"]);
        app = with_runs(app, left);
        assert!(app.agent_view().is_none() && !app.hides_tabs());
        assert_eq!(app.cursor, 0, "el cursor queda en una fila que existe");
        // y con el paso del tiempo, igual: la ventana es de 60 min
        let mut app = fused();
        key(&mut app, KeyCode::Char('2'));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.agent_view().unwrap().name, "frontend");
        app.update(Action::Tick("2026-11-05T12:50:00Z".parse().unwrap()));
        assert!(app.agent_view().is_none(), "frontend terminó 11:45");
    }

    /// Con la ejecución abierta, si el bloque sale de la ventana la ficha
    /// conserva su última foto y Esc ya vuelve a activos.
    #[test]
    fn an_open_run_outlives_its_block_and_esc_then_returns_to_active() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        let left = without(&app, &["toolu_b1", "toolu_b2", "toolu_b3", "toolu_b4"]);
        app = with_runs(app, left);
        assert_eq!(open_id(&app), "toolu_b3", "conserva la foto");
        assert_eq!((app.run_nav(), app.esc_target()), (None, None));
        key(&mut app, KeyCode::Left);
        key(&mut app, KeyCode::Right);
        assert_eq!(open_id(&app), "toolu_b3");
        key(&mut app, KeyCode::Esc);
        assert!(app.run_detail.is_none() && app.agent_view().is_none());
    }

    /// Una hija abierta desde `launched` vuelve con Esc a su madre y de ahí
    /// a la pantalla del agente, que no se ha movido.
    #[test]
    fn a_launched_run_goes_back_to_its_parent_and_then_to_the_agent_screen() {
        let mut app = family();
        let n = app
            .items()
            .into_iter()
            .find_map(|i| match i {
                super::super::Item::LiveAgent { n, line, .. } if line.id == "toolu_p" => Some(n),
                _ => None,
            })
            .unwrap();
        app.cursor = n - 1;
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        assert_eq!(open_id(&app), "toolu_p");
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Enter);
        assert_eq!(open_id(&app), "toolu_c2");
        assert_eq!(
            app.esc_target().as_deref(),
            Some("backend"),
            "Esc vuelve a la ejecución de backend"
        );
        assert_eq!(app.run_nav(), Some((1, 1)), "sobre el bloque de docs");
        assert_eq!(
            app.agent_view().unwrap().name,
            "backend",
            "debajo, sin moverse"
        );
        key(&mut app, KeyCode::Esc);
        assert_eq!(open_id(&app), "toolu_p");
        key(&mut app, KeyCode::Esc);
        assert!(app.run_detail.is_none());
        assert_eq!(at_cursor(&app), "toolu_p");
    }

    /// Un clic elige la ejecución y otro sobre la misma la abre.
    #[test]
    fn a_click_chooses_a_run_and_a_second_one_opens_it() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        // como lo deja la vista: cada línea de una ejecución, a su índice
        app.hits.rows = vec![
            RowHit { row: 5, cursor: 0 },
            RowHit { row: 6, cursor: 0 },
            RowHit { row: 8, cursor: 1 },
            RowHit { row: 9, cursor: 1 },
        ];
        app.update(Action::Click { column: 3, row: 9 });
        assert_eq!(at_cursor(&app), "toolu_b4");
        assert!(app.run_detail.is_none());
        app.update(Action::Click { column: 3, row: 2 });
        assert_eq!(at_cursor(&app), "toolu_b4", "fuera de las filas, nada");
        app.update(Action::Click { column: 3, row: 8 });
        assert_eq!(open_id(&app), "toolu_b4");
        assert_eq!(
            app.cursor, 0,
            "el cursor de la lista no es el de la pantalla"
        );
    }

    /// Si no cabe, la vista se desplaza con el cursor: una ejecución entra
    /// entera (con sus sublíneas) y la primera de una sección, con su regla.
    #[test]
    fn the_screen_scrolls_to_keep_the_run_under_the_cursor_in_view() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        // regla, b3, now, blanco, b4, now, regla, b2, b1
        let frame = |app: &mut App, h: usize| {
            let v = app.agent_view().unwrap();
            assert_eq!(v.items.len(), 9);
            app.agent_window(&v.items, v.cursor, h)
        };
        assert_eq!(frame(&mut app, 4), 0);
        key(&mut app, KeyCode::Down);
        assert_eq!(frame(&mut app, 4), 2, "b4 con su now: líneas 4 y 5");
        key(&mut app, KeyCode::End);
        assert_eq!(frame(&mut app, 4), 5);
        key(&mut app, KeyCode::Char('3'));
        assert_eq!(frame(&mut app, 4), 5, "b2 ya se ve con su regla");
        key(&mut app, KeyCode::Home);
        assert_eq!(frame(&mut app, 4), 0, "la primera, con su regla");
        assert_eq!(frame(&mut app, 20), 0, "si cabe todo no hay desplazamiento");
    }

    /// La rueda mueve la vista y arrastra el cursor a una ejecución que se
    /// vea entera.
    #[test]
    fn the_wheel_moves_the_view_and_drags_the_cursor() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        let v = app.agent_view().unwrap();
        assert_eq!(app.agent_window(&v.items, v.cursor, 4), 0);
        app.update(Action::Scroll(3));
        // se ven: blanco, b4, now, regla
        assert_eq!(at_cursor(&app), "toolu_b4");
        let v = app.agent_view().unwrap();
        assert_eq!(app.agent_window(&v.items, v.cursor, 4), 3);
        app.update(Action::Scroll(-3));
        assert_eq!(at_cursor(&app), "toolu_b3");
        // con la ejecución abierta, la rueda no mueve nada
        key(&mut app, KeyCode::Enter);
        app.update(Action::Scroll(3));
        key(&mut app, KeyCode::Esc);
        assert_eq!(at_cursor(&app), "toolu_b3");
    }

    #[test]
    fn q_quits_from_both_screens() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }
}
