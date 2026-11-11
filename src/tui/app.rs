//! Estado de la pantalla y su actualización: sin terminal ni I/O, para poder
//! probarlo con teclas y mensajes construidos a mano.

use crate::agents::{Agent, Scope};
use crate::report;
use crate::runs::Run;
use crate::status::{
    self, Chain, Context, LaunchedBy, Period, PeriodStats, RankKey, RunDetail, RunKind, RunState,
    Work,
};
use crate::transcripts::Invocation;
use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime, TimeDelta, Utc};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub mod agent;

pub use agent::{AgentItem, AgentView};

/// Días del gráfico de la ficha, contando hoy.
pub const DAYS: usize = 14;

/// Días locales que guarda cada fila, contando hoy: el máximo que puede pedir
/// la barrita elástica de «todos» (`Row::days`).
pub const MAX_DAYS: usize = 90;

/// Evalúa `$body` con `$tz` en la zona de la pantalla: la fija de `App::tz` si
/// la hay (tests) o `chrono::Local`.
macro_rules! in_zone {
    ($app:expr, $tz:ident => $body:expr) => {
        match $app.tz {
            Some(off) => {
                let $tz = &off;
                $body
            }
            None => {
                let $tz = &chrono::Local;
                $body
            }
        }
    };
}

/// Lo que el hilo de lectura manda a la pantalla.
pub enum Msg {
    Agents(Vec<Agent>),
    Activity {
        calls: Vec<Invocation>,
        /// Todas las ejecuciones con transcripción propia (`crate::runs`).
        runs: Vec<Run>,
        sessions: usize,
    },
    Error(String),
}

pub enum Action {
    Key(KeyEvent),
    Tick(DateTime<Utc>),
    Msg(Msg),
    /// Rueda del ratón: líneas que se mueve la vista (negativo, hacia arriba).
    /// El cursor se arrastra si se sale de la vista.
    Scroll(i32),
    /// Clic izquierdo en la celda (`column`, `row`) de la pantalla, contando
    /// desde 0 arriba a la izquierda, tal cual llega de crossterm. Se busca en
    /// `App::hits`.
    Click {
        column: u16,
        row: u16,
    },
    /// Hay una versión más nueva publicada (sin la `v`): la manda el hilo de
    /// comprobación de versiones, como `Action::UpdateAvailable` en moon.
    UpdateAvailable(String),
}

/// Zonas clicables del último fotograma. Las rellena la vista al pintar
/// (vaciándolas antes) con lo que de verdad se ve, así las posiciones no se
/// calculan dos veces; `update(Action::Click)` solo las consulta.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hits {
    /// Las pestañas de la fila superior.
    pub tabs: Vec<TabHit>,
    /// Una entrada por cada línea de pantalla de la lista que pertenece a una
    /// fila seleccionable; ocupa todo el ancho.
    pub rows: Vec<RowHit>,
}

/// Una pestaña: un clic en la fila `row` con la columna dentro de `cols`
/// (final excluido) abre `mode`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabHit {
    pub mode: Mode,
    pub row: u16,
    pub cols: std::ops::Range<u16>,
}

/// Una línea de la lista: la fila de pantalla `row` es de la línea
/// seleccionable `cursor` (el valor de `App::cursor` que la elige, es decir,
/// `Item::n() - 1`). Las líneas de un bloque de agente de activos (la del
/// agente y todas sus filas, con sus `skill` y `now`) llevan una entrada cada
/// una con el mismo `cursor`, el del bloque; las cabeceras de sección, las
/// líneas en blanco y las notas no llevan ninguna.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowHit {
    pub row: u16,
    pub cursor: usize,
}

/// La pestaña abierta: la lista de los activos, la de todos o el ranking por
/// tiempo trabajado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Active,
    All,
    Ranking,
}

impl Mode {
    /// Tab: activos → todos → ranking → activos.
    fn next(self) -> Self {
        match self {
            Mode::Active => Mode::All,
            Mode::All => Mode::Ranking,
            Mode::Ranking => Mode::Active,
        }
    }

    /// `a`: alterna activos y todos; desde el ranking, a activos.
    fn toggle(self) -> Self {
        match self {
            Mode::Active => Mode::All,
            Mode::All | Mode::Ranking => Mode::Active,
        }
    }
}

/// Orden de las filas dentro de cada grupo de la lista (`o`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Order {
    /// En curso, activos por su último uso, inactivos por su último uso y
    /// nunca usados por nombre.
    #[default]
    Recent,
    /// Por tiempo trabajado en total, de más a menos.
    Worked,
}

impl Order {
    fn toggle(self) -> Self {
        match self {
            Order::Recent => Order::Worked,
            Order::Worked => Order::Recent,
        }
    }
}

/// Una fila de la lista: un agente, su última llamada dentro de la ventana y
/// la última de todo el historial.
#[derive(Clone, Debug)]
pub struct Row {
    /// Número que se muestra y al que saltan los dígitos, de 1 en adelante.
    pub n: usize,
    /// Índice en la lista de agentes.
    pub agent: usize,
    /// Última llamada dentro de la ventana: si la hay, el agente está activo.
    pub last: Option<DateTime<Utc>>,
    /// Última llamada de todo el historial; `None` si nunca se usó.
    pub ever: Option<DateTime<Utc>>,
    /// Trabajo de todas sus ejecuciones y la que está en curso, si hay una.
    pub work: Work,
    /// Trabajo de cada uno de los últimos `MAX_DAYS` días locales, el más
    /// antiguo primero y hoy el último (`status::days_of`). Para un tramo más
    /// corto, `Row::days`.
    pub history: Vec<TimeDelta>,
    pub section: String,
}

/// Una llamada de la ficha: cuándo, desde dónde, si cae en la ventana y la
/// ejecución que lanzó, si se pudo enlazar.
#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))] // la ficha ya no lista llamadas (visual); lo usan los tests
pub struct Call<'a> {
    pub at: DateTime<Utc>,
    pub cwd: &'a Path,
    pub in_window: bool,
    pub run: Option<CallRun<'a>>,
}

/// La ejecución enlazada con una llamada (`status::run_of`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))] // la ficha ya no lista llamadas (visual); lo usan los tests
pub struct CallRun<'a> {
    /// Lo trabajado; si está en curso, lo que lleva hasta ahora.
    pub worked: TimeDelta,
    pub tools: usize,
    /// `description` del meta.
    pub description: Option<&'a str>,
    pub in_progress: bool,
}

/// Una ejecución de la ficha, de la que empezó más tarde a la más antigua.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunLine<'a> {
    /// Inicio en la hora local («4 nov 18:12»).
    pub start: Option<NaiveDateTime>,
    /// Lo trabajado; si está en curso, lo que lleva hasta ahora.
    pub worked: TimeDelta,
    pub tools: usize,
    /// `description` del meta.
    pub description: Option<&'a str>,
    /// Desde dónde se lanzó, solo si no es la raíz del proyecto del agente.
    pub cwd: Option<&'a Path>,
    pub in_progress: bool,
}

/// Una ejecución de activos, de cualquier tipo, en curso o terminada dentro de
/// la ventana (`state`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NowLine {
    /// Identificador estable de la ejecución (`status::run_id`): lo que
    /// identifica a la ficha de ejecución entre refrescos.
    pub id: String,
    /// `agentType`, con «workflow» para las de workflow (`status::display_name`).
    pub name: String,
    /// El encargo (`status::NowRun::task`): va en la propia fila.
    pub task: Option<String>,
    pub since: Option<DateTime<Utc>>,
    /// Lo que lleva trabajado hasta ahora.
    pub worked: TimeDelta,
    /// De dónde viene: agente definido, built-in o paso de workflow.
    pub kind: RunKind,
    /// La última skill invocada (`status::NowRun::skill`), tal cual
    /// (`superpowers:brainstorming`). Con ella el bloque lleva `Item::LiveSkill`.
    pub skill: Option<String>,
    /// Última herramienta usada (`Bash · git diff --stat`), para «ahora».
    pub last_tool: Option<String>,
    /// En curso (`RunState::Running`) o terminada y cuándo
    /// (`RunState::Done { ended }`: de ahí sale «hace N min»).
    pub state: RunState,
    /// Ejecuciones (o llamadas) fundidas en la fila, ≥ 1: en curso siempre 1;
    /// solo las terminadas funden las de su agente. `id` es la que la
    /// representa. En `Item::LiveAgent`, en cambio, solo las en curso del
    /// bloque del agente que deja el filtro, sin las que funde su terminada
    /// (0 si solo tiene terminada): de ahí sale `×N`, solo si es mayor que 1.
    pub runs: usize,
    /// Posición de la fila en el bloque (racimo) de su agente
    /// (`status::Chain`), rehecha sobre lo que deja el filtro: primero las en
    /// curso y la terminada, si la hay, al final. La cabeza lleva `Head(N)`
    /// con N filas visibles (en curso más la terminada), la última `Last` y
    /// una que queda sola, `Single`, esté en curso o terminada.
    pub chain: Chain,
    /// Tokens de todas las ejecuciones de la fila (columna «tokens»).
    pub tokens: u64,
    /// Contexto de la representante (columna «ctx»): solo en curso; `None`
    /// si terminó o aún no hay ningún mensaje con uso.
    pub context: Option<Context>,
}

/// El último uso de cualquier agente definido, para el monitor vacío
/// («último uso: <agente> de <proyecto> hace X»).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastUse {
    pub agent: usize,
    /// Grupo del agente: «global» o el proyecto acortado con `~`.
    pub section: String,
    pub at: DateTime<Utc>,
}

impl Row {
    /// En curso ahora o con una llamada dentro de la ventana.
    pub fn active(&self) -> bool {
        self.running() || self.last.is_some()
    }

    /// Tiene una ejecución en curso.
    pub fn running(&self) -> bool {
        self.work.running.is_some()
    }

    /// Trabajo de los últimos `n` días locales (como mucho `MAX_DAYS`), el más
    /// antiguo primero y hoy el último. No recalcula: es un tramo de `history`.
    pub fn days(&self, n: usize) -> &[TimeDelta] {
        &self.history[self.history.len().saturating_sub(n)..]
    }

    /// Trabajo medio por ejecución; `None` sin ejecuciones.
    pub fn mean(&self) -> Option<TimeDelta> {
        let runs = i32::try_from(self.work.runs).ok().filter(|&n| n > 0)?;
        Some(self.work.total / runs)
    }
}

/// Una línea de la lista tal como se pinta. Las seleccionables (`Row`,
/// `Folded`, `Rank`, `LiveAgent`) llevan número (`Item::n`). En activos se
/// numera y se selecciona por bloque de agente, no por ejecución.
pub enum Item {
    /// Activos: regla de un grupo de proyecto. `section` es la clave del grupo
    /// (como `Row::section`: «global» o la carpeta acortada con `~`); el
    /// nombre corto es cosa de la vista. `running` son las filas en curso del
    /// grupo (una por ejecución) y `recent` las terminadas (una por agente),
    /// contadas sobre lo que deja el filtro (`running + recent` son las filas
    /// que le siguen, repartidas en bloques por agente). No se selecciona.
    LiveGroup {
        section: String,
        running: usize,
        recent: usize,
    },
    /// Activos: una fila de ejecución (encargo, contexto, tiempo y tokens),
    /// en curso o terminada (`line.state`). Sin número: no se selecciona por
    /// sí misma, es parte del bloque de su `LiveAgent` (cursor, scroll y
    /// clic). En curso le siguen siempre `LiveSkill` (solo si tiene skill) y
    /// `LiveNow`; terminada, nada (va la última de su bloque).
    Live {
        line: NowLine,
    },
    /// Activos: la línea del agente de un bloque, justo encima de su primera
    /// fila (`Chain::Head`, o `Chain::Single` si va sola: un bloque de uno),
    /// esté en curso o terminada: nombre, etiqueta y `×N` (`line.runs`, las
    /// ejecuciones en curso del bloque, sin las que funde la terminada; sin él
    /// si es 1 o 0), sin cifras. Es lo
    /// seleccionable: lleva el número `n` del bloque, y el bloque entero
    /// (esta línea y todas sus filas, con sus `skill` y `now`) es una unidad
    /// para el cursor, el scroll y el clic. `line` es la de la primera fila
    /// del bloque salvo `runs`: Enter abre la pantalla del agente del bloque
    /// (`line.id` lo identifica). `row` es la
    /// del agente definido si lo hay (un agente definido, o un workflow cuyo
    /// agente real lo es) y `None` si es built-in o un workflow sin agente
    /// definido; lleva el mismo número que el bloque.
    LiveAgent {
        n: usize,
        line: NowLine,
        row: Option<Row>,
    },
    /// Activos: «skill …» (`line.skill`), justo bajo una fila en curso que
    /// tiene skill. No se selecciona (es parte del bloque); va siempre detrás
    /// de su `Live`.
    LiveSkill(NowLine),
    /// Activos: «now …» (`line.last_tool`; la vista pinta `—` si falta).
    /// Solo en las filas en curso, siempre. No se selecciona (es parte del
    /// bloque); va siempre detrás de su `Live` o, si lo hay, de su `LiveSkill`.
    LiveNow(NowLine),
    /// Activos sin nada en curso ni en la ventana (y sin filtro): el último
    /// uso de cualquier agente definido, o `None` si no hay ninguno.
    Quiet {
        last: Option<LastUse>,
    },
    /// Cabecera de las columnas, una vez encima del primer grupo (en activos,
    /// encima del primer grupo de proyecto).
    Columns,
    Section {
        title: String,
        /// Activos del grupo: «todos» ya no los pinta (visual); lo usan los tests.
        #[cfg_attr(not(test), allow(dead_code))]
        active: usize,
        total: usize,
        /// Lo trabajado por las filas del grupo («sin uso» si es cero).
        worked: TimeDelta,
    },
    Row(Row),
    /// Grupo en el que ningún agente se ha usado nunca: una línea que Enter
    /// abre y cierra; abierto, le siguen sus filas.
    Folded {
        title: String,
        total: usize,
        n: usize,
        open: bool,
    },
    /// Regla del ranking: cuántos agentes tienen trabajo en el periodo.
    RankRule {
        agents: usize,
    },
    /// Fila del ranking: el agente y su trabajo, ejecuciones y tokens dentro
    /// del periodo. La barra y el orden siguen a `App::rank_key`.
    Rank {
        row: Row,
        work: PeriodStats,
    },
    /// Línea en blanco: antes de la regla de cada grupo menos el primero y,
    /// en activos, entre bloques de agente del mismo grupo. No se selecciona
    /// ni es parte de ninguna fila.
    Blank,
}

impl Item {
    /// Número de la línea si se puede seleccionar.
    pub fn n(&self) -> Option<usize> {
        match self {
            Item::Row(r) | Item::Rank { row: r, .. } => Some(r.n),
            Item::Folded { n, .. } | Item::LiveAgent { n, .. } => Some(*n),
            _ => None,
        }
    }

    /// El agente definido de la línea, si lo tiene: filas, filas del
    /// ranking y bloques de activos de un agente definido.
    fn row(&self) -> Option<&Row> {
        match self {
            Item::Row(r) | Item::Rank { row: r, .. } => Some(r),
            Item::LiveAgent { row, .. } => row.as_ref(),
            _ => None,
        }
    }
}

/// Una ficha de ejecución de la pila de vuelta, tal como se dejó: su última
/// foto (se refresca al volver), el desplazamiento de los pasos y la hija
/// elegida.
struct RunBack {
    detail: RunDetail,
    steps_scroll: usize,
    launched_sel: Option<usize>,
}

pub struct App {
    pub minutes: i64,
    pub home: PathBuf,
    pub version: &'static str,
    agents: Vec<Agent>,
    calls: Vec<Invocation>,
    /// Ejecuciones del último `Msg::Activity`, sin orden.
    runs: Vec<Run>,
    pub sessions: usize,
    /// Ya llegó la primera lista de agentes.
    pub loaded: bool,
    /// Ya llegaron los agentes; `loaded` espera además la primera pasada del
    /// historial, para no pintar a todos como «nunca» durante ese segundo.
    agents_in: bool,
    pub now: DateTime<Utc>,
    /// Cuándo se abrió la pantalla: la espera de la carga cuenta desde aquí.
    pub started: DateTime<Utc>,
    /// Carpeta de las transcripciones (`~/.claude/projects`), la lectura que
    /// la pantalla de carga nombra. Vacía si nadie la ha dado.
    pub projects: PathBuf,
    /// Zona de los días y las horas: `None` es la local; los tests fijan una.
    pub tz: Option<FixedOffset>,
    pub error: Option<String>,
    /// Posición en las líneas seleccionables.
    pub cursor: usize,
    /// Primera línea de la lista que se pinta.
    pub scroll: usize,
    pub filter: String,
    pub filtering: bool,
    pub mode: Mode,
    /// Orden dentro de los grupos de la lista.
    pub order: Order,
    /// Periodo del ranking.
    pub period: Period,
    /// Por qué se ordena el ranking y qué mide su barra (`s`).
    pub rank_key: RankKey,
    /// Grupos plegados que se han abierto con Enter.
    open: HashSet<String>,
    /// Ficha de agente abierta (la del agente bajo el cursor).
    pub detail: bool,
    /// Pantalla del agente abierta (Enter sobre un bloque de activos): qué
    /// ejecución tiene el cursor. Queda debajo de la pantalla de la
    /// ejecución (`run_detail`) y de la ficha del agente (`detail`) cuando se
    /// abren desde ella: Esc vuelve aquí. Lo que se pinta sale de
    /// `agent_view`.
    agent_screen: Option<agent::AgentScreen>,
    /// Pantalla de la ejecución abierta: la última foto de la ejecución (`id` dentro).
    /// Tiene prioridad sobre `detail`; se refresca con cada `Tick` y
    /// `Msg::Activity` y, si la ejecución acaba o desaparece, se queda
    /// abierta con `ended` (`RunDetail::refresh`).
    pub run_detail: Option<RunDetail>,
    /// Hija elegida en «launched» de la ficha de ejecución (índice en
    /// `RunDetail::launched`): Tab/Shift-Tab o un dígito la eligen y Enter la
    /// abre. `None` sin elegir; al abrir otra ficha vuelve a `None`.
    pub launched_sel: Option<usize>,
    /// Fichas de ejecución de las que se vino abriendo una hija o la madre
    /// (`p`), la más reciente al final: Esc vuelve a la última.
    run_back: Vec<RunBack>,
    /// Pasos de la ficha de ejecución que se saltan por arriba, contando como
    /// se pintan (`steps.len()` líneas). Se acota a `0..=len-1` y, al pintar,
    /// a lo que cabe (`steps_window`).
    pub steps_scroll: usize,
    /// Alto de la lista de pasos en la última vuelta: lo que avanza una página.
    steps_page: usize,
    /// Alto del cuerpo en la última vuelta: lo que avanza una página.
    page: usize,
    /// Dígitos tecleados que aún pueden crecer con otro.
    pending: Option<usize>,
    pub quit: bool,
    rescan: bool,
    /// Zonas clicables del último fotograma (las rellena la vista).
    pub hits: Hits,
    /// Versión más nueva publicada (sin la `v`), si la comprobación encontró
    /// una; `None` mientras no llegue `Action::UpdateAvailable`.
    pub update_available: Option<String>,
}

impl App {
    pub fn new(minutes: i64, home: PathBuf, version: &'static str, now: DateTime<Utc>) -> Self {
        Self {
            minutes,
            home,
            version,
            agents: Vec::new(),
            calls: Vec::new(),
            runs: Vec::new(),
            sessions: 0,
            loaded: false,
            agents_in: false,
            now,
            started: now,
            projects: PathBuf::new(),
            tz: None,
            error: None,
            cursor: 0,
            scroll: 0,
            filter: String::new(),
            filtering: false,
            mode: Mode::Active,
            order: Order::default(),
            period: Period::default(),
            rank_key: RankKey::default(),
            open: HashSet::new(),
            detail: false,
            agent_screen: None,
            run_detail: None,
            launched_sel: None,
            run_back: Vec::new(),
            steps_scroll: 0,
            steps_page: 0,
            page: 0,
            pending: None,
            quit: false,
            rescan: false,
            hits: Hits::default(),
            update_available: None,
        }
    }

    pub fn agent(&self, i: usize) -> &Agent {
        &self.agents[i]
    }

    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    fn since(&self) -> DateTime<Utc> {
        self.now - Duration::minutes(self.minutes)
    }

    fn section_of(&self, scope: &Scope) -> String {
        match scope {
            Scope::Global => "global".to_string(),
            Scope::Project(dir) => report::shorten(dir, &self.home),
        }
    }

    fn in_window(&self, at: DateTime<Utc>) -> bool {
        at >= self.since()
    }

    /// Última vez de uso en todo el historial: llamada o ejecución.
    fn ever_of(&self, agent: &Agent) -> Option<DateTime<Utc>> {
        status::last_use(agent, &self.calls, &self.runs)
    }

    /// Última llamada si cae dentro de la ventana.
    fn last_of(&self, agent: &Agent) -> Option<DateTime<Utc>> {
        self.ever_of(agent).filter(|t| self.in_window(*t))
    }

    /// Trabajo del agente a fecha de ahora.
    fn work_of(&self, agent: &Agent) -> Work {
        status::work_of(agent, &self.runs, self.now)
    }

    /// Agentes activos ahora mismo (en curso o con una llamada en la ventana),
    /// con o sin filtro.
    #[cfg_attr(not(test), allow(dead_code))] // «todos» ya no cuenta activos (visual); lo usan los tests
    pub fn active_count(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| self.last_of(a).is_some() || self.work_of(a).running.is_some())
            .count()
    }

    /// La fila del agente `i`, sin numerar.
    fn row_of(&self, i: usize) -> Row {
        let a = &self.agents[i];
        Row {
            n: 0,
            agent: i,
            last: self.last_of(a),
            ever: self.ever_of(a),
            work: self.work_of(a),
            history: in_zone!(self, tz => status::days_of(a, &self.runs, self.now, tz, MAX_DAYS)),
            section: self.section_of(&a.scope),
        }
    }

    /// El filtro encaja con el nombre o con el grupo.
    fn matches(&self, row: &Row, needle: &str) -> bool {
        needle.is_empty()
            || self.agents[row.agent].name.to_lowercase().contains(needle)
            || row.section.to_lowercase().contains(needle)
    }

    /// Las filas con agente de la vista abierta, en orden y con su número.
    #[cfg_attr(not(test), allow(dead_code))] // las columnas salen de items() (visual); lo usan los tests
    pub fn rows(&self) -> Vec<Row> {
        self.items().iter().filter_map(Item::row).cloned().collect()
    }

    /// Cuántas líneas seleccionables hay: filas, grupos plegados y bloques de
    /// activos.
    pub fn count(&self) -> usize {
        self.items().iter().filter_map(Item::n).max().unwrap_or(0)
    }

    /// Las líneas de la vista abierta: el monitor, la lista agrupada o el
    /// ranking.
    pub fn items(&self) -> Vec<Item> {
        match self.mode {
            Mode::Ranking => self.ranking_items(),
            Mode::Active => self.monitor_items(),
            Mode::All => self.list_items(),
        }
    }

    /// Ejecuciones en curso ahora, de cualquier tipo y sin filtro: la
    /// cabecera de activos (Σ `running` de los grupos).
    pub fn live_count(&self) -> usize {
        self.groups().iter().map(|g| g.running).sum()
    }

    /// Ejecuciones terminadas dentro de la ventana, sin filtro: la cabecera de
    /// activos (Σ `recent` de los grupos).
    pub fn recent_count(&self) -> usize {
        self.groups().iter().map(|g| g.recent).sum()
    }

    /// Los grupos de activos tal como salen de `status::active_groups`, sin
    /// filtro y ya en su orden final.
    fn groups(&self) -> Vec<status::LiveGroup<'_>> {
        status::active_groups(
            &self.agents,
            &self.calls,
            &self.runs,
            self.since(),
            self.now,
            &self.home,
        )
    }

    /// Enter abre algo en la fila del cursor: la pantalla del agente si es
    /// un bloque de activos (en curso o terminado, de cualquier tipo, también
    /// una llamada sin ejecución), o la ficha del agente si la fila tiene uno
    /// («todos», ranking). Desde activos la ficha del agente solo se abre con
    /// `a` dentro de la pantalla del agente.
    pub fn can_open(&self) -> bool {
        match self.mode {
            Mode::Active => self.live_at_cursor().is_some(),
            _ => self.current().is_some(),
        }
    }

    /// El bloque de agente de activos bajo el cursor, si el cursor está en
    /// uno: la línea de su primera ejecución y la fila de su agente.
    fn live_at_cursor(&self) -> Option<(NowLine, Option<Row>)> {
        self.items().into_iter().find_map(|i| match i {
            Item::LiveAgent { n, line, row } if n == self.cursor + 1 => Some((line, row)),
            _ => None,
        })
    }

    /// Los bloques de agente de activos de la lista abierta, en el orden
    /// agrupado: (número, línea de su primera ejecución, fila del agente).
    fn live_lines(&self) -> Vec<(usize, NowLine, Option<Row>)> {
        self.items()
            .into_iter()
            .filter_map(|i| match i {
                Item::LiveAgent { n, line, row } => Some((n, line, row)),
                _ => None,
            })
            .collect()
    }

    /// Los ids de todas las ejecuciones del bloque de agente que contiene
    /// `id` (`status::sibling_ids`): primero las en curso, en el orden del
    /// bloque, y luego las fundidas en su terminada, de la más reciente a la
    /// más antigua; el primero es la cabeza del bloque. Se calcula sobre los
    /// grupos sin filtrar: la ficha no depende de lo que se teclease en `/`.
    fn siblings(&self, id: &str) -> Vec<String> {
        status::sibling_ids(&self.groups(), id)
    }

    /// El bloque de agente de activos que contiene la ejecución `id`: el de
    /// su fila si la tiene (toda en curso, y la que representa a una
    /// terminada); si no, el de una hermana suya (la terminada que la funde
    /// o, si el filtro quitó su fila, otra del bloque). (número, línea de la
    /// primera ejecución del bloque, fila del agente).
    fn live_line_of(&self, id: &str) -> Option<(usize, NowLine, Option<Row>)> {
        // el número de bloque de cada fila de ejecución visible
        let mut block = 0;
        let rows: Vec<(usize, String)> = self
            .items()
            .into_iter()
            .filter_map(|i| match i {
                Item::LiveAgent { n, .. } => {
                    block = n;
                    None
                }
                Item::Live { line } => Some((block, line.id)),
                _ => None,
            })
            .collect();
        let n = rows
            .iter()
            .find(|(_, i)| i == id)
            .or_else(|| {
                let ids = self.siblings(id);
                rows.iter().find(|(_, i)| ids.contains(i))
            })?
            .0;
        self.live_lines().into_iter().find(|(m, _, _)| *m == n)
    }

    /// Posición de la ejecución abierta entre las del bloque de su agente,
    /// en el orden de `siblings`, que es el de la pantalla del agente y su
    /// numeración: (número empezando en 1, total), también si solo hay una
    /// (`(1, 1)`). `None` si no hay pantalla de ejecución o la abierta ya no
    /// está en la ventana de activos. La vista lo usa para `run i of n` y el
    /// pie anuncia `←→ other run` solo si el total pasa de 1.
    pub fn run_nav(&self) -> Option<(usize, usize)> {
        let d = self.run_detail.as_ref()?;
        let ids = self.siblings(&d.id);
        let pos = ids.iter().position(|i| *i == d.id)?;
        Some((pos + 1, ids.len()))
    }

    /// Lo de activos: por proyecto (`status::active_groups`, ya en su orden
    /// final: los grupos por su actividad más reciente; dentro, un bloque por
    /// agente con sus en curso, una por ejecución, y detrás su terminada, una
    /// por agente; primero los agentes con algo en curso y luego los que solo
    /// tienen terminada), con los bloques numerados de corrido por todo (un
    /// número por bloque de agente, no por ejecución). El filtro se aplica
    /// sobre las filas: nombre, carpeta del grupo (`group_dir`, lo que se ve
    /// en el título), `cwd` real o encargo; un grupo sin filas no sale, sus
    /// contadores `running`/`recent` se rehacen con lo que queda y los
    /// bloques también (`rechain`). Arriba del todo, la cabecera de columnas.
    /// Una fila en curso mide dos líneas (fila y `now`), tres si tiene skill
    /// (justo bajo la fila); una terminada, una. Cada bloque empieza por la
    /// línea de su agente (`Item::LiveAgent`), que es la que lleva el número.
    /// Un `Item::Blank` antes de la regla de cada grupo menos el primero y
    /// otro entre bloques de agente del mismo grupo; ninguno es parte de un
    /// bloque para el scroll. Si su agente es uno definido (`NowRun::owner`),
    /// el bloque lleva su fila para la ficha.
    fn live_items(&self, needle: &str, n: &mut usize) -> Vec<Item> {
        let keep = |r: &status::NowRun| {
            let dir = |d: Option<&Path>| {
                d.map(|d| report::shorten(d, &self.home))
                    .is_some_and(|d| contains(&d, needle))
            };
            contains(r.name, needle)
                || dir(r.group_dir)
                || dir(r.cwd)
                || r.task.is_some_and(|t| contains(t, needle))
        };
        let mut items = Vec::new();
        for group in self.groups() {
            let blocks = rechain(group.lines, keep);
            if blocks.is_empty() {
                continue;
            }
            let rows = blocks.iter().map(Vec::len).sum::<usize>();
            let running = blocks
                .iter()
                .flatten()
                .filter(|r| r.state == RunState::Running)
                .count();
            if items.is_empty() {
                items.push(Item::Columns);
            } else {
                // aire antes de la regla de cada grupo menos el primero
                items.push(Item::Blank);
            }
            items.push(Item::LiveGroup {
                section: group.section,
                running,
                recent: rows - running,
            });
            for (b, block) in blocks.into_iter().enumerate() {
                if b > 0 {
                    // aire entre bloques de agente del mismo grupo
                    items.push(Item::Blank);
                }
                // ×N: solo las en curso que deja el filtro; las que funde la
                // terminada no cuentan
                let runs = block
                    .iter()
                    .filter(|r| r.state == RunState::Running)
                    .count();
                // un número por bloque: va en la línea del agente
                *n += 1;
                for r in block {
                    let line = now_line(&r);
                    let running = line.state == RunState::Running;
                    // el nombre del agente va en su propia línea encima de la
                    // primera fila de su bloque, aunque sea de una o solo
                    // terminada; con las en curso del bloque (×N)
                    if matches!(line.chain, Chain::Head(_) | Chain::Single) {
                        items.push(Item::LiveAgent {
                            n: *n,
                            line: NowLine {
                                runs,
                                ..line.clone()
                            },
                            row: r.owner.map(|i| Row {
                                n: *n,
                                ..self.row_of(i)
                            }),
                        });
                    }
                    items.push(Item::Live { line: line.clone() });
                    // las terminadas son una sola línea; las en curso, siempre
                    // desplegadas: skill (si la hay) y now
                    if running {
                        if line.skill.is_some() {
                            items.push(Item::LiveSkill(line.clone()));
                        }
                        items.push(Item::LiveNow(line));
                    }
                }
            }
        }
        items
    }

    /// Activos, el monitor en directo: un grupo por proyecto con un bloque por
    /// agente (lo en curso y, debajo, lo terminado en la ventana), con los
    /// bloques numerados de corrido. Sin secciones
    /// ni plegados. Si no hay nada y no se filtra, el último uso de cualquier
    /// agente; si el filtro lo deja vacío, nada.
    fn monitor_items(&self) -> Vec<Item> {
        let needle = self.filter.to_lowercase();
        let mut n = 0;
        let mut items = self.live_items(&needle, &mut n);
        if items.is_empty() && needle.is_empty() {
            let last = status::last_use_overall(&self.agents, &self.calls, &self.runs).map(
                |(agent, at)| LastUse {
                    agent,
                    section: self.section_of(&self.agents[agent].scope),
                    at,
                },
            );
            items.push(Item::Quiet { last });
        }
        items
    }

    /// La lista agrupada por proyecto y numerada de corrido.
    ///
    /// Grupos: primero los que tienen algo en curso, luego los que tienen algún
    /// activo (el más reciente arriba), luego el resto por su última llamada de
    /// siempre; a igualdad, global va detrás. Dentro del grupo, según `order`:
    /// en curso, activos por su última llamada, inactivos por su última vez y
    /// nunca usados por nombre; o por trabajado. Los grupos en los que ningún
    /// agente se ha usado nunca van al final, plegados en una línea. Lo que
    /// está en curso se ve solo con su ✦ en la fila.
    fn list_items(&self) -> Vec<Item> {
        let needle = self.filter.to_lowercase();
        // Grupos con algún agente usado alguna vez, mirando todos sus agentes
        // y no solo los que deja el filtro.
        let used: HashSet<String> = self
            .agents
            .iter()
            .filter(|a| status::ever_used(a, &self.calls, &self.runs))
            .map(|a| self.section_of(&a.scope))
            .collect();
        let (mut shown, mut unused): (Vec<Row>, Vec<Row>) = (0..self.agents.len())
            .map(|i| self.row_of(i))
            .filter(|r| self.matches(r, &needle))
            .partition(|r| used.contains(&r.section));
        // De cada grupo: si tiene algo en curso, si tiene algún activo y su
        // última llamada de siempre. Las tuplas ordenan campo a campo y `None`
        // (nunca usado) queda por debajo de cualquier `Some`.
        let mut groups: HashMap<String, (bool, bool, Option<DateTime<Utc>>)> = HashMap::new();
        for r in &shown {
            let g = groups.entry(r.section.clone()).or_default();
            g.0 |= r.running();
            g.1 |= r.active();
            g.2 = g.2.max(r.ever);
        }
        let key = |r: &Row| (r.running(), r.active(), r.ever);
        let name = |r: &Row| &self.agents[r.agent].name;
        shown.sort_by(|a, b| {
            groups
                .get(&b.section)
                .cmp(&groups.get(&a.section))
                .then_with(|| (a.section == "global").cmp(&(b.section == "global")))
                .then_with(|| a.section.cmp(&b.section))
                .then_with(|| match self.order {
                    Order::Recent => key(b).cmp(&key(a)),
                    Order::Worked => b.work.total.cmp(&a.work.total),
                })
                .then_with(|| name(a).cmp(name(b)))
        });
        unused.sort_by(|a, b| {
            (a.section == "global")
                .cmp(&(b.section == "global"))
                .then_with(|| a.section.cmp(&b.section))
                .then_with(|| name(a).cmp(name(b)))
        });

        if shown.is_empty() && unused.is_empty() {
            return Vec::new();
        }
        let mut items = vec![Item::Columns];
        let mut n = 0;
        for (k, group) in chunk_by_section(&mut shown).into_iter().enumerate() {
            if k > 0 {
                items.push(Item::Blank);
            }
            items.push(Item::Section {
                title: group[0].section.clone(),
                active: group.iter().filter(|r| r.active()).count(),
                total: group.len(),
                worked: status::total_worked(group.iter().map(|r| &r.work)),
            });
            for r in group {
                n += 1;
                r.n = n;
                items.push(Item::Row(r.clone()));
            }
        }
        if !shown.is_empty() && !unused.is_empty() {
            items.push(Item::Blank);
        }
        let groups = chunk_by_section(&mut unused);
        let last = groups.len().saturating_sub(1);
        for (k, group) in groups.into_iter().enumerate() {
            let title = group[0].section.clone();
            let open = self.open.contains(&title);
            n += 1;
            items.push(Item::Folded {
                title,
                total: group.len(),
                n,
                open,
            });
            if open {
                for r in group {
                    n += 1;
                    r.n = n;
                    items.push(Item::Row(r.clone()));
                }
                if k < last {
                    items.push(Item::Blank);
                }
            }
        }
        items
    }

    /// Trabajo, ejecuciones y tokens del agente `i` en el periodo del ranking.
    fn period_stats(&self, i: usize) -> PeriodStats {
        in_zone!(self, tz => status::stats_in(&self.agents[i], &self.runs, self.now, tz, self.period))
    }

    /// El ranking: los agentes con trabajo o alguna ejecución en el periodo
    /// (una de 0 min también cuenta), de más a menos según `rank_key`, sin
    /// grupos, con la regla delante. Vacío si no hay ninguno.
    fn ranking_items(&self) -> Vec<Item> {
        let needle = self.filter.to_lowercase();
        let mut ranked: Vec<(Row, PeriodStats)> = (0..self.agents.len())
            .map(|i| (i, self.period_stats(i)))
            .filter(|(_, w)| w.worked > TimeDelta::zero() || w.runs > 0)
            .map(|(i, w)| (self.row_of(i), w))
            .filter(|(r, _)| self.matches(r, &needle))
            .collect();
        ranked.sort_by(|(ra, a), (rb, b)| {
            self.rank_key
                .cmp(a, b)
                .then_with(|| self.agents[ra.agent].name.cmp(&self.agents[rb.agent].name))
                .then_with(|| ra.section.cmp(&rb.section))
        });
        if ranked.is_empty() {
            return Vec::new();
        }
        let mut items = vec![Item::RankRule {
            agents: ranked.len(),
        }];
        for (k, (mut row, work)) in ranked.into_iter().enumerate() {
            row.n = k + 1;
            items.push(Item::Rank { row, work });
        }
        items
    }

    /// Suma del ranking que se ve: trabajado, ejecuciones y tokens en el
    /// periodo.
    pub fn ranking_total(&self) -> PeriodStats {
        let mut total = PeriodStats::default();
        if self.mode != Mode::Ranking {
            return total;
        }
        for i in self.ranking_items() {
            if let Item::Rank { work, .. } = i {
                total += work;
            }
        }
        total
    }

    /// Ejecuciones que solo siguen en el historial guardado porque Claude Code
    /// ya borró su transcripción: «history kept: N runs…» del ranking.
    pub fn cached_only(&self) -> usize {
        status::cached_only(&self.runs)
    }

    /// Tokens de las filas de activos que se ven (con el filtro aplicado): la
    /// cabecera de activos.
    pub fn live_tokens(&self) -> u64 {
        if self.filter.is_empty() {
            return status::window_tokens(&self.groups());
        }
        self.items()
            .iter()
            .map(|i| match i {
                Item::Live { line, .. } => line.tokens,
                _ => 0,
            })
            .sum()
    }

    /// La fila bajo el cursor, si es un agente (no un grupo plegado).
    pub fn current(&self) -> Option<Row> {
        self.items()
            .into_iter()
            .find(|i| i.n() == Some(self.cursor + 1))
            .and_then(|i| i.row().cloned())
    }

    /// El grupo plegado bajo el cursor, si lo es.
    fn folded_at_cursor(&self) -> Option<String> {
        self.items().into_iter().find_map(|i| match i {
            Item::Folded { title, n, .. } if n == self.cursor + 1 => Some(title),
            _ => None,
        })
    }

    /// Llamadas del agente en todo el historial, la más reciente primero,
    /// marcando las que caen dentro de la ventana y con la ejecución que
    /// lanzaron si se enlazó (las antiguas, sin `id`, no enlazan).
    #[cfg_attr(not(test), allow(dead_code))] // la ficha ya no lista llamadas (visual); lo usan los tests
    pub fn calls_of(&self, agent: usize) -> Vec<Call<'_>> {
        status::invocations_of(&self.agents[agent], &self.calls)
            .into_iter()
            .map(|c| Call {
                at: c.at,
                cwd: &c.cwd,
                in_window: self.in_window(c.at),
                run: status::run_of(c, &self.runs).map(|r| CallRun {
                    worked: r.worked_until(self.now),
                    tools: r.tools,
                    description: r.description.as_deref(),
                    in_progress: r.in_progress(self.now),
                }),
            })
            .collect()
    }

    /// Todas las ejecuciones del agente para la ficha, también las de workflow
    /// con su `agentType`, de la que empezó más tarde a la más antigua.
    pub fn runs_of(&self, agent: usize) -> Vec<RunLine<'_>> {
        let agent = &self.agents[agent];
        let root = match &agent.scope {
            Scope::Project(dir) => Some(dir.as_path()),
            Scope::Global => None,
        };
        status::runs_of(agent, &self.runs)
            .into_iter()
            .map(|r| RunLine {
                start: in_zone!(self, tz => r.start_local(tz).map(|s| s.naive_local())),
                worked: r.worked_until(self.now),
                tools: r.tools,
                description: r.description.as_deref(),
                cwd: r.cwd.as_deref().filter(|c| Some(*c) != root),
                in_progress: r.in_progress(self.now),
            })
            .collect()
    }

    /// Llamadas antiguas del agente sin ejecución enlazada: «+ N llamadas
    /// antiguas sin registro de duración».
    pub fn old_calls(&self, agent: usize) -> usize {
        status::calls_without_run(&self.agents[agent], &self.calls, &self.runs)
    }

    /// Líneas seleccionables de `items`, en el orden de su número.
    fn row_lines(items: &[Item]) -> Vec<usize> {
        items
            .iter()
            .enumerate()
            .filter_map(|(l, i)| i.n().is_some().then_some(l))
            .collect()
    }

    /// Primera línea que tiene que verse para que la fila de la línea `line`
    /// se vea (en activos, `line` es la del agente de un bloque): todo lo de
    /// encima si es la primera (columnas, regla del ranking, o la regla de
    /// grupo en activos) o su cabecera si es la primera de su sección.
    fn row_top(items: &[Item], line: usize) -> usize {
        if items[..line].iter().all(|i| i.n().is_none()) {
            0
        } else if line >= 1
            && matches!(
                items[line - 1],
                Item::Section { .. } | Item::LiveGroup { .. }
            )
        {
            line - 1
        } else {
            line
        }
    }

    /// Última línea de la fila de la línea `line`: en activos, la última
    /// del bloque de agente que empieza en `line` (todas sus filas, con sus
    /// `skill` y `now`); fuera de activos, ella misma.
    fn row_end(items: &[Item], line: usize) -> usize {
        let mut end = line;
        while matches!(
            items.get(end + 1),
            Some(Item::Live { .. } | Item::LiveSkill(_) | Item::LiveNow(_))
        ) {
            end += 1;
        }
        end
    }

    /// Mueve la vista `delta` líneas y arrastra el cursor si se queda fuera.
    fn scroll_by(&mut self, delta: i64) {
        let items = self.items();
        let page = self.page.max(1);
        let max = items.len().saturating_sub(page) as i64;
        self.scroll = (self.scroll as i64 + delta).clamp(0, max.max(0)) as usize;
        let lines = Self::row_lines(&items);
        let Some(&line) = lines.get(self.cursor) else {
            return;
        };
        let (top, bottom) = (self.scroll, self.scroll + page);
        // Fila que se ve entera, cabecera incluida: así repintar no mueve la vista.
        let fits = |l: usize| Self::row_top(&items, l) >= top && Self::row_end(&items, l) < bottom;
        if Self::row_top(&items, line) < top {
            if let Some(i) = lines.iter().position(|&l| fits(l)) {
                self.cursor = i;
            }
        } else if Self::row_end(&items, line) >= bottom {
            if let Some(i) = lines.iter().rposition(|&l| fits(l)) {
                self.cursor = i;
            }
        }
    }

    /// Una página arriba (`dir < 0`) o abajo: la vista y el cursor avanzan el
    /// alto del cuerpo; en los extremos, el cursor va a la primera o la última.
    fn page_by(&mut self, dir: i64) {
        let items = self.items();
        let lines = Self::row_lines(&items);
        let Some(&line) = lines.get(self.cursor) else {
            return;
        };
        let page = self.page.max(1);
        self.cursor = if dir > 0 {
            let target = line + page;
            lines
                .iter()
                .position(|&l| l >= target)
                .unwrap_or(lines.len() - 1)
        } else {
            let target = line.saturating_sub(page);
            lines.iter().rposition(|&l| l <= target).unwrap_or(0)
        };
        self.scroll_by(dir * page as i64);
    }

    /// Mueve `scroll` lo justo para que la línea del cursor quede a la vista.
    /// Guarda además `height` como alto de página para la rueda y las páginas.
    pub fn scroll_to_cursor(&mut self, items: &[Item], height: usize) {
        self.page = height;
        let Some(line) = items.iter().position(|i| i.n() == Some(self.cursor + 1)) else {
            self.scroll = 0;
            return;
        };
        // La cabecera de la sección acompaña a su primera fila, y las demás
        // líneas de un bloque de activos, a la de su agente.
        let top = Self::row_top(items, line);
        let end = Self::row_end(items, line);
        if top < self.scroll {
            self.scroll = top;
        } else if height > 0 && end >= self.scroll + height {
            // un bloque más alto que la vista enseña su principio: lo que quepa
            self.scroll = (end + 1 - height).min(top);
        }
        self.scroll = self.scroll.min(items.len().saturating_sub(height));
    }

    /// En la ficha, ↑↓ pasan al agente anterior o siguiente de la vista
    /// abierta, saltando los grupos plegados y, en activos, las filas en
    /// curso sin agente definido; en los extremos no se mueve.
    fn step_agent(&mut self, dir: i64) {
        let agents: Vec<usize> = self
            .items()
            .iter()
            .filter_map(|i| i.row().map(|r| r.n - 1))
            .collect();
        let next = if dir > 0 {
            agents.iter().find(|&&c| c > self.cursor)
        } else {
            agents.iter().rev().find(|&&c| c < self.cursor)
        };
        if let Some(&c) = next {
            self.cursor = c;
        }
    }

    /// Abre la ficha de la ejecución `id` con su última lectura, en lugar de
    /// la abierta y sin tocar la pila de vuelta; si no está en disco no hace
    /// nada. Devuelve si la abrió.
    fn open_run(&mut self, id: &str) -> bool {
        let Some(d) = status::activity_detail(&self.agents, &self.calls, &self.runs, id, self.now)
        else {
            return false;
        };
        self.run_detail = Some(d);
        self.steps_scroll = 0;
        self.launched_sel = None;
        true
    }

    /// Abre la ficha de `id` (una hija o la madre) guardando la abierta en la
    /// pila de vuelta, para que Esc regrese a ella tal como estaba.
    fn push_run(&mut self, id: &str) {
        let Some(here) = self.run_detail.clone() else {
            return;
        };
        let back = RunBack {
            detail: here,
            steps_scroll: self.steps_scroll,
            launched_sel: self.launched_sel,
        };
        if self.open_run(id) {
            self.run_back.push(back);
        }
    }

    /// Esc (o Enter sin hija elegida): vuelve a la ficha de la que se vino,
    /// puesta al día, o, si no hay, cierra la ficha.
    fn back_run(&mut self) {
        let Some(b) = self.run_back.pop() else {
            self.close_run();
            return;
        };
        self.run_detail = Some(b.detail);
        self.steps_scroll = b.steps_scroll;
        self.launched_sel = b.launched_sel;
        self.refresh_run();
    }

    /// Hay una ficha de ejecución a la que Esc vuelve (el pie dice «back»
    /// en lugar de cerrar).
    #[cfg_attr(not(test), allow(dead_code))] // hasta que visual lo pinte
    pub fn can_go_back(&self) -> bool {
        self.run_detail.is_some() && !self.run_back.is_empty()
    }

    /// La ficha abierta la lanzó otra ejecución que sigue en los datos: `p`
    /// abre su ficha. El pie solo anuncia `p parent` si es cierto.
    pub fn can_open_parent(&self) -> bool {
        self.parent_id().is_some()
    }

    fn parent_id(&self) -> Option<String> {
        match self.run_detail.as_ref()?.launched_by.as_ref()? {
            LaunchedBy::Run(link) => Some(link.id.clone()),
            LaunchedBy::Session { .. } | LaunchedBy::GoneRun { .. } => None,
        }
    }

    /// Cuántas hijas tiene la ficha abierta en «launched».
    fn launched_len(&self) -> usize {
        self.run_detail.as_ref().map_or(0, |d| d.launched.len())
    }

    /// Tab (`dir > 0`) y Shift-Tab: la hija siguiente o anterior de
    /// «launched». Sin elegida, Tab va a la primera y Shift-Tab a la última;
    /// pasada la última (o antes de la primera) no queda ninguna elegida.
    fn step_launched(&mut self, dir: i64) {
        let len = self.launched_len();
        if len == 0 {
            return;
        }
        self.launched_sel = match (self.launched_sel, dir > 0) {
            (None, true) => Some(0),
            (None, false) => Some(len - 1),
            (Some(i), true) => (i + 1 < len).then_some(i + 1),
            (Some(i), false) => i.checked_sub(1),
        };
    }

    /// Enter con una hija elegida: abre su ficha (Esc vuelve aquí).
    fn open_launched(&mut self) {
        let id = self
            .run_detail
            .as_ref()
            .zip(self.launched_sel)
            .and_then(|(d, i)| d.launched.get(i))
            .map(|l| l.id.clone());
        if let Some(id) = id {
            self.push_run(&id);
        }
    }

    /// Un dígito tecleado sobre una lista de `total` líneas numeradas desde 1:
    /// el número que forman con los anteriores si cae en rango. Deja en
    /// `pending` lo tecleado si otro dígito aún podría formar uno válido.
    fn type_digit(&mut self, d: usize, total: usize) -> Option<usize> {
        let n = self.pending.unwrap_or(0) * 10 + d;
        self.pending = (n * 10 <= total).then_some(n);
        (1..=total).contains(&n).then_some(n)
    }

    /// Cierra la pantalla de la ejecución. Con la pantalla del agente debajo
    /// se vuelve a ella tal como estaba; sin ella (su bloque salió de la
    /// ventana mientras se miraba la ejecución) se vuelve a activos, con el
    /// cursor en el bloque que contiene esa ejecución si sigue en la lista.
    fn close_run(&mut self) {
        if let Some(d) = self.run_detail.take() {
            if self.agent_screen.is_none() {
                if let Some((n, _, _)) = self.live_line_of(&d.id) {
                    self.cursor = n - 1;
                }
            }
        }
        self.steps_scroll = 0;
        self.launched_sel = None;
        self.run_back.clear();
    }

    /// En la pantalla de la ejecución, → y ← pasan a la ejecución siguiente
    /// y a la anterior del bloque de agente de la abierta, en el orden y la
    /// numeración de la pantalla del agente (`siblings`): → hacia la más
    /// antigua (número mayor) y ← hacia la primera. En los extremos, o sin
    /// hermanas, no hacen nada. Si la abierta es la que se abrió desde la
    /// pantalla del agente (no una hija ni la madre, que tienen pila de
    /// vuelta), el cursor de esa pantalla la sigue: Esc vuelve sobre ella.
    fn step_sibling(&mut self, dir: i64) {
        let Some(d) = &self.run_detail else { return };
        let ids = self.siblings(&d.id);
        let Some(pos) = ids.iter().position(|i| *i == d.id) else {
            return;
        };
        let next = if dir > 0 {
            Some(pos + 1)
        } else {
            pos.checked_sub(1)
        };
        if let Some(id) = next.and_then(|i| ids.get(i)) {
            let id = id.clone();
            if self.open_run(&id) {
                if self.run_back.is_empty() {
                    self.agent_follow(&id);
                }
                if let Some((n, _, _)) = self.live_line_of(&id) {
                    self.cursor = n - 1;
                }
            }
        }
    }

    /// Tope de `steps_scroll`: el último paso, o lo que deja una última
    /// página llena si ya se conoce el alto.
    fn steps_max(&self) -> usize {
        let len = self.run_detail.as_ref().map_or(0, |d| d.steps.len());
        len.saturating_sub(self.steps_page.max(1))
    }

    /// Desplaza la lista de pasos `delta` líneas (negativo, hacia arriba),
    /// acotado a `0..=steps_max`.
    fn scroll_steps(&mut self, delta: i64) {
        let max = self.steps_max() as i64;
        self.steps_scroll = (self.steps_scroll as i64 + delta).clamp(0, max) as usize;
    }

    /// Para la vista: guarda `height` (líneas de pasos que caben) como alto
    /// de página, acota `steps_scroll` a lo que deja la última página llena y
    /// devuelve el offset a pintar.
    pub fn steps_window(&mut self, height: usize) -> usize {
        self.steps_page = height;
        self.steps_scroll = self.steps_scroll.min(self.steps_max());
        self.steps_scroll
    }

    /// Pone al día la ficha de ejecución abierta con las ejecuciones actuales.
    fn refresh_run(&mut self) {
        if let Some(d) = &mut self.run_detail {
            d.refresh(status::activity_detail(
                &self.agents,
                &self.calls,
                &self.runs,
                &d.id,
                self.now,
            ));
        }
    }

    /// Teclas de la pantalla de la ejecución. Se come todo lo que no es suyo
    /// (el filtro, la pestaña…): lo de debajo no cambia mientras se mira.
    /// ↑↓ ya no cambian de agente: mueven los pasos una línea.
    fn run_key(&mut self, k: KeyEvent) {
        let page = self.steps_page.max(1) as i64;
        if let Some(d) = match k.code {
            KeyCode::Char(c) => c.to_digit(10),
            _ => None,
        } {
            let total = self.launched_len();
            if let Some(n) = self.type_digit(d as usize, total) {
                self.launched_sel = Some(n - 1);
            }
            return;
        }
        self.pending = None;
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Enter if self.launched_sel.is_some() => self.open_launched(),
            KeyCode::Esc | KeyCode::Enter => self.back_run(),
            KeyCode::Tab => self.step_launched(1),
            KeyCode::BackTab => self.step_launched(-1),
            KeyCode::Char('p') => {
                if let Some(id) = self.parent_id() {
                    self.push_run(&id);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.scroll_steps(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_steps(-1),
            KeyCode::Left | KeyCode::Char('h') => self.step_sibling(-1),
            KeyCode::Right | KeyCode::Char('l') => self.step_sibling(1),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_steps(page),
            KeyCode::PageUp => self.scroll_steps(-page),
            KeyCode::Home | KeyCode::Char('g') => self.steps_scroll = 0,
            KeyCode::End | KeyCode::Char('G') => self.steps_scroll = self.steps_max(),
            _ => {}
        }
    }

    pub fn take_rescan(&mut self) -> bool {
        std::mem::take(&mut self.rescan)
    }

    pub fn update(&mut self, action: Action) {
        match action {
            Action::Tick(now) => {
                self.now = now;
                self.refresh_run();
            }
            Action::Msg(Msg::Agents(agents)) => {
                self.agents = agents;
                self.agents_in = true;
                self.error = None;
            }
            Action::Msg(Msg::Activity {
                calls,
                runs,
                sessions,
            }) => {
                self.calls = calls;
                self.runs = runs;
                self.sessions = sessions;
                self.loaded = self.agents_in;
                self.error = None;
                self.refresh_run();
            }
            Action::Msg(Msg::Error(e)) => self.error = Some(e),
            Action::Key(k) => self.key(k),
            // En la ficha no hay lista que mover.
            Action::Scroll(_) if self.detail || self.run_detail.is_some() => {}
            Action::Scroll(delta) if self.agent_screen.is_some() => {
                self.agent_scroll_by(delta.into())
            }
            Action::Scroll(delta) => self.scroll_by(delta.into()),
            Action::Click { column, row } => self.click(column, row),
            Action::UpdateAvailable(v) => self.update_available = Some(v),
        }
        self.refresh_agent();
        self.clamp();
    }

    /// Clic en la pantalla. Una pestaña cambia de modo como Tab; una fila
    /// mueve el cursor a ella y, si ya estaba, hace lo mismo que Enter. Con la
    /// ficha de agente abierta solo responden las pestañas (y la cierran);
    /// con la pantalla de la ejecución o el filtro en edición no responde
    /// nada. En la pantalla del agente (sin pestañas) una ejecución se elige
    /// con un clic y se abre con otro (`agent_click`).
    fn click(&mut self, column: u16, row: u16) {
        if self.run_detail.is_some() || self.filtering {
            return;
        }
        self.pending = None;
        if self.agent_screen.is_some() && !self.detail {
            self.agent_click(row);
            return;
        }
        if let Some(tab) = self
            .hits
            .tabs
            .iter()
            .find(|t| t.row == row && t.cols.contains(&column))
        {
            self.mode = tab.mode;
            self.detail = false;
            // la ficha se abrió desde la pantalla del agente: ya no se vuelve
            self.agent_screen = None;
            return;
        }
        if self.detail {
            return;
        }
        if let Some(hit) = self.hits.rows.iter().find(|h| h.row == row) {
            if hit.cursor == self.cursor {
                self.activate();
            } else {
                self.cursor = hit.cursor;
            }
        }
    }

    /// Enter sobre la línea del cursor: despliega o pliega un grupo, abre la
    /// pantalla del agente de un bloque de activos o la ficha del agente.
    fn activate(&mut self) {
        if let Some(title) = self.folded_at_cursor() {
            if !self.open.remove(&title) {
                self.open.insert(title);
            }
        } else if let Some((line, _)) = self.live_at_cursor() {
            self.open_agent(&line.id);
        } else if self.current().is_some() {
            self.detail = true;
        }
    }

    fn clamp(&mut self) {
        let n = self.count();
        self.cursor = self.cursor.min(n.saturating_sub(1));
        if self.detail && self.current().is_none() {
            self.detail = false;
        }
        let launched = self.launched_len();
        self.launched_sel = self
            .launched_sel
            .filter(|_| launched > 0)
            .map(|i| i.min(launched - 1));
        self.steps_scroll = self.steps_scroll.min(self.steps_max());
    }

    fn key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.run_detail.is_some() {
            self.run_key(k);
            return;
        }
        if self.agent_screen.is_some() {
            if !self.detail {
                self.agent_key(k);
                return;
            }
            // La ficha del agente abierta con `a` desde su pantalla: Esc y
            // Enter vuelven a ella. Cualquier otra tecla es de la ficha de
            // siempre y actúa sobre la lista (↑↓ otro agente, tab…): la
            // pantalla del agente se deja y Esc ya vuelve a la lista.
            if !matches!(k.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                self.agent_screen = None;
            }
        }
        if self.filtering {
            match k.code {
                KeyCode::Char(c) => self.filter.push(c),
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Enter => self.filtering = false,
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filtering = false;
                }
                _ => {}
            }
            return;
        }
        let total = self.count();
        let digit = match k.code {
            KeyCode::Char(c) => c.to_digit(10).map(|d| d as usize),
            _ => None,
        };
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc if self.detail => self.detail = false,
            KeyCode::Enter if self.detail => self.detail = false,
            KeyCode::Enter => self.activate(),
            KeyCode::Down | KeyCode::Char('j') if self.detail => self.step_agent(1),
            KeyCode::Up | KeyCode::Char('k') if self.detail => self.step_agent(-1),
            KeyCode::Down | KeyCode::Char('j') => self.cursor += 1,
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Home | KeyCode::Char('g') => self.cursor = 0,
            KeyCode::End | KeyCode::Char('G') => self.cursor = total.saturating_sub(1),
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Tab => self.mode = self.mode.next(),
            KeyCode::Char('a') => self.mode = self.mode.toggle(),
            KeyCode::Char('o') if self.mode == Mode::All => self.order = self.order.toggle(),
            KeyCode::Char('p') if self.mode == Mode::Ranking => self.period = self.period.next(),
            KeyCode::Char('s') if self.mode == Mode::Ranking => {
                self.rank_key = self.rank_key.next()
            }
            KeyCode::PageDown | KeyCode::Char(' ') => self.page_by(1),
            KeyCode::PageUp => self.page_by(-1),
            KeyCode::Char('r') => self.rescan = true,
            KeyCode::Char(_) if digit.is_some() => {
                // Si otro dígito aún podría formar un número válido, se espera.
                if let Some(n) = self.type_digit(digit.unwrap_or(0), total) {
                    self.cursor = n - 1;
                }
                return;
            }
            _ => {}
        }
        self.pending = None;
    }
}

/// Las filas de un grupo de activos que deja `keep`, en racimos (bloques
/// por agente) rehechos sobre lo que queda: un racimo empieza en la `Head` o
/// la `Single` de `status`, esté en curso o terminada (la terminada de un
/// agente cierra su racimo), y sus supervivientes se vuelven a marcar (una
/// sola, `Single`; varias, `Head(N)`…`Last`). No se reordena nada. Un racimo
/// que el filtro deja vacío no sale.
fn rechain<'a>(
    lines: Vec<status::NowRun<'a>>,
    keep: impl Fn(&status::NowRun) -> bool,
) -> Vec<Vec<status::NowRun<'a>>> {
    let mut clusters: Vec<Vec<status::NowRun<'a>>> = Vec::new();
    for l in lines {
        if matches!(l.chain, Chain::Single | Chain::Head(_)) || clusters.is_empty() {
            clusters.push(Vec::new());
        }
        if keep(&l) {
            clusters.last_mut().expect("recién creado").push(l);
        }
    }
    clusters.retain(|c| !c.is_empty());
    for c in &mut clusters {
        let n = c.len();
        for (i, l) in c.iter_mut().enumerate() {
            l.chain = match (n, i) {
                (1, _) => Chain::Single,
                (_, 0) => Chain::Head(n),
                (_, i) if i + 1 == n => Chain::Last,
                _ => Chain::Middle,
            };
        }
    }
    clusters
}

/// La línea de activos de una fila de `status`, con datos propios.
fn now_line(r: &status::NowRun) -> NowLine {
    NowLine {
        id: r.id.clone(),
        name: r.name.to_string(),
        task: r.task.map(String::from),
        since: r.since,
        worked: r.worked,
        kind: r.kind,
        skill: r.skill.map(String::from),
        last_tool: r.last_tool.map(String::from),
        state: r.state,
        runs: r.runs,
        tokens: r.tokens,
        context: r.context,
        chain: r.chain,
    }
}

/// `haystack` contiene `needle` (ya en minúsculas) sin mirar mayúsculas.
fn contains(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

/// Parte filas ya ordenadas por grupo en un trozo por grupo seguido.
fn chunk_by_section(rows: &mut [Row]) -> Vec<&mut [Row]> {
    let mut out = Vec::new();
    let mut rest = rows;
    while !rest.is_empty() {
        let len = rest
            .iter()
            .position(|r| r.section != rest[0].section)
            .unwrap_or(rest.len());
        let (head, tail) = std::mem::take(&mut rest).split_at_mut(len);
        out.push(head);
        rest = tail;
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::agents::Scope;
    use chrono::DurationRound;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::Path;

    const NOW: &str = "2026-11-05T12:00:00Z";

    fn agent(name: &str, dir: Option<&str>) -> Agent {
        let (scope, path) = match dir {
            Some(d) => (
                Scope::Project(PathBuf::from(d)),
                format!("{d}/.claude/agents/{name}.md"),
            ),
            None => (Scope::Global, format!("/Users/x/.claude/agents/{name}.md")),
        };
        Agent {
            name: name.into(),
            path: PathBuf::from(path),
            scope,
            description: Some(format!("Agente {name}")),
        }
    }

    #[test]
    fn update_available_starts_empty_and_keeps_the_new_version() {
        let mut app = App::new(10, PathBuf::from("/Users/x"), "0.1.0", NOW.parse().unwrap());
        assert_eq!(app.update_available, None);
        app.update(Action::UpdateAvailable("0.2.0".into()));
        assert_eq!(app.update_available.as_deref(), Some("0.2.0"));
    }

    pub(crate) fn call(agent: &str, cwd: &str, at: &str) -> Invocation {
        Invocation {
            agent: agent.into(),
            cwd: PathBuf::from(cwd),
            at: at.parse().unwrap(),
            id: None,
            description: None,
            prompt_line: None,
            launch_dir: None,
        }
    }

    /// Dos proyectos y un global; `backend` tiene una llamada fuera de la ventana.
    /// Actividad más reciente: taskflow (hace 5 s), global (15 s), station (2 min).
    pub(crate) fn fixture() -> App {
        let mut app = App::new(10, PathBuf::from("/Users/x"), "0.1.0", NOW.parse().unwrap());
        app.mode = Mode::All; // los tests de la lista asumen la vista «todos»
        app.tz = Some(utc());
        app.update(Action::Msg(Msg::Agents(vec![
            agent("backend", Some("/Users/x/Offing/argi/station")),
            agent("docs", Some("/Users/x/Offing/argi/station")),
            agent("frontend", Some("/Users/x/Offing/argi/station")),
            agent("visual", Some("/Users/x/Offing/taskflow")),
            agent("react-structure-checker", None),
        ])));
        app.update(Action::Msg(Msg::Activity {
            calls: vec![
                call(
                    "frontend",
                    "/Users/x/Offing/argi/station",
                    "2026-11-05T11:58:00Z",
                ),
                call(
                    "docs",
                    "/Users/x/Offing/argi/station/front",
                    "2026-11-05T11:52:00Z",
                ),
                call(
                    "backend",
                    "/Users/x/Offing/argi/station",
                    "2026-11-05T11:30:00Z",
                ),
                call("visual", "/Users/x/Offing/taskflow", "2026-11-05T11:59:55Z"),
                call(
                    "react-structure-checker",
                    "/Users/x/Offing/taskflow",
                    "2026-11-05T11:59:45Z",
                ),
            ],
            runs: vec![],
            sessions: 4,
        }));
        app
    }

    /// `n` agentes globales, todos con una llamada antigua a la misma hora: un
    /// solo grupo, sin plegar, ordenado por nombre.
    fn many(n: usize) -> App {
        let mut app = App::new(10, PathBuf::from("/Users/x"), "0.1.0", NOW.parse().unwrap());
        app.mode = Mode::All; // estos tests asumen la vista «todos»
        app.tz = Some(utc());
        app.update(Action::Msg(Msg::Agents(
            (0..n).map(|i| agent(&format!("a{i:02}"), None)).collect(),
        )));
        app.update(Action::Msg(Msg::Activity {
            calls: (0..n)
                .map(|i| call(&format!("a{i:02}"), "/Users/x/p", "2026-10-01T10:00:00Z"))
                .collect(),
            runs: vec![],
            sessions: 0,
        }));
        app
    }

    pub(crate) fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    pub(super) fn key(app: &mut App, code: KeyCode) {
        app.update(Action::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn names(app: &App) -> Vec<String> {
        app.rows()
            .iter()
            .map(|r| app.agent(r.agent).name.clone())
            .collect()
    }

    fn sections(app: &App) -> Vec<(String, usize, usize)> {
        app.items()
            .iter()
            .filter_map(|i| match i {
                Item::Section {
                    title,
                    active,
                    total,
                    ..
                } => Some((title.clone(), *active, *total)),
                _ => None,
            })
            .collect()
    }

    fn row_named(app: &App, name: &str) -> Row {
        app.rows()
            .into_iter()
            .find(|r| app.agent(r.agent).name == name)
            .unwrap()
    }

    #[test]
    fn groups_with_activity_go_first_most_recent_on_top() {
        assert_eq!(
            sections(&fixture()),
            vec![
                ("~/Offing/taskflow".to_string(), 1, 1),
                ("global".to_string(), 1, 1),
                ("~/Offing/argi/station".to_string(), 2, 3),
            ]
        );
    }

    #[test]
    fn groups_never_used_go_folded_by_name_with_global_last() {
        let mut app = fixture();
        app.update(Action::Msg(Msg::Activity {
            calls: vec![],
            runs: vec![],
            sessions: 0,
        }));
        assert!(sections(&app).is_empty());
        let titles: Vec<String> = folded(&app).into_iter().map(|f| f.0).collect();
        assert_eq!(
            titles,
            vec!["~/Offing/argi/station", "~/Offing/taskflow", "global"]
        );
    }

    #[test]
    fn rows_put_active_first_by_recency_then_name() {
        assert_eq!(
            names(&fixture()),
            vec![
                "visual",
                "react-structure-checker",
                "frontend",
                "docs",
                "backend"
            ]
        );
    }

    #[test]
    fn rows_are_numbered_across_sections() {
        let app = fixture();
        let last = app.rows().pop().unwrap();
        assert_eq!(last.n, 5);
        assert_eq!(app.agent(last.agent).name, "backend");
    }

    #[test]
    fn calls_older_than_the_window_do_not_count() {
        let app = fixture();
        assert!(
            row_named(&app, "frontend").active(),
            "frontend tuvo una llamada hace 2 min"
        );
        assert!(
            !row_named(&app, "backend").active(),
            "backend solo tiene una de hace 30 min"
        );
        assert_eq!(app.active_count(), 4);
    }

    #[test]
    fn cursor_moves_and_clamps() {
        let mut app = fixture();
        for _ in 0..6 {
            key(&mut app, KeyCode::Down);
        }
        assert_eq!(app.cursor, 4);
        for _ in 0..9 {
            key(&mut app, KeyCode::Char('k'));
        }
        assert_eq!(app.cursor, 0);
        key(&mut app, KeyCode::End);
        assert_eq!(app.cursor, 4);
    }

    #[test]
    fn digits_jump_to_the_row() {
        let mut app = fixture();
        key(&mut app, KeyCode::Char('4'));
        assert_eq!(app.cursor, 3);
        let mut app = many(12);
        key(&mut app, KeyCode::Char('1'));
        key(&mut app, KeyCode::Char('2'));
        assert_eq!(app.cursor, 11, "dos dígitos seguidos forman el 12");
        key(&mut app, KeyCode::Char('3'));
        assert_eq!(
            app.cursor, 2,
            "el buffer se vació al no caber un tercer dígito"
        );
    }

    #[test]
    fn active_mode_hides_inactive_rows_and_keeps_cursor_valid() {
        let mut app = fixture();
        key(&mut app, KeyCode::End);
        key(&mut app, KeyCode::Char('a'));
        assert_eq!(
            names(&app),
            vec!["visual", "react-structure-checker", "frontend", "docs"]
        );
        assert_eq!(app.cursor, 3);
        key(&mut app, KeyCode::Char('a'));
        assert_eq!(app.rows().len(), 5);
    }

    #[test]
    fn filter_matches_name_or_project_and_esc_clears_it() {
        let mut app = fixture();
        key(&mut app, KeyCode::Char('/'));
        for c in "task".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        assert!(app.filtering);
        assert_eq!(names(&app), vec!["visual"]);
        key(&mut app, KeyCode::Enter);
        assert!(
            !app.filtering,
            "enter deja de editar pero conserva el filtro"
        );
        assert_eq!(app.filter, "task");
        key(&mut app, KeyCode::Char('/'));
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.filter, "");
        assert_eq!(app.rows().len(), 5);
    }

    #[test]
    fn a_new_app_opens_on_the_active_view_with_a_clean_state() {
        let app = App::new(10, PathBuf::from("/Users/x"), "0.1.0", NOW.parse().unwrap());
        assert_eq!(app.mode, Mode::Active);
        assert_eq!((app.cursor, app.scroll), (0, 0));
        assert!(app.filter.is_empty() && !app.filtering && !app.detail);
    }

    #[test]
    fn while_filtering_letters_go_to_the_filter() {
        let mut app = fixture();
        key(&mut app, KeyCode::Char('/'));
        key(&mut app, KeyCode::Char('a'));
        key(&mut app, KeyCode::Char('q'));
        assert_eq!(app.filter, "aq");
        assert_eq!(app.mode, Mode::All);
        assert!(!app.quit);
        key(&mut app, KeyCode::Backspace);
        assert_eq!(app.filter, "a");
    }

    #[test]
    fn enter_opens_the_detail_and_esc_closes_it() {
        let mut app = fixture();
        key(&mut app, KeyCode::Enter);
        assert!(app.detail);
        assert_eq!(app.agent(app.current().unwrap().agent).name, "visual");
        key(&mut app, KeyCode::Esc);
        assert!(!app.detail);
    }

    #[test]
    fn arrows_in_the_detail_move_to_the_next_agent() {
        let mut app = fixture();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Down);
        assert!(app.detail, "la ficha sigue abierta");
        assert_eq!(
            app.agent(app.current().unwrap().agent).name,
            "react-structure-checker"
        );
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        let mut app = fixture();
        key(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
        let mut app = fixture();
        app.update(Action::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.quit);
    }

    #[test]
    fn activity_message_replaces_the_calls() {
        let mut app = fixture();
        app.update(Action::Msg(Msg::Activity {
            calls: vec![],
            runs: vec![],
            sessions: 1,
        }));
        assert_eq!(app.active_count(), 0);
        assert_eq!(app.sessions, 1);
    }

    #[test]
    fn r_requests_a_rescan_once() {
        let mut app = fixture();
        assert!(!app.take_rescan());
        key(&mut app, KeyCode::Char('r'));
        assert!(app.take_rescan());
        assert!(!app.take_rescan());
    }

    #[test]
    fn calls_of_the_current_agent_are_listed_for_the_detail() {
        let app = fixture();
        let row = app.current().unwrap();
        let calls = app.calls_of(row.agent);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].cwd, Path::new("/Users/x/Offing/taskflow"));
        assert!(calls[0].in_window);
    }

    #[test]
    fn calls_of_cover_the_whole_history_and_mark_the_window() {
        let mut app = fixture();
        app.update(Action::Msg(Msg::Activity {
            calls: vec![
                call(
                    "docs",
                    "/Users/x/Offing/argi/station",
                    "2026-10-01T08:00:00Z",
                ),
                call(
                    "docs",
                    "/Users/x/Offing/argi/station",
                    "2026-11-05T11:55:00Z",
                ),
                call(
                    "docs",
                    "/Users/x/Offing/argi/station",
                    "2026-11-04T08:00:00Z",
                ),
                call("docs", "/Users/x/Offing/otro", "2026-11-05T11:59:00Z"),
            ],
            runs: vec![],
            sessions: 1,
        }));
        let docs = row_named(&app, "docs").agent;
        let got: Vec<(String, bool)> = app
            .calls_of(docs)
            .iter()
            .map(|c| (c.at.to_rfc3339(), c.in_window))
            .collect();
        assert_eq!(
            got,
            vec![
                ("2026-11-05T11:55:00+00:00".to_string(), true),
                ("2026-11-04T08:00:00+00:00".to_string(), false),
                ("2026-10-01T08:00:00+00:00".to_string(), false),
            ],
            "la más reciente primero; la de otro proyecto no cuenta"
        );
    }

    #[test]
    fn rows_know_the_last_call_ever_besides_the_window() {
        let app = fixture();
        let backend = row_named(&app, "backend");
        assert!(!backend.active());
        assert_eq!(backend.last, None);
        assert_eq!(backend.ever, Some("2026-11-05T11:30:00Z".parse().unwrap()));
        let visual = row_named(&app, "visual");
        assert_eq!(visual.last, visual.ever);
    }

    /// Un grupo activo, dos con llamadas antiguas, uno nunca usado y global
    /// nunca usado: el orden de «todos» y el de dentro de cada grupo.
    fn history() -> App {
        let mut app = App::new(10, PathBuf::from("/Users/x"), "0.1.0", NOW.parse().unwrap());
        app.mode = Mode::All; // estos tests asumen la vista «todos»
        app.tz = Some(utc());
        app.update(Action::Msg(Msg::Agents(vec![
            agent("zeta", Some("/Users/x/a-viejo")),
            agent("uno", Some("/Users/x/b-ayer")),
            agent("dos", Some("/Users/x/b-ayer")),
            agent("tres", Some("/Users/x/b-ayer")),
            agent("cuatro", Some("/Users/x/b-ayer")),
            agent("nada", Some("/Users/x/c-nunca")),
            agent("ahora", Some("/Users/x/d-ahora")),
            agent("suelto", None),
        ])));
        app.update(Action::Msg(Msg::Activity {
            calls: vec![
                call("zeta", "/Users/x/a-viejo", "2026-09-01T10:00:00Z"),
                call("uno", "/Users/x/b-ayer", "2026-11-04T10:00:00Z"),
                call("dos", "/Users/x/b-ayer", "2026-11-04T12:00:00Z"),
                call("ahora", "/Users/x/d-ahora", "2026-11-05T11:59:00Z"),
            ],
            runs: vec![],
            sessions: 0,
        }));
        app
    }

    #[test]
    fn all_mode_orders_groups_by_window_then_by_last_call_ever_then_never() {
        let titles: Vec<String> = sections(&history()).into_iter().map(|s| s.0).collect();
        assert_eq!(titles, vec!["~/d-ahora", "~/b-ayer", "~/a-viejo"]);
        let folded: Vec<String> = folded(&history()).into_iter().map(|f| f.0).collect();
        assert_eq!(
            folded,
            vec!["~/c-nunca", "global"],
            "los nunca usados, plegados"
        );
    }

    #[test]
    fn inside_a_group_inactive_go_by_last_call_ever_and_never_used_by_name() {
        assert_eq!(
            names(&history()),
            vec!["ahora", "dos", "uno", "cuatro", "tres", "zeta"]
        );
    }

    #[test]
    fn mode_starts_in_all_tab_cycles_three_and_a_toggles_active_all() {
        let mut app = fixture();
        assert_eq!(app.mode, Mode::All);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.mode, Mode::Ranking);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.mode, Mode::Active);
        assert_eq!(app.rows().len(), 4);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.mode, Mode::All);
        key(&mut app, KeyCode::Char('a'));
        assert_eq!(app.mode, Mode::Active);
        key(&mut app, KeyCode::Char('a'));
        assert_eq!(app.mode, Mode::All);
        assert_eq!(app.rows().len(), 5);
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Char('a'));
        assert_eq!(app.mode, Mode::Active, "desde el ranking, 'a' va a activos");
    }

    /// Una ejecución de `agent_type` en `cwd` que empezó en `start` y trabajó
    /// `mins` minutos, lanzada por la llamada `id`; terminada si `finished`.
    pub(crate) fn run(
        agent_type: &str,
        cwd: &str,
        start: &str,
        mins: i64,
        id: Option<&str>,
        finished: bool,
    ) -> Run {
        let start: DateTime<Utc> = start.parse().unwrap();
        let last = start + Duration::minutes(mins);
        Run {
            agent_type: agent_type.into(),
            description: Some(format!("{agent_type}: tarea")),
            tool_use_id: id.map(String::from),
            session_id: None,
            cwd: Some(PathBuf::from(cwd)),
            temp_cwd: None,
            start: Some(start),
            last: Some(last),
            worked: Duration::minutes(mins),
            worked_slots: std::collections::BTreeMap::from([(
                start.duration_trunc(crate::runs::SLOT).unwrap(),
                Duration::minutes(mins),
            )]),
            tools: 7,
            task: None,
            task_full: None,
            last_tool: None,
            steps: Default::default(),
            tool_counts: Default::default(),
            finished: Some(finished),
            modified: last,
            ..Default::default()
        }
    }

    pub(crate) fn with_runs(mut app: App, runs: Vec<Run>) -> App {
        let calls = app.calls.clone();
        app.update(Action::Msg(Msg::Activity {
            calls,
            runs,
            sessions: 4,
        }));
        app
    }

    const STATION: &str = "/Users/x/Offing/argi/station";
    const STATION_SECTION: &str = "~/Offing/argi/station";

    #[test]
    fn rows_carry_the_work_of_every_run() {
        let app = with_runs(
            fixture(),
            vec![
                run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true),
                run("docs", STATION, "2026-11-03T09:00:00Z", 22, None, true),
                run(
                    "docs",
                    "/Users/x/otro",
                    "2026-11-03T09:00:00Z",
                    99,
                    None,
                    true,
                ),
            ],
        );
        let docs = row_named(&app, "docs");
        assert_eq!(docs.work.total, Duration::minutes(72));
        assert_eq!(docs.work.runs, 2, "la de otro proyecto no cuenta");
        assert_eq!(docs.work.running, None);
        assert_eq!(row_named(&app, "visual").work.runs, 0);
    }

    #[test]
    fn a_run_in_progress_makes_the_agent_active_without_a_call_in_the_window() {
        // backend solo tiene una llamada de hace 30 min, pero trabaja ahora
        let app = with_runs(
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
        let backend = row_named(&app, "backend");
        assert!(backend.active());
        assert_eq!(
            backend.last,
            Some("2026-11-05T11:58:00Z".parse().unwrap()),
            "el último paso de la ejecución cuenta como uso"
        );
        let running = backend.work.running.unwrap();
        assert_eq!(running.since, Some("2026-11-05T11:50:00Z".parse().unwrap()));
        assert_eq!(running.worked, Duration::minutes(10));
        assert_eq!(app.active_count(), 5, "cuenta en la cabecera");
    }

    #[test]
    fn active_mode_shows_runs_in_progress() {
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
        key(&mut app, KeyCode::Char('a'));
        assert!(names(&app).contains(&"backend".to_string()));
    }

    #[test]
    fn an_interrupted_run_does_not_make_the_agent_active() {
        // sin terminar pero sin tocarse desde hace más de 10 min
        let app = with_runs(
            fixture(),
            vec![run(
                "backend",
                STATION,
                "2026-11-05T11:00:00Z",
                20,
                None,
                false,
            )],
        );
        let backend = row_named(&app, "backend");
        assert!(!backend.active());
        assert_eq!(backend.work.running, None);
        assert_eq!(backend.work.total, Duration::minutes(20));
        assert_eq!(app.active_count(), 4);
    }

    #[test]
    fn inside_an_active_group_runs_in_progress_go_first() {
        // en station, frontend (11:58) y docs (11:52) están activos; backend
        // trabaja ahora y pasa delante de los dos
        let app = with_runs(
            fixture(),
            vec![run(
                "backend",
                STATION,
                "2026-11-05T11:55:00Z",
                4,
                None,
                false,
            )],
        );
        let station: Vec<String> = app
            .rows()
            .iter()
            .filter(|r| r.section == "~/Offing/argi/station")
            .map(|r| app.agent(r.agent).name.clone())
            .collect();
        assert_eq!(station, vec!["backend", "frontend", "docs"]);
        assert_eq!(
            sections(&app)
                .iter()
                .find(|s| s.0 == "~/Offing/argi/station")
                .unwrap()
                .1,
            3
        );
    }

    #[test]
    fn a_group_with_a_run_in_progress_goes_first() {
        // station tiene la llamada menos reciente de los tres grupos activos
        let app = with_runs(
            fixture(),
            vec![run(
                "backend",
                STATION,
                "2026-11-05T11:55:00Z",
                4,
                None,
                false,
            )],
        );
        let titles: Vec<String> = sections(&app).into_iter().map(|s| s.0).collect();
        assert_eq!(
            titles,
            vec!["~/Offing/argi/station", "~/Offing/taskflow", "global"]
        );
    }

    #[test]
    fn a_running_agent_never_called_still_goes_first_in_its_group() {
        // una ejecución de workflow: sin llamada a Agent en la transcripción padre
        let app = with_runs(
            history(),
            vec![run(
                "cuatro",
                "/Users/x/b-ayer",
                "2026-11-05T11:58:00Z",
                1,
                None,
                false,
            )],
        );
        assert_eq!(
            names(&app),
            vec!["cuatro", "dos", "uno", "tres", "ahora", "zeta"]
        );
    }

    #[test]
    fn calls_in_the_detail_carry_their_linked_run() {
        let mut app = fixture();
        let mut done = call("docs", STATION, "2026-11-05T09:00:00Z");
        done.id = Some("toolu_1".into());
        let mut now = call("docs", STATION, "2026-11-05T11:55:00Z");
        now.id = Some("toolu_2".into());
        let old = call("docs", STATION, "2026-10-01T09:00:00Z");
        app.update(Action::Msg(Msg::Activity {
            calls: vec![done, now, old],
            runs: vec![
                run(
                    "docs",
                    STATION,
                    "2026-11-05T09:00:00Z",
                    12,
                    Some("toolu_1"),
                    true,
                ),
                run(
                    "docs",
                    STATION,
                    "2026-11-05T11:55:00Z",
                    4,
                    Some("toolu_2"),
                    false,
                ),
            ],
            sessions: 1,
        }));
        let docs = row_named(&app, "docs").agent;
        let calls = app.calls_of(docs);
        assert_eq!(calls.len(), 3);
        let running = calls[0].run.as_ref().unwrap();
        assert!(running.in_progress);
        assert_eq!(running.worked, Duration::minutes(5), "lleva hasta ahora");
        let done = calls[1].run.as_ref().unwrap();
        assert!(!done.in_progress);
        assert_eq!(done.worked, Duration::minutes(12));
        assert_eq!(done.tools, 7);
        assert_eq!(done.description, Some("docs: tarea"));
        assert!(calls[2].run.is_none(), "sin id no hay ejecución enlazada");
    }

    /// Lo que hace la pantalla en cada vuelta: ajustar la vista al cursor con
    /// el alto del cuerpo. Devuelve la línea del cursor.
    fn frame(app: &mut App, height: usize) -> usize {
        let items = app.items();
        app.scroll_to_cursor(&items, height);
        items
            .iter()
            .position(|i| i.n() == Some(app.cursor + 1))
            .unwrap()
    }

    #[test]
    fn wheel_moves_the_view_and_drags_the_cursor() {
        // 30 filas en global: las columnas en la línea 0, la sección en la 1 y
        // la fila n en la línea n + 1.
        let mut app = many(30);
        frame(&mut app, 10);
        app.update(Action::Scroll(3));
        assert_eq!(app.scroll, 3);
        assert_eq!(app.cursor, 1, "el cursor baja a la primera fila a la vista");
        frame(&mut app, 10);
        assert_eq!(app.scroll, 3, "la vista no vuelve atrás al repintar");
        app.update(Action::Scroll(-3));
        assert_eq!(app.scroll, 0);
        assert_eq!(app.cursor, 1, "el cursor sigue a la vista: no se mueve");
        app.update(Action::Scroll(-3));
        assert_eq!(app.scroll, 0, "no pasa del principio");
        app.update(Action::Scroll(100));
        assert_eq!(
            app.scroll, 22,
            "no pasa del final: 32 líneas, 10 a la vista"
        );
        assert_eq!(app.cursor, 20);
        key(&mut app, KeyCode::End);
        frame(&mut app, 10);
        app.update(Action::Scroll(-3));
        assert_eq!(app.scroll, 19);
        assert_eq!(app.cursor, 26, "el cursor sube a la última fila a la vista");
        assert_eq!(frame(&mut app, 10), 28);
        assert_eq!(app.scroll, 19);
    }

    #[test]
    fn wheel_does_not_leave_the_cursor_under_a_hidden_header() {
        let mut app = history();
        frame(&mut app, 4);
        // Líneas: 0 columnas, 1 d-ahora, 2 ahora, 3 blanco, 4 b-ayer, 5 dos…
        app.update(Action::Scroll(4));
        let line = frame(&mut app, 4);
        assert_eq!(app.scroll, 4, "repintar no deshace la rueda");
        assert!((4..8).contains(&line), "línea {line}");
    }

    #[test]
    fn page_keys_and_space_move_a_page() {
        let mut app = many(30);
        frame(&mut app, 10);
        key(&mut app, KeyCode::PageDown);
        assert_eq!(app.cursor, 10);
        frame(&mut app, 10);
        key(&mut app, KeyCode::Char(' '));
        assert_eq!(app.cursor, 20);
        frame(&mut app, 10);
        key(&mut app, KeyCode::PageDown);
        key(&mut app, KeyCode::PageDown);
        assert_eq!(app.cursor, 29, "se para en la última");
        frame(&mut app, 10);
        key(&mut app, KeyCode::PageUp);
        assert_eq!(app.cursor, 19);
        frame(&mut app, 10);
        key(&mut app, KeyCode::PageUp);
        key(&mut app, KeyCode::PageUp);
        assert_eq!(app.cursor, 0, "se para en la primera");
        assert_eq!(frame(&mut app, 10), 2);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn page_keys_also_move_the_view() {
        let mut app = many(30);
        frame(&mut app, 10);
        key(&mut app, KeyCode::PageDown);
        assert_eq!(app.scroll, 10);
        let line = frame(&mut app, 10);
        assert_eq!(app.scroll, 10);
        assert_eq!(line, 12);
    }

    #[test]
    fn space_while_filtering_goes_to_the_filter() {
        let mut app = many(30);
        key(&mut app, KeyCode::Char('/'));
        key(&mut app, KeyCode::Char(' '));
        assert_eq!(app.filter, " ");
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn scroll_follows_the_cursor() {
        let mut app = many(30);
        key(&mut app, KeyCode::End);
        let items = app.items();
        app.scroll_to_cursor(&items, 10);
        let line = items.iter().position(|i| i.n() == Some(30)).unwrap();
        assert!(
            line >= app.scroll && line < app.scroll + 10,
            "línea {line}, scroll {}",
            app.scroll
        );
        key(&mut app, KeyCode::Home);
        let items = app.items();
        app.scroll_to_cursor(&items, 10);
        assert_eq!(app.scroll, 0);
    }

    // --- Pestañas, franja «Ahora», grupos plegados, orden, ranking y ficha ---

    fn folded(app: &App) -> Vec<(String, usize, usize, bool)> {
        app.items()
            .iter()
            .filter_map(|i| match i {
                Item::Folded {
                    title,
                    total,
                    n,
                    open,
                } => Some((title.clone(), *total, *n, *open)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn never_used_groups_go_folded_at_the_end_numbered_after_the_rows() {
        let app = history();
        assert_eq!(
            folded(&app),
            vec![
                ("~/c-nunca".to_string(), 1, 7, false),
                ("global".to_string(), 1, 8, false)
            ]
        );
        assert!(matches!(app.items().last(), Some(Item::Folded { .. })));
        assert_eq!(app.count(), 8, "las líneas plegadas cuentan como filas");
    }

    #[test]
    fn digits_and_enter_open_and_close_a_folded_group() {
        let mut app = history();
        key(&mut app, KeyCode::Char('7'));
        assert_eq!(app.cursor, 6);
        assert!(app.current().is_none(), "un grupo plegado no es un agente");
        key(&mut app, KeyCode::Enter);
        assert!(!app.detail, "enter despliega, no abre la ficha");
        assert!(folded(&app)[0].3);
        assert_eq!(names(&app).last().unwrap(), "nada");
        assert_eq!(row_named(&app, "nada").n, 8);
        assert_eq!(folded(&app)[1], ("global".to_string(), 1, 9, false));
        assert_eq!(app.cursor, 6, "el cursor sigue en la línea del grupo");
        key(&mut app, KeyCode::Char('8'));
        key(&mut app, KeyCode::Enter);
        assert!(app.detail);
        assert_eq!(app.agent(app.current().unwrap().agent).name, "nada");
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('7'));
        key(&mut app, KeyCode::Enter);
        assert!(!folded(&app)[0].3, "enter otra vez lo pliega");
        assert_eq!(app.count(), 8);
    }

    #[test]
    fn active_mode_has_no_folded_groups() {
        let mut app = history();
        key(&mut app, KeyCode::Char('a'));
        assert!(folded(&app).is_empty());
        assert_eq!(names(&app), vec!["ahora"]);
    }

    #[test]
    fn arrows_in_the_detail_skip_folded_groups() {
        let mut app = history();
        key(&mut app, KeyCode::Char('7'));
        key(&mut app, KeyCode::Enter);
        // zeta 6, c-nunca 7 (abierto), nada 8, global 9 (plegado)
        key(&mut app, KeyCode::Char('6'));
        key(&mut app, KeyCode::Enter);
        assert!(app.detail);
        key(&mut app, KeyCode::Down);
        assert_eq!(app.agent(app.current().unwrap().agent).name, "nada");
        key(&mut app, KeyCode::Down);
        assert!(app.detail, "no hay otro agente: se queda");
        assert_eq!(app.agent(app.current().unwrap().agent).name, "nada");
        key(&mut app, KeyCode::Up);
        assert_eq!(app.agent(app.current().unwrap().agent).name, "zeta");
    }

    #[test]
    fn columns_header_goes_once_above_the_first_group() {
        let app = fixture();
        let items = app.items();
        assert!(matches!(items[0], Item::Columns));
        assert!(matches!(items[1], Item::Section { .. }));
        assert_eq!(
            items.iter().filter(|i| matches!(i, Item::Columns)).count(),
            1
        );
    }

    #[test]
    fn sections_carry_the_work_of_their_group() {
        let app = with_runs(
            fixture(),
            vec![
                run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true),
                run("frontend", STATION, "2026-11-04T10:00:00Z", 5, None, true),
            ],
        );
        let worked: Vec<(String, TimeDelta)> = app
            .items()
            .into_iter()
            .filter_map(|i| match i {
                Item::Section { title, worked, .. } => Some((title, worked)),
                _ => None,
            })
            .collect();
        assert!(worked.contains(&("~/Offing/argi/station".to_string(), Duration::minutes(55))));
        assert!(worked.contains(&("global".to_string(), TimeDelta::zero())));
    }

    // --- Monitor de «activos»: grupos con lo en curso y lo terminado ---

    /// Cuatro en curso (una built-in, una de workflow, otra built-in y una
    /// definida) y una terminada fuera de la ventana; más las cuatro llamadas
    /// sin ejecución del `fixture` que caen en la ventana (terminadas).
    ///
    /// Orden agrupado, con su número:
    /// - `~/Offing/taskflow`: 1 visual, 2 react-structure-checker (terminadas)
    /// - `~/Offing/argi/station`: 3 Explore, 4 workflow, 5 backend (en curso),
    ///   6 frontend (terminada)
    /// - `~/otro`: 7 general-purpose (en curso)
    /// - `~/Offing/argi/station/front`: 8 docs (terminada)
    pub(super) fn busy() -> App {
        with_runs(
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
                run(
                    "workflow-subagent",
                    STATION,
                    "2026-11-05T11:56:00Z",
                    3,
                    None,
                    false,
                ),
                run("Explore", STATION, "2026-11-05T11:57:00Z", 2, None, false),
                run("docs", STATION, "2026-11-05T09:00:00Z", 12, None, true),
            ],
        )
    }

    pub(super) fn active(mut app: App) -> App {
        key(&mut app, KeyCode::Char('a'));
        assert_eq!(app.mode, Mode::Active);
        app
    }

    /// Lo seleccionable del monitor: (número, nombre, agente definido).
    fn monitor(app: &App) -> Vec<(usize, String, Option<String>)> {
        app.items()
            .into_iter()
            .filter_map(|i| match i {
                Item::LiveAgent { n, line, row } => {
                    Some((n, line.name, row.map(|r| app.agent(r.agent).name.clone())))
                }
                _ => None,
            })
            .collect()
    }

    /// Las reglas de grupo: (proyecto, filas en curso, filas terminadas).
    /// Comprueba de paso que los contadores cuadran con las filas que siguen.
    fn live_groups(app: &App) -> Vec<(String, usize, usize)> {
        let mut out: Vec<(String, usize, usize)> = Vec::new();
        let mut seen: Vec<(usize, usize)> = Vec::new();
        for i in app.items() {
            match i {
                Item::LiveGroup {
                    section,
                    running,
                    recent,
                } => {
                    out.push((section, running, recent));
                    seen.push((0, 0));
                }
                Item::Live { line, .. } => {
                    let g = seen.last_mut().expect("una fila va bajo su grupo");
                    if line.state == RunState::Running {
                        g.0 += 1;
                    } else {
                        g.1 += 1;
                    }
                }
                _ => {}
            }
        }
        let counted: Vec<(usize, usize)> = out.iter().map(|g| (g.1, g.2)).collect();
        assert_eq!(counted, seen, "los contadores son las filas que siguen");
        out
    }

    /// Pulsa `/`, teclea `text` y confirma.
    pub(super) fn type_filter(app: &mut App, text: &str) {
        key(app, KeyCode::Char('/'));
        for c in text.chars() {
            key(app, KeyCode::Char(c));
        }
        key(app, KeyCode::Enter);
    }

    fn some(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    fn ended(app: &App, n: usize) -> RunState {
        app.items()
            .into_iter()
            .find_map(|i| match i {
                Item::LiveAgent { n: m, line, .. } if m == n => Some(line.state),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn all_mode_has_no_now_strip_and_starts_with_the_columns() {
        let app = busy();
        let items = app.items();
        assert!(matches!(items[0], Item::Columns));
        assert!(!items.iter().any(|i| matches!(
            i,
            Item::LiveGroup { .. } | Item::Live { .. } | Item::LiveSkill(_) | Item::LiveNow(_)
        )));
        assert!(
            row_named(&app, "backend").running(),
            "su ✦ sigue en la fila"
        );
    }

    #[test]
    fn active_mode_is_a_monitor_without_sections_or_folds() {
        let app = active(history());
        assert!(!app
            .items()
            .iter()
            .any(|i| matches!(i, Item::Section { .. } | Item::Folded { .. } | Item::Row(_))));
        let app = active(fixture());
        let items = app.items();
        assert!(
            matches!(items[0], Item::Columns),
            "una vez, arriba del todo"
        );
        assert_eq!(
            items.iter().filter(|i| matches!(i, Item::Columns)).count(),
            1
        );
        assert_eq!(
            live_groups(&app),
            vec![
                ("~/Offing/taskflow".to_string(), 0, 2),
                ("~/Offing/argi/station".to_string(), 0, 1),
                ("~/Offing/argi/station/front".to_string(), 0, 1),
            ],
            "sin nada en curso, solo terminadas: no hay otra sección"
        );
        // los blancos solo separan: ni al final ni dos seguidos
        assert!(!matches!(items.last(), Some(Item::Blank)));
        assert!(!items
            .windows(2)
            .any(|w| matches!(w, [Item::Blank, Item::Blank])));
        assert_eq!(
            monitor(&app),
            vec![
                (1, "visual".into(), some("visual")),
                (
                    2,
                    "react-structure-checker".into(),
                    some("react-structure-checker")
                ),
                (3, "frontend".into(), some("frontend")),
                (4, "docs".into(), some("docs")),
            ],
            "por grupo; backend (hace 30 min) fuera"
        );
    }

    /// El cursor sigue viendo su bloque entero (la línea de su agente, la
    /// fila y su `now`) al bajar y subir; los blancos, antes de un título de
    /// grupo o entre bloques de agente, no son parte de ninguna fila.
    #[test]
    fn a_running_row_keeps_its_now_line_in_view() {
        let mut app = active(busy());
        // Explore (3): regla de station en 8, su agente en 9, la fila en 10 y
        // su now en 11; con 4 de alto la vista empieza en la regla
        app.cursor = 2;
        assert_eq!(frame(&mut app, 4), 9);
        assert_eq!(app.scroll, 8);
        // backend (5): blanco en 16, agente en 17, fila en 18 y now en 19;
        // la vista acaba en el 19 y el blanco de encima no cuenta
        app.cursor = 4;
        assert_eq!(frame(&mut app, 4), 17);
        assert_eq!(app.scroll, 16);
        // general-purpose (7): regla en 24, agente en 25, fila en 26 y now en
        // 27; la vista acaba en el 27
        app.cursor = 6;
        assert_eq!(frame(&mut app, 4), 25);
        assert_eq!(app.scroll, 24);
        // y de vuelta al primero, con las columnas, su regla y su agente
        app.cursor = 0;
        assert_eq!(frame(&mut app, 4), 2);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn a_group_lists_what_is_running_and_then_what_finished_numbered_straight_through() {
        let app = active(busy());
        let items = app.items();
        assert_eq!(
            live_groups(&app),
            vec![
                ("~/Offing/taskflow".to_string(), 0, 2),
                ("~/Offing/argi/station".to_string(), 3, 1),
                ("~/otro".to_string(), 1, 0),
                ("~/Offing/argi/station/front".to_string(), 0, 1),
            ],
            "grupos por su actividad más reciente; en curso y luego terminadas"
        );
        // Columnas arriba; un blanco antes de cada regla de grupo salvo la
        // primera y otro entre bloques de agente del mismo grupo, nunca al
        // final. Todo bloque lleva encima la línea de su agente; una
        // terminada mide una línea más; una en curso, dos (la fila y su
        // `now`), sin skill.
        let shape: Vec<&str> = items
            .iter()
            .map(|i| match i {
                Item::Columns => "columns",
                Item::LiveGroup { .. } => "group",
                Item::Live { .. } => "row",
                Item::LiveSkill(_) => "skill",
                Item::LiveNow(_) => "now",
                Item::LiveAgent { .. } => "agent",
                Item::Blank => "blank",
                _ => "other",
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                "columns", "group", // taskflow
                "agent", "row", // 1 visual, terminada
                "blank", "agent", "row", // 2 react-structure-checker, terminada
                "blank", "group", // station
                "agent", "row", "now", // 3 Explore
                "blank", "agent", "row", "now", // 4 workflow
                "blank", "agent", "row", "now", // 5 backend
                "blank", "agent", "row", // 6 frontend, terminada
                "blank", "group", "agent", "row", "now", // otro: 7 general-purpose
                "blank", "group", "agent", "row", // station/front: 8 docs, terminada
            ]
        );
        assert_eq!(
            monitor(&app),
            vec![
                (1, "visual".into(), some("visual")),
                (
                    2,
                    "react-structure-checker".into(),
                    some("react-structure-checker")
                ),
                (3, "Explore".into(), None),
                (4, "workflow-subagent: tarea".into(), None),
                (5, "backend".into(), some("backend")),
                (6, "frontend".into(), some("frontend")),
                (7, "general-purpose".into(), None),
                (8, "docs".into(), some("docs")),
            ],
            "en curso por inicio y luego terminadas por fin, dentro de cada grupo"
        );
        assert_eq!(app.count(), 8, "un bloque es una sola fila, termine o no");
        let ended_at = |s: &str| RunState::Done {
            ended: s.parse().unwrap(),
        };
        assert_eq!(ended(&app, 1), ended_at("2026-11-05T11:59:55Z"));
        assert_eq!(ended(&app, 3), RunState::Running);
        assert_eq!(ended(&app, 6), ended_at("2026-11-05T11:58:00Z"));
        assert_eq!(ended(&app, 7), RunState::Running);
        let Item::LiveNow(wf) = &items[15] else {
            panic!("segunda línea de workflow")
        };
        assert_eq!(wf.name, "workflow-subagent: tarea");
        assert_eq!(wf.kind, RunKind::WorkflowStep);
        assert_eq!(wf.worked, Duration::minutes(4), "lleva hasta ahora");
        assert_eq!(wf.skill, None);
        assert_eq!(wf.last_tool, None);
        assert_eq!(wf.since, Some("2026-11-05T11:56:00Z".parse().unwrap()));
        assert_eq!(wf.chain, Chain::Single, "único en curso de su agente");
        let Item::Live { line, .. } = &items[10] else {
            panic!("Explore")
        };
        assert_eq!(line.kind, RunKind::BuiltIn);
    }

    /// Una fila en curso con skill mide tres líneas, la skill justo bajo la
    /// fila y luego `now`; sin skill, dos. Ya no hay línea `cwd`. El encargo va
    /// en la propia fila (`NowLine::task`).
    #[test]
    fn a_block_has_a_skill_line_under_the_name_only_if_the_run_has_a_skill() {
        let mut sk = run("Explore", STATION, "2026-11-05T11:56:00Z", 2, None, false);
        sk.skill = Some("superpowers:brainstorming".into());
        sk.task = Some("pintar la pantalla".into());
        let mut plain = run(
            "Plan",
            "/Users/x/otro",
            "2026-11-05T11:57:00Z",
            2,
            None,
            false,
        );
        plain.task = Some("planear".into());
        let app = active(with_runs(fixture(), vec![sk, plain]));
        let items = app.items();
        let live = |name: &str| {
            items
                .iter()
                .position(|i| matches!(i, Item::Live { line, .. } if line.name == name))
                .unwrap_or_else(|| panic!("sin bloque {name}"))
        };
        let i = live("Explore");
        let Item::LiveSkill(line) = &items[i + 1] else {
            panic!("la skill, justo bajo el nombre")
        };
        assert_eq!(line.skill.as_deref(), Some("superpowers:brainstorming"));
        assert!(matches!(items[i + 2], Item::LiveNow(_)));
        assert!(matches!(items[i - 1], Item::LiveAgent { .. }));
        assert_eq!(App::row_end(&items, i - 1), i + 2, "el bloque mide cuatro");
        let Item::Live { line, .. } = &items[i] else {
            unreachable!()
        };
        assert_eq!(line.task.as_deref(), Some("pintar la pantalla"));
        let j = live("Plan");
        assert!(
            matches!(items[j + 1], Item::LiveNow(_)),
            "sin skill no hay línea"
        );
        assert_eq!(App::row_end(&items, j - 1), j + 1, "el bloque mide tres");
        assert!(!items
            .iter()
            .any(|it| matches!(it, Item::LiveSkill(l) if l.name == "Plan")));
    }

    /// Una terminada mide una línea aunque tenga skill y última herramienta:
    /// sus `skill`/`now` no salen en la lista (siguen en la ficha).
    #[test]
    fn a_finished_row_is_a_single_line_even_with_skill_and_last_tool() {
        let mut app = fixture();
        let mut calls = app.calls.clone();
        calls[0].id = Some("t1".into()); // frontend, 11:58
        let mut done = run(
            "frontend",
            STATION,
            "2026-11-05T11:58:00Z",
            1,
            Some("t1"),
            true,
        );
        done.skill = Some("superpowers:brainstorming".into());
        done.last_tool = Some("Read · docs/interface.md".into());
        app.update(Action::Msg(Msg::Activity {
            calls,
            runs: vec![done],
            sessions: 4,
        }));
        let app = active(app);
        let items = app.items();
        let i = items
            .iter()
            .position(|i| matches!(i, Item::Live { line, .. } if line.name == "frontend"))
            .unwrap();
        let Item::Live { line, .. } = &items[i] else {
            unreachable!()
        };
        assert_eq!(line.skill.as_deref(), Some("superpowers:brainstorming"));
        assert_eq!(App::row_end(&items, i - 1), i, "agente y una sola línea");
        assert!(!items
            .iter()
            .any(|i| matches!(i, Item::LiveSkill(_) | Item::LiveNow(_))));
    }

    #[test]
    fn a_finished_run_in_the_window_joins_its_group_below_the_running_ones() {
        let mut app = fixture();
        let mut calls = app.calls.clone();
        calls[0].id = Some("t1".into()); // frontend, 11:58
        app.update(Action::Msg(Msg::Activity {
            calls,
            runs: vec![
                run(
                    "frontend",
                    STATION,
                    "2026-11-05T11:58:00Z",
                    1,
                    Some("t1"),
                    true,
                ),
                run("Explore", STATION, "2026-11-05T11:57:00Z", 2, None, false),
            ],
            sessions: 4,
        }));
        let app = active(app);
        assert_eq!(
            live_groups(&app)[1],
            ("~/Offing/argi/station".to_string(), 1, 1),
            "la llamada y su ejecución son una sola línea"
        );
        assert_eq!(
            monitor(&app).into_iter().map(|m| m.1).collect::<Vec<_>>(),
            vec![
                "visual",
                "react-structure-checker",
                "Explore",
                "frontend",
                "docs"
            ]
        );
        let line = app
            .items()
            .into_iter()
            .find_map(|i| match i {
                Item::Live { line, .. } if line.name == "frontend" => Some(line),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            line.state,
            RunState::Done {
                ended: "2026-11-05T11:59:00Z".parse().unwrap()
            },
            "termina en su último paso"
        );
        assert_eq!(line.worked, Duration::minutes(1));
    }

    #[test]
    fn the_cursor_and_the_digits_follow_the_grouped_order() {
        let mut app = active(busy());
        key(&mut app, KeyCode::Char('7'));
        let Some(Item::LiveAgent { line, .. }) = app
            .items()
            .into_iter()
            .find(|i| i.n() == Some(app.cursor + 1))
        else {
            panic!("el cursor está en un bloque")
        };
        assert_eq!(line.name, "general-purpose", "la 7 es la del grupo otro");
        key(&mut app, KeyCode::Up);
        assert_eq!(
            app.current().unwrap().agent,
            row_named(&app, "frontend").agent,
            "la terminada de station, antes que la de otro"
        );
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Down);
        assert_eq!(
            app.current().unwrap().agent,
            row_named(&app, "docs").agent,
            "pasa de grupo sin saltarse nada"
        );
        key(&mut app, KeyCode::Down);
        assert_eq!(app.cursor, 7, "no pasa de la última");
        key(&mut app, KeyCode::Home);
        assert_eq!(app.cursor, 0);
        key(&mut app, KeyCode::End);
        assert_eq!(app.cursor, 7);
    }

    #[test]
    fn the_header_counts_runs_in_progress_and_finished_in_the_window() {
        let app = busy();
        assert_eq!(app.live_count(), 4, "Σ running");
        assert_eq!(app.recent_count(), 4, "Σ recent: las terminadas");
        let app = fixture();
        assert_eq!(app.live_count(), 0);
        assert_eq!(app.recent_count(), 4);
        let mut filtered = active(busy());
        type_filter(&mut filtered, "explore");
        assert_eq!(
            (filtered.live_count(), filtered.recent_count()),
            (4, 4),
            "la cabecera cuenta sin filtro"
        );
    }

    #[test]
    fn enter_twice_opens_the_first_run_of_every_block_running_or_finished() {
        let mut app = active(busy());
        for (n, name, kind, owner, done) in [
            (1u32, "visual", RunKind::Defined, true, true),
            (2, "react-structure-checker", RunKind::Defined, true, true),
            (3, "Explore", RunKind::BuiltIn, false, false),
            (
                4,
                "workflow-subagent: tarea",
                RunKind::WorkflowStep,
                false,
                false,
            ),
            (5, "backend", RunKind::Defined, true, false),
            (6, "frontend", RunKind::Defined, true, true),
            (7, "general-purpose", RunKind::BuiltIn, false, false),
            (8, "docs", RunKind::Defined, true, true),
        ] {
            key(&mut app, KeyCode::Char(char::from_digit(n, 10).unwrap()));
            assert!(app.can_open(), "{name}");
            key(&mut app, KeyCode::Enter);
            assert!(app.run_detail.is_none(), "primero, la pantalla del agente");
            assert_eq!(app.agent_view().expect(name).name, name);
            key(&mut app, KeyCode::Enter);
            let d = app.run_detail.as_ref().expect(name);
            assert_eq!(
                (
                    d.name.as_str(),
                    d.kind,
                    d.owner.is_some(),
                    d.ended.is_some()
                ),
                (name, kind, owner, done),
                "{name}"
            );
            assert!(!app.detail, "es la de la ejecución, no la ficha del agente");
            key(&mut app, KeyCode::Esc);
            assert!(app.run_detail.is_none() && app.agent_view().is_some());
            key(&mut app, KeyCode::Esc);
            assert!(app.agent_view().is_none());
            assert_eq!(
                app.cursor,
                n as usize - 1,
                "Esc deja el cursor en su bloque"
            );
        }
    }

    #[test]
    fn enter_on_a_call_without_a_run_opens_a_minimal_detail_with_ended() {
        let mut app = active(fixture());
        key(&mut app, KeyCode::Char('3'));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        let d = app.run_detail.as_ref().expect("la llamada abre su ficha");
        assert_eq!(d.name, "frontend");
        assert_eq!(
            d.ended,
            Some("2026-11-05T11:58:00Z".parse::<DateTime<Utc>>().unwrap())
        );
        assert!(d.steps.is_empty() && d.tool_counts.is_empty());
        assert_eq!((d.tools, d.worked), (0, TimeDelta::zero()));
        assert!(!app.detail);
        // los refrescos no la cierran ni la cambian
        let before = app.run_detail.clone();
        app.update(Action::Tick("2026-11-05T12:00:30Z".parse().unwrap()));
        assert_eq!(app.run_detail, before);
        // y se puede abrir la ficha del agente desde su pantalla
        key(&mut app, KeyCode::Esc);
        assert!(app.can_open_agent());
        key(&mut app, KeyCode::Char('a'));
        assert!(app.run_detail.is_none() && app.detail);
        assert_eq!(app.agent(app.current().unwrap().agent).name, "frontend");
    }

    #[test]
    fn enter_opens_the_run_of_a_workflow_whose_real_agent_is_a_defined_one() {
        let mut wf = run(
            "workflow-subagent",
            STATION,
            "2026-11-05T11:57:00Z",
            2,
            None,
            false,
        );
        wf.description = Some("backend".into());
        wf.task = Some("revisar el cruce".into());
        let mut app = active(with_runs(fixture(), vec![wf]));
        let items = app.items();
        // taskflow (2 terminadas, dos bloques con su agente y un blanco entre
        // ellos) va antes; el workflow abre station: la 3, bajo la línea de su
        // agente
        assert!(matches!(items[10], Item::Live { .. }));
        let Item::LiveAgent { n, line, row } = &items[9] else {
            panic!("el workflow abre el grupo de station")
        };
        assert_eq!(*n, 3);
        assert_eq!(line.kind, RunKind::Workflow);
        assert!(row.is_some(), "viene de owner");
        key(&mut app, KeyCode::Char('3'));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        let d = app.run_detail.as_ref().unwrap();
        assert_eq!(d.kind, RunKind::Workflow);
        assert_eq!(d.task.as_deref(), Some("revisar el cruce"));
        key(&mut app, KeyCode::Esc);
        assert!(app.can_open_agent());
    }

    #[test]
    fn a_finished_run_stays_open_with_its_last_photo_and_ended() {
        let mut app = active(busy());
        key(&mut app, KeyCode::Char('5'));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        let before = app.run_detail.clone().unwrap();
        assert_eq!(before.ended, None);
        // la ejecución termina
        let mut runs = app.runs.clone();
        for r in runs.iter_mut().filter(|r| r.agent_type == "backend") {
            r.finished = Some(true);
        }
        app = with_runs(app, runs);
        let d = app.run_detail.as_ref().expect("no se cierra");
        assert!(d.ended.is_some());
        assert_eq!(d.name, "backend");
        // desaparece del disco: conserva la foto
        let without: Vec<Run> = app
            .runs
            .iter()
            .filter(|r| r.agent_type != "backend")
            .cloned()
            .collect();
        let was = app.run_detail.clone();
        app = with_runs(app, without);
        assert_eq!(app.run_detail, was);
        // su bloque ya no está en activos: Esc vuelve a la lista
        key(&mut app, KeyCode::Esc);
        assert!(app.run_detail.is_none() && app.agent_view().is_none());
    }

    #[test]
    fn steps_scroll_is_bounded() {
        let mut r = run("Explore", STATION, "2026-11-05T11:57:00Z", 2, None, false);
        r.steps = (0..5)
            .map(|i| crate::runs::ToolStep {
                at: "2026-11-05T11:58:00Z".parse().unwrap(),
                name: "Bash".into(),
                arg: Some(format!("cmd {i}")),
            })
            .collect::<Vec<_>>()
            .into();
        let mut app = active(with_runs(fixture(), vec![r]));
        key(&mut app, KeyCode::Char('3'));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.run_detail.as_ref().unwrap().name, "Explore");
        key(&mut app, KeyCode::PageUp);
        assert_eq!(app.steps_scroll, 0, "no pasa de 0");
        assert_eq!(app.steps_window(2), 0);
        key(&mut app, KeyCode::PageDown);
        assert_eq!(app.steps_scroll, 2, "una página de 2");
        key(&mut app, KeyCode::PageDown);
        key(&mut app, KeyCode::PageDown);
        assert_eq!(app.steps_scroll, 3, "no pasa de la última página llena");
        key(&mut app, KeyCode::Home);
        assert_eq!(app.steps_scroll, 0);
        key(&mut app, KeyCode::End);
        assert_eq!(app.steps_scroll, 3);
        assert_eq!(app.steps_window(10), 0, "la vista acota a lo que cabe");
    }

    #[test]
    fn esc_returns_with_the_cursor_on_the_same_block_and_the_filter_survives() {
        let mut app = active(busy());
        type_filter(&mut app, "station");
        // station: 1 Explore, 2 workflow, 3 backend, 4 frontend; station/front: 5 docs
        key(&mut app, KeyCode::Char('2'));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Right);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.run_detail.as_ref().unwrap().name, "backend");
        key(&mut app, KeyCode::Char('/'));
        assert!(
            !app.filtering,
            "la pantalla de la ejecución no abre el filtro"
        );
        assert_eq!(app.filter, "station");
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('/'));
        assert!(!app.filtering, "ni la del agente");
        key(&mut app, KeyCode::Esc);
        assert!(app.run_detail.is_none() && app.agent_view().is_none());
        assert_eq!(app.cursor, 2);
        let Some(Item::LiveAgent { line, .. }) = app
            .items()
            .into_iter()
            .find(|i| i.n() == Some(app.cursor + 1))
        else {
            panic!("el cursor está en un bloque")
        };
        assert_eq!(line.name, "backend");
    }

    #[test]
    fn the_filter_in_active_matches_the_task_not_the_workflow_name() {
        let mut r = run("Explore", STATION, "2026-11-05T11:56:00Z", 2, None, false);
        r.task = Some("pintar la pantalla".into());
        let mut app = active(with_runs(fixture(), vec![r]));
        type_filter(&mut app, "pintar");
        assert_eq!(monitor(&app).len(), 1);
        assert_eq!(
            live_groups(&app),
            vec![("~/Offing/argi/station".to_string(), 1, 0)]
        );
    }

    #[test]
    fn the_filter_in_active_matches_name_project_or_task() {
        let filtered = |f: &str| -> Vec<String> {
            let mut app = active(busy());
            type_filter(&mut app, f);
            monitor(&app).into_iter().map(|m| m.1).collect()
        };
        assert_eq!(filtered("explore"), vec!["Explore"], "nombre");
        assert_eq!(
            filtered("otro"),
            vec!["general-purpose"],
            "ruta de la en curso"
        );
        assert_eq!(
            filtered("taskflow"),
            vec!["visual", "react-structure-checker"],
            "proyecto de las terminadas"
        );
        assert_eq!(
            filtered("tarea"),
            vec![
                "Explore",
                "workflow-subagent: tarea",
                "backend",
                "general-purpose"
            ],
            "encargo (y el nombre del workflow); agrupado, por proyecto"
        );
        let mut app = active(busy());
        type_filter(&mut app, "zzz");
        assert!(
            app.items().is_empty(),
            "sin coincidencias no hay estado vacío"
        );
    }

    /// Una sesión lanzada en `~/lanzadera` cuyo subagente trabaja en
    /// `~/obrador`: el grupo es la lanzadera y la línea cwd, el obrador.
    fn moved_run() -> App {
        let mut r = run(
            "Explore",
            "/Users/x/obrador",
            "2026-11-05T11:56:00Z",
            2,
            None,
            false,
        );
        r.launch_dir = Some(PathBuf::from("/Users/x/lanzadera"));
        active(with_runs(fixture(), vec![r]))
    }

    /// El `cwd` real ya no tiene línea en la lista (sigue en la ficha): la
    /// fila en curso acaba en su `now` y el grupo va por la lanzadera.
    #[test]
    fn a_row_has_no_cwd_line_and_groups_by_the_launch_dir() {
        let app = moved_run();
        let items = app.items();
        assert!(
            live_groups(&app).contains(&("~/lanzadera".to_string(), 1, 0)),
            "agrupa por donde se lanzó la sesión"
        );
        let rule = items
            .iter()
            .position(|i| matches!(i, Item::LiveGroup { section, .. } if section == "~/lanzadera"))
            .unwrap();
        assert!(matches!(items[rule + 1], Item::LiveAgent { .. }));
        assert!(matches!(items[rule + 2], Item::Live { .. }));
        assert!(matches!(items[rule + 3], Item::LiveNow(_)));
        assert_eq!(App::row_top(&items, rule + 1), rule);
        assert_eq!(App::row_end(&items, rule + 1), rule + 3);
    }

    #[test]
    fn the_filter_in_active_matches_the_group_dir_and_the_real_cwd() {
        let filtered = |f: &str| {
            let mut app = moved_run();
            type_filter(&mut app, f);
            (live_groups(&app), monitor(&app).len())
        };
        let lanzadera = vec![("~/lanzadera".to_string(), 1, 0)];
        assert_eq!(
            filtered("lanzadera"),
            (lanzadera.clone(), 1),
            "regla del grupo"
        );
        assert_eq!(filtered("obrador"), (lanzadera, 1), "cwd real");
    }

    #[test]
    fn a_filter_that_leaves_only_finished_runs_rebuilds_the_group_counters() {
        let mut app = active(busy());
        type_filter(&mut app, "station");
        assert_eq!(
            live_groups(&app),
            vec![
                ("~/Offing/argi/station".to_string(), 3, 1),
                ("~/Offing/argi/station/front".to_string(), 0, 1),
            ],
            "taskflow y otro se van; station conserva sus 3 + 1"
        );
        let mut app = active(busy());
        type_filter(&mut app, "frontend");
        assert_eq!(
            live_groups(&app),
            vec![("~/Offing/argi/station".to_string(), 0, 1)],
            "solo queda una terminada: sin en curso en el grupo"
        );
        assert_eq!(
            monitor(&app),
            vec![(1, "frontend".into(), some("frontend"))],
            "la numeración se rehace sobre lo que deja el filtro"
        );
        assert!(matches!(app.items()[0], Item::Columns));
        assert_eq!(app.count(), 1);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.run_detail.as_ref().unwrap().name, "frontend");
    }

    #[test]
    fn an_empty_monitor_says_the_last_use_of_any_agent() {
        let mut app = active(fixture());
        app.update(Action::Tick("2026-11-05T13:00:00Z".parse().unwrap()));
        let items = app.items();
        assert_eq!(items.len(), 1);
        let Item::Quiet { last: Some(last) } = &items[0] else {
            panic!("estado vacío con último uso")
        };
        assert_eq!(app.agent(last.agent).name, "visual");
        assert_eq!(last.section, "~/Offing/taskflow");
        assert_eq!(
            last.at,
            "2026-11-05T11:59:55Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(app.count(), 0);
        assert!(!app.can_open());
        key(&mut app, KeyCode::Enter);
        assert!(!app.detail && app.run_detail.is_none());

        let mut app = active(fixture());
        app.update(Action::Msg(Msg::Activity {
            calls: vec![],
            runs: vec![],
            sessions: 0,
        }));
        assert!(matches!(app.items()[..], [Item::Quiet { last: None }]));
    }

    #[test]
    fn a_block_scrolls_into_view_with_all_its_lines_running_or_finished() {
        let mut app = active(busy());
        key(&mut app, KeyCode::Char('7'));
        // general-purpose en la 26, su agente en la 25 y su now en la 27; su
        // regla en la 24
        assert_eq!(frame(&mut app, 4), 25);
        assert_eq!(
            app.scroll, 24,
            "se ven la regla y las tres líneas de la fila"
        );
        key(&mut app, KeyCode::Char('1'));
        frame(&mut app, 3);
        assert_eq!(app.scroll, 0, "el primero arrastra la regla y las columnas");
        // una terminada (la 6, en la 22, con su agente en la 21) mide una
        // línea más la de su agente
        key(&mut app, KeyCode::Char('6'));
        frame(&mut app, 3);
        assert_eq!(app.scroll, 20, "la 6 en la última línea de la vista");
        // la última (la 8, en la 31): con la regla de su grupo (29) y su
        // agente (30) a la vista
        key(&mut app, KeyCode::Char('8'));
        assert_eq!(frame(&mut app, 3), 30);
        assert_eq!(app.scroll, 29);
        key(&mut app, KeyCode::Home);
        frame(&mut app, 5);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn o_does_nothing_in_active() {
        let mut app = active(fixture());
        key(&mut app, KeyCode::Char('o'));
        assert_eq!(app.order, Order::Recent);
    }

    #[test]
    fn o_orders_inside_groups_by_worked_and_back() {
        // station por recencia: frontend (11:58), docs (11:52), backend (11:30)
        let mut app = with_runs(
            fixture(),
            vec![
                run("backend", STATION, "2026-11-01T09:00:00Z", 40, None, true),
                run("docs", STATION, "2026-11-02T09:00:00Z", 20, None, true),
                run("frontend", STATION, "2026-11-03T09:00:00Z", 5, None, true),
            ],
        );
        let station = |app: &App| -> Vec<String> {
            app.rows()
                .iter()
                .filter(|r| r.section == "~/Offing/argi/station")
                .map(|r| app.agent(r.agent).name.clone())
                .collect()
        };
        let titles =
            |app: &App| -> Vec<String> { sections(app).into_iter().map(|s| s.0).collect() };
        assert_eq!(app.order, Order::Recent);
        assert_eq!(station(&app), vec!["frontend", "docs", "backend"]);
        let before = titles(&app);
        key(&mut app, KeyCode::Char('o'));
        assert_eq!(app.order, Order::Worked);
        assert_eq!(station(&app), vec!["backend", "docs", "frontend"]);
        assert_eq!(titles(&app), before, "los grupos no cambian de orden");
        let ns: Vec<usize> = app.rows().iter().map(|r| r.n).collect();
        assert_eq!(ns, vec![1, 2, 3, 4, 5]);
        key(&mut app, KeyCode::Char('o'));
        assert_eq!(app.order, Order::Recent);
        assert_eq!(station(&app), vec!["frontend", "docs", "backend"]);
    }

    fn ranked(app: &App) -> Vec<(String, i64, usize)> {
        app.items()
            .iter()
            .filter_map(|i| match i {
                Item::Rank { row, work } => Some((
                    app.agent(row.agent).name.clone(),
                    work.worked.num_minutes(),
                    work.runs,
                )),
                _ => None,
            })
            .collect()
    }

    /// En el ranking. docs 72 min en dos (una de hace 40 días), visual 30 de
    /// hace 10 días y frontend 5 de ayer.
    fn ranking() -> App {
        let mut app = with_runs(
            fixture(),
            vec![
                run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true),
                run("docs", STATION, "2026-09-26T09:00:00Z", 22, None, true),
                run("frontend", STATION, "2026-11-04T10:00:00Z", 5, None, true),
                run(
                    "visual",
                    "/Users/x/Offing/taskflow",
                    "2026-10-26T10:00:00Z",
                    30,
                    None,
                    true,
                ),
            ],
        );
        key(&mut app, KeyCode::Tab);
        app
    }

    fn s(v: &[(&str, i64, usize)]) -> Vec<(String, i64, usize)> {
        v.iter().map(|(n, w, r)| (n.to_string(), *w, *r)).collect()
    }

    #[test]
    fn ranking_lists_agents_with_work_by_worked_desc() {
        let app = ranking();
        assert_eq!(app.mode, Mode::Ranking);
        assert_eq!(app.period, Period::Ever);
        let items = app.items();
        assert!(matches!(items[0], Item::RankRule { agents: 3 }));
        assert_eq!(
            ranked(&app),
            s(&[("docs", 72, 2), ("visual", 30, 1), ("frontend", 5, 1)])
        );
        let ns: Vec<usize> = app.rows().iter().map(|r| r.n).collect();
        assert_eq!(ns, vec![1, 2, 3]);
        assert_eq!(
            app.ranking_total(),
            PeriodStats {
                worked: Duration::minutes(107),
                runs: 4,
                tokens: 0,
            }
        );
        assert!(!items
            .iter()
            .any(|i| matches!(i, Item::Columns | Item::Section { .. } | Item::Row(_))));
    }

    #[test]
    fn p_cycles_the_period_of_the_ranking() {
        let mut app = ranking();
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.period, Period::Week);
        assert_eq!(ranked(&app), s(&[("docs", 50, 1), ("frontend", 5, 1)]));
        assert_eq!(
            app.ranking_total(),
            PeriodStats {
                worked: Duration::minutes(55),
                runs: 2,
                tokens: 0,
            }
        );
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.period, Period::Month);
        assert_eq!(
            ranked(&app),
            s(&[("docs", 50, 1), ("visual", 30, 1), ("frontend", 5, 1)])
        );
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.period, Period::Ever);
    }

    #[test]
    fn p_and_o_only_act_where_they_mean_something() {
        let mut app = fixture();
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(app.period, Period::Ever, "p solo en el ranking");
        let mut app = ranking();
        key(&mut app, KeyCode::Char('o'));
        assert_eq!(app.order, Order::Recent, "o solo en las listas");
    }

    #[test]
    fn enter_in_the_ranking_opens_the_agent_and_arrows_follow_the_ranking() {
        let mut app = ranking();
        key(&mut app, KeyCode::Char('2'));
        key(&mut app, KeyCode::Enter);
        assert!(app.detail);
        assert_eq!(app.agent(app.current().unwrap().agent).name, "visual");
        key(&mut app, KeyCode::Down);
        assert_eq!(app.agent(app.current().unwrap().agent).name, "frontend");
        key(&mut app, KeyCode::Down);
        assert_eq!(app.agent(app.current().unwrap().agent).name, "frontend");
        key(&mut app, KeyCode::Up);
        key(&mut app, KeyCode::Up);
        assert_eq!(app.agent(app.current().unwrap().agent).name, "docs");
        key(&mut app, KeyCode::Esc);
        assert!(!app.detail);
        assert_eq!(app.mode, Mode::Ranking);
    }

    #[test]
    fn the_filter_also_works_in_the_ranking() {
        let mut app = ranking();
        key(&mut app, KeyCode::Char('/'));
        for c in "fro".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Enter);
        assert_eq!(ranked(&app), s(&[("frontend", 5, 1)]));
        assert_eq!(app.count(), 1);
    }

    #[test]
    fn the_ranking_scrolls_like_the_list() {
        let mut app = many(30);
        let runs = (0..30)
            .map(|i| {
                run(
                    &format!("a{i:02}"),
                    "/Users/x/p",
                    "2026-11-04T09:00:00Z",
                    60 - i,
                    None,
                    true,
                )
            })
            .collect();
        let mut app = {
            app.tz = Some(utc());
            with_runs(app, runs)
        };
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.count(), 30);
        frame(&mut app, 10);
        key(&mut app, KeyCode::End);
        assert_eq!(app.cursor, 29);
        assert_eq!(frame(&mut app, 10), 30, "la regla en la línea 0");
        assert_eq!(app.scroll, 21);
        key(&mut app, KeyCode::PageUp);
        assert_eq!(app.cursor, 19);
        key(&mut app, KeyCode::Home);
        frame(&mut app, 10);
        assert_eq!(app.scroll, 0, "con la primera fila se ve la regla");
    }

    #[test]
    fn rows_carry_ninety_local_days_oldest_first() {
        let app = with_runs(
            fixture(),
            vec![
                run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true),
                run("docs", STATION, "2026-08-08T09:00:00Z", 22, None, true),
                run("docs", STATION, "2026-08-07T09:00:00Z", 99, None, true),
            ],
        );
        let days = row_named(&app, "docs").history;
        assert_eq!(days.len(), MAX_DAYS);
        assert_eq!(days[89], TimeDelta::zero(), "hoy");
        assert_eq!(days[88], Duration::minutes(50), "ayer");
        assert_eq!(days[0], Duration::minutes(22), "hoy - 89");
        assert_eq!(
            days.iter().sum::<TimeDelta>(),
            Duration::minutes(72),
            "hoy - 90 queda fuera"
        );
    }

    #[test]
    fn a_row_gives_its_last_n_days_with_today_last() {
        let app = with_runs(
            fixture(),
            vec![
                run("docs", STATION, "2026-11-04T09:00:00Z", 50, None, true),
                run("docs", STATION, "2026-10-30T09:00:00Z", 30, None, true),
            ],
        );
        let row = row_named(&app, "docs");
        let week = row.days(7);
        assert_eq!(week.len(), 7);
        assert_eq!(week[6], TimeDelta::zero(), "hoy");
        assert_eq!(week[5], Duration::minutes(50), "ayer");
        assert_eq!(week[0], Duration::minutes(30), "hoy - 6");
        assert_eq!(row.days(1), [TimeDelta::zero()]);
        assert_eq!(row.days(0).len(), 0);
        assert_eq!(row.days(500).len(), MAX_DAYS, "más de 90 se queda en 90");
        assert_eq!(row.days(MAX_DAYS), row.history.as_slice());
    }

    #[test]
    fn the_days_follow_the_zone_of_the_screen() {
        let mut app = with_runs(
            fixture(),
            vec![run("docs", STATION, "2026-11-04T23:30:00Z", 20, None, true)],
        );
        assert_eq!(row_named(&app, "docs").days(2)[0], Duration::minutes(20));
        app.tz = Some(FixedOffset::east_opt(3600).unwrap());
        assert_eq!(row_named(&app, "docs").days(1)[0], Duration::minutes(20));
    }

    #[test]
    fn the_detail_lists_every_run_of_the_agent_with_its_local_start() {
        let front = format!("{STATION}/front");
        let mut app = with_runs(
            fixture(),
            vec![
                run("docs", STATION, "2026-11-04T17:12:00Z", 6, None, true),
                run("docs", &front, "2026-11-05T11:55:00Z", 4, None, false),
                run(
                    "workflow-subagent",
                    STATION,
                    "2026-11-05T10:00:00Z",
                    9,
                    None,
                    true,
                ),
            ],
        );
        app.tz = Some(FixedOffset::east_opt(3600).unwrap());
        let docs = row_named(&app, "docs").agent;
        let runs = app.runs_of(docs);
        assert_eq!(runs.len(), 2, "la de workflow no es de docs");
        let at = |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        assert_eq!(runs[0].start, Some(at("2026-11-05 12:55")));
        assert!(runs[0].in_progress);
        assert_eq!(runs[0].worked, Duration::minutes(5));
        assert_eq!(runs[0].cwd, Some(Path::new(&front)));
        assert_eq!(runs[0].tools, 7);
        assert_eq!(runs[0].description, Some("docs: tarea"));
        assert_eq!(runs[1].start, Some(at("2026-11-04 18:12")));
        assert!(!runs[1].in_progress);
        assert_eq!(runs[1].worked, Duration::minutes(6));
        assert_eq!(runs[1].cwd, None, "en la raíz del proyecto no se repite");
    }

    #[test]
    fn the_detail_counts_old_calls_without_a_run_and_the_mean() {
        let mut app = fixture();
        let mut linked = call("docs", STATION, "2026-11-05T09:00:00Z");
        linked.id = Some("toolu_1".into());
        app.update(Action::Msg(Msg::Activity {
            calls: vec![
                linked,
                call("docs", STATION, "2026-10-01T09:00:00Z"),
                call("docs", STATION, "2026-10-02T09:00:00Z"),
            ],
            runs: vec![
                run(
                    "docs",
                    STATION,
                    "2026-11-05T09:00:00Z",
                    12,
                    Some("toolu_1"),
                    true,
                ),
                run("docs", STATION, "2026-11-03T09:00:00Z", 30, None, true),
            ],
            sessions: 1,
        }));
        let docs = row_named(&app, "docs");
        assert_eq!(app.old_calls(docs.agent), 2);
        assert_eq!(docs.mean(), Some(Duration::minutes(21)));
        assert_eq!(row_named(&app, "frontend").mean(), None);
    }

    /// Station con `backend` ejecutado cuatro veces y `frontend` una:
    /// `toolu_b1` y `toolu_b2` terminadas (se funden en una fila, b2 la
    /// representa), `toolu_b3` (11:55) y `toolu_b4` (11:50) en curso, en
    /// racimo, una fila cada una, y la terminada de backend lo cierra.
    /// Bloques: 1 backend ×2 (sus dos en curso), con las filas b3 (cabeza), b4 (en medio) y su
    /// terminada ×2 (terminó 11:35, última); 2 frontend (terminó 11:45, en
    /// su propio bloque).
    pub(super) fn fused() -> App {
        let mut app = App::new(60, PathBuf::from("/Users/x"), "0.1.0", NOW.parse().unwrap());
        app.tz = Some(utc());
        app.update(Action::Msg(Msg::Agents(vec![
            agent("backend", Some(STATION)),
            agent("frontend", Some(STATION)),
        ])));
        let mut b3 = run(
            "backend",
            STATION,
            "2026-11-05T11:55:00Z",
            4,
            Some("toolu_b3"),
            false,
        );
        b3.task = Some("revisar el cruce".into());
        let mut b4 = run(
            "backend",
            STATION,
            "2026-11-05T11:50:00Z",
            9,
            Some("toolu_b4"),
            false,
        );
        b4.task = Some("revisar los tests".into());
        let app = with_runs(
            app,
            vec![
                run(
                    "backend",
                    STATION,
                    "2026-11-05T11:10:00Z",
                    5,
                    Some("toolu_b1"),
                    true,
                ),
                run(
                    "backend",
                    STATION,
                    "2026-11-05T11:30:00Z",
                    5,
                    Some("toolu_b2"),
                    true,
                ),
                b3,
                b4,
                run(
                    "frontend",
                    STATION,
                    "2026-11-05T11:40:00Z",
                    5,
                    Some("toolu_f1"),
                    true,
                ),
            ],
        );
        assert_eq!(app.mode, Mode::Active, "una App nueva abre en activos");
        app
    }

    pub(super) fn open_id(app: &App) -> String {
        app.run_detail.as_ref().unwrap().id.clone()
    }

    /// (número de su bloque, nombre, ejecuciones, id, racimo, encargo) de
    /// cada fila de ejecución.
    fn chained(app: &App) -> Vec<(usize, String, usize, String, Chain, Option<String>)> {
        let mut block = 0;
        app.items()
            .into_iter()
            .filter_map(|i| match i {
                Item::LiveAgent { n, .. } => {
                    block = n;
                    None
                }
                Item::Live { line } => {
                    Some((block, line.name, line.runs, line.id, line.chain, line.task))
                }
                _ => None,
            })
            .collect()
    }

    /// Un bloque por agente: sus en curso (una fila por ejecución) y detrás
    /// su terminada (una por agente, que funde las suyas), cerrando el racimo.
    /// El número es del bloque: las tres filas de backend llevan el 1.
    #[test]
    fn a_row_per_running_run_in_a_chain_closed_by_the_finished_row_of_its_agent() {
        let app = fused();
        assert_eq!(
            chained(&app),
            vec![
                (
                    1,
                    "backend".into(),
                    1,
                    "toolu_b3".into(),
                    Chain::Head(3),
                    some("revisar el cruce")
                ),
                (
                    1,
                    "backend".into(),
                    1,
                    "toolu_b4".into(),
                    Chain::Middle,
                    some("revisar los tests")
                ),
                (
                    1,
                    "backend".into(),
                    2,
                    "toolu_b2".into(),
                    Chain::Last,
                    some("backend: tarea")
                ),
                (
                    2,
                    "frontend".into(),
                    1,
                    "toolu_f1".into(),
                    Chain::Single,
                    some("frontend: tarea")
                ),
            ]
        );
        assert_eq!(app.count(), 2, "se selecciona por bloque, no por fila");
        assert_eq!((app.live_count(), app.recent_count()), (2, 2));
        assert_eq!(live_groups(&app), vec![(STATION_SECTION.into(), 2, 2)]);
    }

    /// El filtro deja filas sueltas de un racimo (en curso o su terminada):
    /// lo que queda se vuelve a encadenar (una sola, `Single`) para que no se
    /// pinte un `└─` huérfano.
    #[test]
    fn the_filter_works_on_rows_and_rechains_what_is_left() {
        let mut app = fused();
        type_filter(&mut app, "front");
        assert_eq!(live_groups(&app), vec![(STATION_SECTION.into(), 0, 1)]);
        assert_eq!(app.count(), 1);
        let mut app = fused();
        type_filter(&mut app, "tests"); // encargo de b4, en medio del racimo
        assert_eq!(live_groups(&app), vec![(STATION_SECTION.into(), 1, 0)]);
        let rows = chained(&app);
        assert_eq!((rows[0].3.as_str(), rows[0].4), ("toolu_b4", Chain::Single));
        let mut app = fused();
        type_filter(&mut app, "revisar"); // b3 y b4: el racimo sin su terminada
        let rows = chained(&app);
        assert_eq!(
            rows.iter().map(|r| r.4).collect::<Vec<_>>(),
            vec![Chain::Head(2), Chain::Last]
        );
        // la terminada es un miembro más del racimo y lo cierra
        let mut app = fused();
        type_filter(&mut app, "backend");
        assert_eq!(
            chained(&app)
                .into_iter()
                .map(|r| (r.3, r.4))
                .collect::<Vec<_>>(),
            vec![
                ("toolu_b3".to_string(), Chain::Head(3)),
                ("toolu_b4".to_string(), Chain::Middle),
                ("toolu_b2".to_string(), Chain::Last),
            ]
        );
        // solo las terminadas: cada una, sola en su bloque
        let mut app = fused();
        type_filter(&mut app, ": tarea");
        assert_eq!(
            chained(&app)
                .into_iter()
                .map(|r| (r.3, r.4))
                .collect::<Vec<_>>(),
            vec![
                ("toolu_b2".to_string(), Chain::Single),
                ("toolu_f1".to_string(), Chain::Single),
            ]
        );
    }

    /// Un blanco entre bloques de agente del mismo grupo, que no se
    /// selecciona ni rompe la numeración; ni al principio del grupo ni al
    /// final.
    #[test]
    fn a_blank_line_separates_the_agent_blocks_of_a_group() {
        let app = fused();
        let shape: Vec<String> = app
            .items()
            .iter()
            .map(|i| match i {
                Item::Columns => "columns".into(),
                Item::LiveGroup { .. } => "group".into(),
                Item::Live { .. } => "row".into(),
                Item::LiveSkill(_) => "skill".into(),
                Item::LiveNow(_) => "now".into(),
                Item::LiveAgent { n, line, .. } => format!("agent {n} {}", line.name),
                Item::Blank => "blank".into(),
                _ => "other".into(),
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                "columns",
                "group",
                "agent 1 backend",
                "row",
                "now",
                "row",
                "now",
                "row",
                "blank",
                "agent 2 frontend",
                "row",
            ]
        );
        assert_eq!(app.count(), 2);
    }

    /// La línea del agente lleva en `runs` solo las ejecuciones en curso de
    /// su bloque (las que deja el filtro), sin las que funde la terminada: de
    /// ahí sale `×N`. Un bloque solo con terminada lleva 0.
    #[test]
    fn the_agent_line_counts_only_the_running_runs_of_its_block() {
        let app = fused();
        let agents: Vec<(String, usize)> = app
            .items()
            .into_iter()
            .filter_map(|i| match i {
                Item::LiveAgent { line, .. } => Some((line.name, line.runs)),
                _ => None,
            })
            .collect();
        assert_eq!(
            agents,
            vec![("backend".to_string(), 2), ("frontend".to_string(), 0)]
        );
    }

    /// `backend` con `running` ejecuciones en curso (las dos primeras con
    /// encargo «revisar …», el resto «pintar …») y `done` terminadas, que se
    /// funden en una sola fila. En activos.
    pub(crate) fn counted(running: usize, done: usize) -> App {
        let mut runs = Vec::new();
        for i in 0..done {
            let start = format!("2026-11-05T11:{:02}:00Z", 55 + i);
            let id = format!("toolu_d{i}");
            runs.push(run("backend", STATION, &start, 1, Some(&id), true));
        }
        for i in 0..running {
            let start = format!("2026-11-05T11:{:02}:00Z", 56 + i);
            let id = format!("toolu_r{i}");
            let mut r = run("backend", STATION, &start, 1, Some(&id), false);
            let verb = if i < 2 { "revisar" } else { "pintar" };
            r.task = Some(format!("{verb} la parte {i}"));
            runs.push(r);
        }
        let mut app = with_runs(fixture(), runs);
        app.mode = Mode::Active;
        app
    }

    /// `runs` de la línea del agente de `backend` y lo que funde su
    /// terminada, si la hay.
    fn agent_and_done(app: &App) -> (usize, Option<usize>) {
        let items = app.items();
        let agent = items.iter().find_map(|i| match i {
            Item::LiveAgent { line, .. } if line.name == "backend" => Some(line.runs),
            _ => None,
        });
        let done = items.iter().find_map(|i| match i {
            Item::Live { line } if line.name == "backend" && line.state != RunState::Running => {
                Some(line.runs)
            }
            _ => None,
        });
        (agent.expect("backend"), done)
    }

    /// 4 en curso y una terminada que funde 3: `×4`, no `×7`; la terminada
    /// sigue fundiendo sus 3 (la vista no las cuenta en la línea).
    #[test]
    fn four_running_and_a_finished_row_of_three_count_four() {
        assert_eq!(agent_and_done(&counted(4, 3)), (4, Some(3)));
    }

    /// Una sola en curso con terminadas: 1, sin `×`.
    #[test]
    fn one_running_with_finished_rows_counts_one() {
        assert_eq!(agent_and_done(&counted(1, 3)).0, 1);
    }

    /// Solo terminadas: 0, sin `×`, aunque la fila funda varias.
    #[test]
    fn only_finished_rows_count_none() {
        assert_eq!(agent_and_done(&counted(0, 3)), (0, Some(3)));
    }

    /// Con filtro: solo las en curso que quedan (2 de 4 dicen «revisar»; la
    /// terminada no pasa el filtro).
    #[test]
    fn the_filter_leaves_two_of_four_running() {
        let mut app = counted(4, 3);
        type_filter(&mut app, "revisar");
        assert_eq!(agent_and_done(&app), (2, None));
        let mut app = fused();
        type_filter(&mut app, "revisar");
        let Some(Item::LiveAgent { line, .. }) = app
            .items()
            .into_iter()
            .find(|i| matches!(i, Item::LiveAgent { .. }))
        else {
            panic!("backend")
        };
        assert_eq!(line.runs, 2, "solo lo que deja el filtro");
    }

    /// La línea del agente es lo seleccionable de su bloque y lleva su
    /// número; las filas de ejecución, ninguno. Una por bloque, también si
    /// es de una sola fila o solo terminada.
    #[test]
    fn the_number_goes_on_the_agent_line_one_per_block() {
        let agents = |app: &App| -> Vec<(Option<usize>, String, Chain, String)> {
            app.items()
                .iter()
                .filter_map(|i| match i {
                    Item::LiveAgent { line, .. } => {
                        Some((i.n(), line.name.clone(), line.chain, line.id.clone()))
                    }
                    _ => None,
                })
                .collect()
        };
        let app = fused();
        assert_eq!(
            agents(&app),
            vec![
                (Some(1), "backend".into(), Chain::Head(3), "toolu_b3".into()),
                (Some(2), "frontend".into(), Chain::Single, "toolu_f1".into()),
            ],
            "la línea del agente lleva el id de la cabeza de su bloque"
        );
        assert!(
            app.items()
                .iter()
                .filter(|i| matches!(i, Item::Live { .. } | Item::LiveNow(_)))
                .all(|i| i.n().is_none()),
            "las filas de ejecución no se numeran"
        );
        // el agente definido para `a` va con el bloque, con su mismo número
        let rows: Vec<(usize, String)> = app
            .rows()
            .into_iter()
            .map(|r| (r.n, app.agent(r.agent).name.clone()))
            .collect();
        assert_eq!(rows, vec![(1, "backend".into()), (2, "frontend".into())]);
        // el filtro deja sola una del racimo: su bloque sigue numerado
        let mut app = fused();
        type_filter(&mut app, "tests");
        assert_eq!(
            agents(&app),
            vec![(Some(1), "backend".into(), Chain::Single, "toolu_b4".into())]
        );
    }

    /// Un bloque va de la línea de su agente a su última fila (con sus
    /// `now`): el scroll lo trata como una unidad.
    #[test]
    fn a_block_is_one_unit_for_the_scroll_from_its_agent_line_to_its_last_row() {
        let mut app = fused();
        let items = app.items();
        // columnas 0, regla 1, backend 2..=7 (agente, b3, now, b4, now,
        // terminada), blanco 8, frontend 9..=10
        assert_eq!((App::row_top(&items, 2), App::row_end(&items, 2)), (0, 7));
        assert_eq!((App::row_top(&items, 9), App::row_end(&items, 9)), (9, 10));
        // frontend, abajo: entra entero y el blanco de encima no cuenta
        app.cursor = 1;
        assert_eq!(frame(&mut app, 4), 9);
        assert_eq!(app.scroll, 7);
        // backend, el primero: arrastra todo lo de encima
        app.cursor = 0;
        assert_eq!(frame(&mut app, 4), 2);
        assert_eq!(app.scroll, 0);
        // la rueda no deja el cursor en un bloque cortado: con backend a
        // medias, pasa a frontend, que se ve entero
        frame(&mut app, 6);
        app.update(Action::Scroll(5));
        assert_eq!((app.scroll, app.cursor), (5, 1));
    }

    /// Un bloque más alto que la vista enseña su principio (la línea del
    /// agente, con el número) y lo que quepa.
    #[test]
    fn a_block_taller_than_the_view_shows_its_top() {
        let mut app = fused();
        // frontend delante y backend detrás, como si fuese el segundo bloque
        let mut items = app.items();
        let frontend: Vec<Item> = items.drain(9..).collect();
        let backend: Vec<Item> = items.drain(2..8).collect();
        let v: Vec<Item> = frontend.into_iter().chain(backend).collect();
        assert!(matches!(&v[2], Item::LiveAgent { n: 1, .. }));
        assert_eq!((App::row_top(&v, 2), App::row_end(&v, 2)), (2, 7));
        app.cursor = 0;
        app.scroll = 0;
        app.scroll_to_cursor(&v, 3);
        assert_eq!(app.scroll, 2, "desde la línea del agente");
        app.scroll_to_cursor(&v, 6);
        assert_eq!(app.scroll, 2, "y entero si cabe");
    }

    /// ↑↓ y los dígitos van de bloque en bloque; un clic en cualquier línea
    /// del bloque (la vista las apunta todas a su cursor) lo selecciona, y
    /// otro lo abre.
    #[test]
    fn the_cursor_digits_and_clicks_select_agent_blocks() {
        let mut app = fused();
        key(&mut app, KeyCode::Down);
        assert_eq!(app.cursor, 1);
        assert_eq!(app.live_at_cursor().unwrap().0.name, "frontend");
        key(&mut app, KeyCode::Down);
        assert_eq!(app.cursor, 1, "no hay tercer bloque");
        key(&mut app, KeyCode::Char('1'));
        assert_eq!(app.cursor, 0);
        key(&mut app, KeyCode::Char('3'));
        assert_eq!(app.cursor, 0, "las filas no tienen número propio");
        // como lo deja la vista: las seis líneas de backend, al cursor 0
        app.hits.rows = (2..8)
            .map(|row| RowHit { row, cursor: 0 })
            .chain((9..11).map(|row| RowHit { row, cursor: 1 }))
            .collect();
        app.update(Action::Click { column: 5, row: 10 });
        assert_eq!(app.cursor, 1);
        // la `now` de b4, en medio del bloque de backend
        app.update(Action::Click { column: 5, row: 6 });
        assert_eq!(app.cursor, 0);
        assert!(app.run_detail.is_none());
        assert!(app.agent_view().is_none());
        // y la fila terminada del mismo bloque: ya estaba, abre su pantalla
        app.update(Action::Click { column: 5, row: 7 });
        assert!(app.run_detail.is_none());
        assert_eq!(app.agent_view().unwrap().name, "backend");
    }

    #[test]
    fn enter_twice_opens_the_first_run_of_the_block() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        assert_eq!(open_id(&app), "toolu_b3", "la cabeza del bloque");
        assert_eq!(app.run_nav(), Some((1, 4)), "todas las del bloque");
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('2'));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        assert_eq!(open_id(&app), "toolu_f1", "un bloque solo terminado");
        assert_eq!(app.run_nav(), Some((1, 1)), "una sola: run 1 of 1");
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.run_nav(), None, "sin ejecución abierta no hay posición");
        // con filtro, la pantalla del agente sigue entera: abre en su primera
        let mut app = fused();
        type_filter(&mut app, "tests");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        assert_eq!(open_id(&app), "toolu_b3");
        assert_eq!(
            app.run_nav(),
            Some((1, 4)),
            "la pantalla no depende del filtro"
        );
    }

    #[test]
    fn without_siblings_left_and_right_do_nothing() {
        let mut app = fused();
        key(&mut app, KeyCode::Char('2'));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            (open_id(&app).as_str(), app.run_nav()),
            ("toolu_f1", Some((1, 1)))
        );
        key(&mut app, KeyCode::Left);
        key(&mut app, KeyCode::Right);
        assert_eq!(open_id(&app), "toolu_f1");
    }

    /// Termina b3: pasa a la fila terminada de backend y la representa; b4
    /// se queda sola en curso, de cabeza. La ficha abierta (b1) no cambia.
    #[test]
    fn the_refresh_keeps_the_open_run_when_the_representative_changes() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        for _ in 0..3 {
            key(&mut app, KeyCode::Right);
        }
        assert_eq!(open_id(&app), "toolu_b1");
        let mut runs = app.runs.clone();
        for r in &mut runs {
            if r.tool_use_id.as_deref() == Some("toolu_b3") {
                r.finished = Some(true);
            }
        }
        let calls = app.calls.clone();
        app.update(Action::Msg(Msg::Activity {
            calls,
            runs,
            sessions: 4,
        }));
        app.update(Action::Tick("2026-11-05T12:00:30Z".parse().unwrap()));
        assert_eq!(open_id(&app), "toolu_b1");
        assert_eq!(app.run_nav(), Some((4, 4)), "b4, b3, b2, b1");
        assert_eq!(
            chained(&app)
                .into_iter()
                .map(|r| (r.0, r.3, r.4))
                .collect::<Vec<_>>(),
            vec![
                (1, "toolu_b4".to_string(), Chain::Head(2)),
                (1, "toolu_b3".to_string(), Chain::Last),
                (2, "toolu_f1".to_string(), Chain::Single),
            ]
        );
        // y con ella abierta, Esc vuelve a su agente y de ahí a su bloque
        app.cursor = 1;
        key(&mut app, KeyCode::Esc);
        assert_eq!(
            app.agent_view().unwrap().cursor,
            4,
            "b1 sigue bajo el cursor"
        );
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn esc_returns_to_the_block_that_contains_the_open_run() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        for _ in 0..3 {
            key(&mut app, KeyCode::Right);
        }
        assert_eq!(open_id(&app), "toolu_b1", "fundida en la terminada");
        app.cursor = 1; // fuera de sitio a propósito
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Esc);
        assert!(app.run_detail.is_none() && app.agent_view().is_none());
        assert_eq!(app.cursor, 0, "toolu_b1 vive en el bloque de backend");
        // una en curso que no es la cabeza, también a su bloque
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Right);
        assert_eq!(open_id(&app), "toolu_b4");
        app.cursor = 1;
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.cursor, 0);
        // si el filtro quitó su fila, al bloque de una hermana
        type_filter(&mut app, "cruce");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Right);
        assert_eq!(
            open_id(&app),
            "toolu_b4",
            "la pantalla no depende del filtro"
        );
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn the_agent_detail_opens_with_the_cursor_on_a_run_that_is_not_the_representative() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::End);
        assert_eq!(app.agent_view().unwrap().cursor, 4, "toolu_b1");
        assert!(app.can_open_agent());
        key(&mut app, KeyCode::Char('a'));
        assert!(app.detail && app.run_detail.is_none());
        assert_eq!(app.agent(app.current().unwrap().agent).name, "backend");
    }

    #[test]
    fn the_run_detail_exposes_the_temp_cwd_fields() {
        let mut app = fused();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        let d = app.run_detail.as_ref().unwrap();
        assert_eq!((d.temp_cwd.as_deref(), d.cwd_resolved), (None, false));
    }

    /// Un uso con `total` tokens repartidos en entrada y salida, y el contexto
    /// del último mensaje sobre una ventana de 200k.
    fn usage(total: u64, context: u64) -> crate::runs::usage::Usage {
        crate::runs::usage::Usage {
            tokens: crate::runs::usage::Tokens {
                input: total - total / 4,
                output: total / 4,
                cache_read: 0,
                cache_creation: 0,
            },
            model: Some("claude-opus-5-5".into()),
            context: Some(context),
            context_window: 200_000,
        }
    }

    /// `fused` con tokens: b1 1000, b2 2000, b3 (en curso) 4000 con 50k de
    /// contexto, b4 (en curso) 500 con 30k, f1 300.
    fn fused_with_tokens() -> App {
        let mut app = fused();
        let mut runs = app.runs.clone();
        for r in &mut runs {
            r.usage = match r.tool_use_id.as_deref() {
                Some("toolu_b1") => usage(1000, 900),
                Some("toolu_b2") => usage(2000, 1900),
                Some("toolu_b3") => usage(4000, 50_000),
                Some("toolu_b4") => usage(500, 30_000),
                _ => usage(300, 200),
            };
        }
        app = with_runs(app, runs);
        app
    }

    /// La fila de la ejecución `id` (o la terminada que representa).
    fn live_line(app: &App, id: &str) -> NowLine {
        app.items()
            .into_iter()
            .find_map(|i| match i {
                Item::Live { line } if line.id == id => Some(line),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn live_lines_carry_the_tokens_of_the_row_and_the_context_only_running() {
        let app = fused_with_tokens();
        let b3 = live_line(&app, "toolu_b3");
        assert_eq!(b3.tokens, 4000, "en curso: solo la suya");
        let ctx = b3.context.unwrap();
        assert_eq!((ctx.used, ctx.window, ctx.percent()), (50_000, 200_000, 25));
        let b4 = live_line(&app, "toolu_b4");
        assert_eq!((b4.tokens, b4.context.map(|c| c.used)), (500, Some(30_000)));
        let frontend = live_line(&app, "toolu_f1");
        assert_eq!((frontend.tokens, frontend.context), (300, None));
        let backend = live_line(&app, "toolu_b2");
        assert_eq!(
            (backend.tokens, backend.context),
            (3000, None),
            "terminada: las dos ejecuciones de la fila, sin contexto"
        );
    }

    #[test]
    fn the_active_header_sums_the_tokens_of_the_rows_it_shows() {
        let mut app = fused_with_tokens();
        assert_eq!(app.live_tokens(), 7800);
        type_filter(&mut app, "front");
        assert_eq!(app.live_tokens(), 300, "solo lo que deja el filtro");
        let mut app = fused_with_tokens();
        type_filter(&mut app, "nada-que-encaje");
        assert_eq!(app.live_tokens(), 0);
    }

    /// Una ejecución con id de datos `data` y `tool_use_id` `tool`, en
    /// station, que empezó `start` y trabajó `mins`; terminada si `finished`.
    fn linked(agent_type: &str, data: &str, tool: &str, start: &str, mins: i64) -> Run {
        // las que acaban a las 11:58 siguen en curso; las demás, terminadas
        let end = start.parse::<DateTime<Utc>>().unwrap() + Duration::minutes(mins);
        let finished = end < "2026-11-05T11:58:00Z".parse::<DateTime<Utc>>().unwrap();
        let mut r = run(agent_type, STATION, start, mins, Some(tool), finished);
        r.id = data.into();
        r.session_id = Some("sess-1234abcd".into());
        r.session_prompt = Some("arregla el ranking".into());
        r
    }

    /// Station: `backend` (toolu_p, en curso) lanzó `frontend` (toolu_c1,
    /// terminada) y `docs` (toolu_c2, en curso); `docs` lanzó a su vez
    /// `visual` (toolu_g1). `backend` lo lanzó la sesión.
    pub(super) fn family() -> App {
        let mut app = App::new(60, PathBuf::from("/Users/x"), "0.1.0", NOW.parse().unwrap());
        app.tz = Some(utc());
        app.update(Action::Msg(Msg::Agents(vec![
            agent("backend", Some(STATION)),
            agent("frontend", Some(STATION)),
            agent("docs", Some(STATION)),
            agent("visual", Some(STATION)),
        ])));
        let mut p = linked("backend", "a-p", "toolu_p", "2026-11-05T11:40:00Z", 18);
        p.children = vec!["a-c1".into(), "a-c2".into()];
        let mut c1 = linked("frontend", "a-c1", "toolu_c1", "2026-11-05T11:42:00Z", 3);
        c1.parent_run = Some("a-p".into());
        let mut c2 = linked("docs", "a-c2", "toolu_c2", "2026-11-05T11:45:00Z", 13);
        c2.parent_run = Some("a-p".into());
        c2.children = vec!["a-g1".into()];
        let mut g1 = linked("visual", "a-g1", "toolu_g1", "2026-11-05T11:50:00Z", 8);
        g1.parent_run = Some("a-c2".into());
        with_runs(app, vec![p, c1, c2, g1])
    }

    /// Abre la pantalla de la ejecución `tool` (la primera de su bloque)
    /// desde la lista de activos, pasando por la pantalla de su agente.
    fn open_from_list(app: &mut App, tool: &str) {
        let n = app
            .items()
            .into_iter()
            .find_map(|i| match i {
                Item::LiveAgent { n, line, .. } if line.id == tool => Some(n),
                _ => None,
            })
            .unwrap();
        app.cursor = n - 1;
        key(app, KeyCode::Enter);
        key(app, KeyCode::Enter);
        assert_eq!(open_id(app), tool);
    }

    #[test]
    fn tab_walks_the_launched_runs_and_enter_opens_the_selected_one() {
        let mut app = family();
        open_from_list(&mut app, "toolu_p");
        let launched: Vec<String> = app
            .run_detail
            .as_ref()
            .unwrap()
            .launched
            .iter()
            .map(|l| l.id.clone())
            .collect();
        assert_eq!(launched, vec!["toolu_c1", "toolu_c2"]);
        assert_eq!(app.launched_sel, None, "al abrir no hay ninguna elegida");
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.launched_sel, Some(0));
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.launched_sel, Some(1));
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.launched_sel, None, "tras la última, ninguna");
        key(&mut app, KeyCode::BackTab);
        assert_eq!(app.launched_sel, Some(1), "shift-tab, desde el final");
        key(&mut app, KeyCode::BackTab);
        key(&mut app, KeyCode::BackTab);
        assert_eq!(app.launched_sel, None);
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Enter);
        assert_eq!(open_id(&app), "toolu_c2");
        assert_eq!(app.launched_sel, None, "la hija se abre sin elegida");
        assert!(app.can_go_back());
    }

    #[test]
    fn digits_in_the_run_detail_select_a_launched_run() {
        let mut app = family();
        open_from_list(&mut app, "toolu_p");
        key(&mut app, KeyCode::Char('2'));
        assert_eq!(app.launched_sel, Some(1));
        key(&mut app, KeyCode::Char('9'));
        assert_eq!(app.launched_sel, Some(1), "fuera de rango no cambia");
        key(&mut app, KeyCode::Char('1'));
        assert_eq!(app.launched_sel, Some(0));
        key(&mut app, KeyCode::Enter);
        assert_eq!(open_id(&app), "toolu_c1");
    }

    #[test]
    fn without_launched_runs_tab_and_digits_select_nothing() {
        let mut app = family();
        open_from_list(&mut app, "toolu_c1");
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Char('1'));
        assert_eq!(app.launched_sel, None);
        key(&mut app, KeyCode::Enter);
        assert!(app.run_detail.is_none(), "sin elegida, Enter vuelve");
        assert!(app.agent_view().is_some(), "a la pantalla del agente");
    }

    #[test]
    fn esc_goes_back_through_the_opened_runs_and_then_to_the_list() {
        let mut app = family();
        open_from_list(&mut app, "toolu_p");
        let cursor = app.cursor;
        app.steps_scroll = 0;
        key(&mut app, KeyCode::Char('2'));
        key(&mut app, KeyCode::Enter); // docs
        key(&mut app, KeyCode::Char('1'));
        key(&mut app, KeyCode::Enter); // visual
        assert_eq!(open_id(&app), "toolu_g1");
        key(&mut app, KeyCode::Esc);
        assert_eq!(open_id(&app), "toolu_c2");
        assert_eq!(
            app.launched_sel,
            Some(0),
            "vuelve con la hija que abrió elegida"
        );
        key(&mut app, KeyCode::Esc);
        assert_eq!(open_id(&app), "toolu_p");
        assert_eq!(app.launched_sel, Some(1));
        assert!(!app.can_go_back());
        key(&mut app, KeyCode::Esc);
        assert!(app.run_detail.is_none());
        assert_eq!(app.agent_view().unwrap().name, "backend");
        key(&mut app, KeyCode::Esc);
        assert!(app.agent_view().is_none());
        assert_eq!(app.cursor, cursor, "el cursor sigue en la fila de partida");
    }

    #[test]
    fn p_opens_the_parent_run_and_esc_returns_to_the_child() {
        let mut app = family();
        open_from_list(&mut app, "toolu_g1");
        assert!(app.can_open_parent());
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(open_id(&app), "toolu_c2");
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(open_id(&app), "toolu_p");
        assert!(!app.can_open_parent(), "a backend lo lanzó la sesión");
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(open_id(&app), "toolu_p", "sin madre, p no hace nada");
        key(&mut app, KeyCode::Esc);
        assert_eq!(open_id(&app), "toolu_c2");
        key(&mut app, KeyCode::Esc);
        assert_eq!(open_id(&app), "toolu_g1");
        key(&mut app, KeyCode::Esc);
        assert!(app.run_detail.is_none());
    }

    #[test]
    fn a_parent_that_is_gone_cannot_be_opened() {
        let mut app = family();
        let runs: Vec<Run> = app
            .runs
            .iter()
            .filter(|r| r.id != "a-c2")
            .cloned()
            .collect();
        app = with_runs(app, runs);
        open_from_list(&mut app, "toolu_g1");
        assert!(matches!(
            app.run_detail.as_ref().unwrap().launched_by,
            Some(status::LaunchedBy::GoneRun { .. })
        ));
        assert!(!app.can_open_parent());
        key(&mut app, KeyCode::Char('p'));
        assert_eq!(open_id(&app), "toolu_g1");
    }

    #[test]
    fn arrows_keep_the_back_stack() {
        let mut app = family();
        open_from_list(&mut app, "toolu_p");
        key(&mut app, KeyCode::Char('1'));
        key(&mut app, KeyCode::Enter);
        assert!(app.can_go_back());
        key(&mut app, KeyCode::Left); // sin hermanas: nada, la pila sigue
        assert!(app.can_go_back());
        key(&mut app, KeyCode::Up); // ↑↓ ya no cambian de agente
        assert_eq!(open_id(&app), "toolu_c1");
        assert!(app.can_go_back());
        key(&mut app, KeyCode::Esc);
        assert_eq!(open_id(&app), "toolu_p");
        key(&mut app, KeyCode::Esc);
        assert!(app.run_detail.is_none());
    }

    /// La ficha del agente ya no se abre desde la pantalla de la ejecución:
    /// `a` vive en la pantalla del agente.
    #[test]
    fn the_agent_key_does_nothing_in_the_run_screen() {
        let mut app = family();
        open_from_list(&mut app, "toolu_p");
        key(&mut app, KeyCode::Char('a'));
        assert!(!app.detail && app.mode == Mode::Active);
        assert_eq!(open_id(&app), "toolu_p");
        key(&mut app, KeyCode::Char('1'));
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('a'));
        assert!(!app.detail && app.can_go_back());
        assert_eq!(open_id(&app), "toolu_c1");
    }

    #[test]
    fn the_selection_shrinks_when_a_launched_run_disappears() {
        let mut app = family();
        open_from_list(&mut app, "toolu_p");
        key(&mut app, KeyCode::Char('2'));
        let runs: Vec<Run> = app
            .runs
            .iter()
            .filter(|r| r.id != "a-c2")
            .cloned()
            .collect();
        app = with_runs(app, runs);
        assert_eq!(app.run_detail.as_ref().unwrap().launched.len(), 1);
        assert_eq!(app.launched_sel, Some(0));
    }

    /// `ranking()` con tokens: docs 500 + 100 (la de hace 40 días), visual
    /// 9000, frontend 2000; y `backend` con una ejecución de 0 min de ayer
    /// con 50 tokens.
    fn ranking_with_tokens() -> App {
        let app = ranking();
        let mut runs = app.runs.clone();
        for r in &mut runs {
            r.usage = match (r.agent_type.as_str(), r.worked.num_minutes()) {
                ("docs", 50) => usage(500, 1),
                ("docs", _) => usage(100, 1),
                ("visual", _) => usage(9000, 1),
                _ => usage(2000, 1),
            };
        }
        let mut idle = run("backend", STATION, "2026-11-04T08:00:00Z", 0, None, true);
        idle.usage = usage(50, 1);
        idle.worked_slots.clear();
        runs.push(idle);
        with_runs(app, runs)
    }

    fn stats(app: &App) -> Vec<(String, i64, u64, usize)> {
        app.items()
            .iter()
            .filter_map(|i| match i {
                Item::Rank { row, work } => Some((
                    app.agent(row.agent).name.clone(),
                    work.worked.num_minutes(),
                    work.tokens,
                    work.runs,
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_ranking_carries_tokens_and_includes_runs_without_worked_time() {
        let app = ranking_with_tokens();
        assert_eq!(app.rank_key, RankKey::Worked);
        assert_eq!(
            stats(&app),
            vec![
                ("docs".into(), 72, 600, 2),
                ("visual".into(), 30, 9000, 1),
                ("frontend".into(), 5, 2000, 1),
                ("backend".into(), 0, 50, 1),
            ]
        );
        assert!(matches!(app.items()[0], Item::RankRule { agents: 4 }));
        assert_eq!(
            app.ranking_total(),
            PeriodStats {
                worked: Duration::minutes(107),
                runs: 5,
                tokens: 11_650,
            }
        );
    }

    #[test]
    fn s_cycles_the_rank_key_and_reorders_the_ranking() {
        let mut app = ranking_with_tokens();
        let order = |app: &App| -> Vec<String> { stats(app).into_iter().map(|s| s.0).collect() };
        key(&mut app, KeyCode::Char('s'));
        assert_eq!(app.rank_key, RankKey::Tokens);
        assert_eq!(order(&app), vec!["visual", "frontend", "docs", "backend"]);
        let ns: Vec<usize> = app.rows().iter().map(|r| r.n).collect();
        assert_eq!(ns, vec![1, 2, 3, 4]);
        key(&mut app, KeyCode::Char('s'));
        assert_eq!(app.rank_key, RankKey::Runs);
        // docs 2; el resto 1, desempatados por trabajado
        assert_eq!(order(&app), vec!["docs", "visual", "frontend", "backend"]);
        key(&mut app, KeyCode::Char('s'));
        assert_eq!(app.rank_key, RankKey::Worked);
    }

    #[test]
    fn s_only_acts_in_the_ranking() {
        let mut app = fixture();
        key(&mut app, KeyCode::Char('s'));
        assert_eq!(app.rank_key, RankKey::Worked);
        let mut app = fused();
        key(&mut app, KeyCode::Char('s'));
        assert_eq!(app.rank_key, RankKey::Worked);
    }

    #[test]
    fn the_ranking_counts_the_runs_kept_only_in_the_history_cache() {
        let app = ranking();
        assert_eq!(app.cached_only(), 0);
        let mut runs = app.runs.clone();
        runs[1].from_cache = true;
        runs[3].from_cache = true;
        let app = with_runs(app, runs);
        assert_eq!(app.cached_only(), 2);
    }

    // --- ratón: clics sobre el mapa `hits` que rellena la vista ---

    fn click(app: &mut App, column: u16, row: u16) {
        app.update(Action::Click { column, row });
    }

    /// Pestañas en la fila 0 tal como las pintaría la vista.
    fn tab_hits() -> Vec<TabHit> {
        vec![
            TabHit {
                mode: Mode::Active,
                row: 0,
                cols: 2..8,
            },
            TabHit {
                mode: Mode::All,
                row: 0,
                cols: 10..13,
            },
            TabHit {
                mode: Mode::Ranking,
                row: 0,
                cols: 15..22,
            },
        ]
    }

    /// Una línea de pantalla por fila seleccionable: la `i` en la fila `3 + i`.
    fn row_hits(n: usize) -> Vec<RowHit> {
        (0..n)
            .map(|i| RowHit {
                row: 3 + i as u16,
                cursor: i,
            })
            .collect()
    }

    #[test]
    fn clicking_a_tab_switches_to_that_mode_like_tab() {
        let mut app = fixture();
        app.hits.tabs = tab_hits();
        app.cursor = 2;
        click(&mut app, 16, 0);
        assert_eq!(app.mode, Mode::Ranking);
        click(&mut app, 10, 0);
        assert_eq!(app.mode, Mode::All);
        click(&mut app, 7, 0);
        assert_eq!(app.mode, Mode::Active);
        click(&mut app, 7, 0);
        assert_eq!(app.mode, Mode::Active, "la pestaña abierta no cambia nada");
    }

    #[test]
    fn a_tab_click_needs_the_right_row_and_column() {
        let mut app = fixture();
        app.mode = Mode::Active;
        app.hits.tabs = tab_hits();
        click(&mut app, 9, 0); // entre «active» y «all»
        click(&mut app, 13, 0); // el final del rango no cuenta
        click(&mut app, 11, 1); // otra fila
        assert_eq!(app.mode, Mode::Active);
    }

    #[test]
    fn clicking_a_row_moves_the_cursor_and_a_second_click_opens_it() {
        let mut app = many(5);
        app.hits.rows = row_hits(5);
        click(&mut app, 40, 6);
        assert_eq!(app.cursor, 3);
        assert!(!app.detail, "el primer clic solo mueve el cursor");
        click(&mut app, 0, 6);
        assert!(app.detail, "clic en la fila ya elegida = enter");
        assert_eq!(app.agent(app.current().unwrap().agent).name, "a03");
    }

    #[test]
    fn clicking_outside_any_zone_does_nothing() {
        let mut app = many(5);
        app.hits.tabs = tab_hits();
        app.hits.rows = row_hits(5);
        app.cursor = 1;
        click(&mut app, 0, 2);
        click(&mut app, 0, 40);
        assert_eq!((app.cursor, app.mode, app.detail), (1, Mode::All, false));
    }

    #[test]
    fn a_second_click_on_a_folded_group_unfolds_it_like_enter() {
        let mut app = history();
        app.hits.rows = row_hits(app.count());
        click(&mut app, 1, 9);
        assert_eq!(app.cursor, 6);
        click(&mut app, 1, 9);
        assert!(folded(&app)[0].3, "desplegado");
        assert!(!app.detail);
    }

    #[test]
    fn with_the_agent_detail_open_only_tabs_answer_and_they_close_it() {
        let mut app = many(5);
        app.hits.tabs = tab_hits();
        app.hits.rows = row_hits(5);
        key(&mut app, KeyCode::Enter);
        assert!(app.detail);
        click(&mut app, 0, 5);
        assert_eq!(
            (app.cursor, app.detail),
            (0, true),
            "las filas no responden"
        );
        click(&mut app, 16, 0);
        assert_eq!((app.mode, app.detail), (Mode::Ranking, false));
    }

    #[test]
    fn with_the_run_detail_open_clicks_are_ignored() {
        let mut app = fused();
        app.hits.tabs = tab_hits();
        app.hits.rows = row_hits(2);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Enter);
        click(&mut app, 16, 0);
        click(&mut app, 0, 4);
        assert_eq!(app.mode, Mode::Active);
        assert_eq!(open_id(&app), "toolu_b3");
    }

    #[test]
    fn while_typing_the_filter_clicks_are_ignored() {
        let mut app = many(5);
        app.hits.tabs = tab_hits();
        app.hits.rows = row_hits(5);
        key(&mut app, KeyCode::Char('/'));
        click(&mut app, 16, 0);
        click(&mut app, 0, 5);
        assert_eq!((app.mode, app.cursor, app.filtering), (Mode::All, 0, true));
    }

    #[test]
    fn a_click_ends_a_pending_digit() {
        let mut app = many(12);
        app.hits.rows = row_hits(12);
        key(&mut app, KeyCode::Char('1')); // podría ser 10, 11 o 12
        click(&mut app, 0, 5);
        assert_eq!(app.cursor, 2);
        key(&mut app, KeyCode::Char('2'));
        assert_eq!(
            app.cursor, 1,
            "el 1 de antes ya no espera: es el 2, no el 12"
        );
    }
}
