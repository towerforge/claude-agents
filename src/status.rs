//! Cruce entre definiciones, invocaciones y ejecuciones: qué agente está
//! activo, desde cuándo y cuánto ha trabajado.

use crate::agents::{Agent, Scope};
use crate::runs::usage::Usage;
use crate::runs::{Run, SkillUse, ToolStep};
use crate::transcripts::Invocation;
use chrono::{DateTime, NaiveDate, TimeDelta, TimeZone, Utc};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::AddAssign;
use std::path::{Path, PathBuf};

/// La regla común a llamadas y ejecuciones: mismo nombre; un agente global
/// encaja con cualquier carpeta, uno de proyecto solo con una carpeta dentro
/// del proyecto (por componentes enteros). Sin carpeta, solo encaja uno global.
/// La carpeta es la de lanzamiento de la sesión (`run_dir`/`call_dir`): Claude
/// Code carga `.claude/agents/` de ahí, no del `cwd` del subagente.
fn belongs_to(agent: &Agent, name: &str, dir: Option<&Path>) -> bool {
    name == agent.name
        && match &agent.scope {
            Scope::Global => true,
            Scope::Project(root) => dir.is_some_and(|d| d.starts_with(root)),
        }
}

/// Carpeta de una ejecución para el dueño y el grupo: donde se lanzó la sesión
/// (`Run::launch_dir`) o, si no se sabe, su `cwd`.
fn run_dir(r: &Run) -> Option<&Path> {
    r.launch_dir.as_deref().or(r.cwd.as_deref())
}

/// Lo mismo para una llamada: `Invocation::launch_dir` o, si no, su `cwd`.
fn call_dir(c: &Invocation) -> &Path {
    c.launch_dir.as_deref().unwrap_or(&c.cwd)
}

/// Invocaciones que encajan con el agente, de la más reciente a la más antigua.
/// Un agente global encaja con cualquier sesión; uno de proyecto, solo con las
/// sesiones lanzadas dentro del proyecto (o, sin ese dato, con `cwd` dentro).
pub fn invocations_of<'a>(agent: &Agent, calls: &'a [Invocation]) -> Vec<&'a Invocation> {
    let mut found: Vec<&Invocation> = calls
        .iter()
        .filter(|c| belongs_to(agent, &c.agent, Some(call_dir(c))))
        .collect();
    found.sort_by_key(|c| std::cmp::Reverse(c.at));
    found
}

/// Instante de la última invocación que encaja con el agente, si hay alguna.
pub fn last_invocation(agent: &Agent, calls: &[Invocation]) -> Option<DateTime<Utc>> {
    invocations_of(agent, calls).first().map(|c| c.at)
}

/// Última vez que se usó el agente: su última llamada o el último paso de una
/// de sus ejecuciones, lo que sea más tarde. Las ejecuciones lanzadas desde un
/// workflow no tienen llamada a `Agent`, pero sí trabajo.
pub fn last_use(agent: &Agent, calls: &[Invocation], runs: &[Run]) -> Option<DateTime<Utc>> {
    let run_last = runs_of(agent, runs).iter().filter_map(|r| r.last).max();
    last_invocation(agent, calls).max(run_last)
}

/// Ejecuciones del agente (misma regla que las llamadas, con `agentType` como
/// nombre), de la que empezó más tarde a la más antigua.
pub fn runs_of<'a>(agent: &Agent, runs: &'a [Run]) -> Vec<&'a Run> {
    let mut found: Vec<&Run> = runs
        .iter()
        .filter(|r| belongs_to(agent, &r.agent_type, run_dir(r)))
        .collect();
    found.sort_by_key(|r| std::cmp::Reverse(r.start));
    found
}

/// Una ejecución en curso: cuándo empezó y lo que lleva trabajado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Running {
    pub since: Option<DateTime<Utc>>,
    pub worked: TimeDelta,
}

/// Trabajo de un agente sumando todas sus ejecuciones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Work {
    /// Suma de `Run::worked_until(now)`: las en curso cuentan hasta ahora.
    pub total: TimeDelta,
    /// Número de ejecuciones; 0 si no hay ninguna registrada.
    pub runs: usize,
    /// `Tokens::total` de esas mismas ejecuciones, de siempre. Las en curso,
    /// lo que llevan.
    pub tokens: u64,
    /// La ejecución en curso que empezó antes, si hay alguna.
    pub running: Option<Running>,
}

/// Cuánto ha trabajado el agente a fecha de `now`, en cuántas ejecuciones y si
/// alguna sigue en curso (`Run::in_progress`: sin terminar y tocada hace menos
/// de 10 min; si no, interrumpida).
pub fn work_of(agent: &Agent, runs: &[Run], now: DateTime<Utc>) -> Work {
    let mine = runs_of(agent, runs);
    let running = mine
        .iter()
        .filter(|r| r.in_progress(now))
        .min_by_key(|r| r.start)
        .map(|r| Running {
            since: r.start,
            worked: r.worked_until(now),
        });
    Work {
        total: mine.iter().map(|r| r.worked_until(now)).sum(),
        runs: mine.len(),
        tokens: mine.iter().map(|r| r.usage.tokens.total()).sum(),
        running,
    }
}

/// La ejecución que lanzó la llamada: su `toolUseId` es el `id` del bloque
/// `tool_use`. Una llamada sin `id` (las antiguas) no enlaza con ninguna.
pub fn run_of<'a>(call: &Invocation, runs: &'a [Run]) -> Option<&'a Run> {
    let id = call.id.as_deref()?;
    runs.iter().find(|r| r.tool_use_id.as_deref() == Some(id))
}

/// Hoy en la zona `tz` (`chrono::Local` en producción).
fn today<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> NaiveDate {
    now.with_timezone(tz).date_naive()
}

/// Trabajo del agente en cada uno de los últimos `n` días locales, del más
/// antiguo (hoy - `n` + 1) a hoy, el último. Las ejecuciones en curso cuentan
/// hasta `now` (`Run::worked_by_day_until`); cada hueco va al día donde empieza.
pub fn days_of<Tz: TimeZone>(
    agent: &Agent,
    runs: &[Run],
    now: DateTime<Utc>,
    tz: &Tz,
    n: usize,
) -> Vec<TimeDelta> {
    let today = today(now, tz);
    let mut days = vec![TimeDelta::zero(); n];
    for r in runs_of(agent, runs) {
        for (day, worked) in r.worked_by_day_until(now, tz) {
            let back = (today - day).num_days();
            if (0..n as i64).contains(&back) {
                days[n - 1 - back as usize] += worked;
            }
        }
    }
    days
}

/// Periodo del ranking. Por defecto, siempre.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Period {
    Week,
    Month,
    #[default]
    Ever,
}

impl Period {
    /// El siguiente al pulsar `p`: 7 d → 30 d → siempre → 7 d.
    pub fn next(self) -> Self {
        match self {
            Period::Week => Period::Month,
            Period::Month => Period::Ever,
            Period::Ever => Period::Week,
        }
    }

    /// Días locales que abarca, contando hoy; `None` es todo el historial.
    pub fn days(self) -> Option<u32> {
        match self {
            Period::Week => Some(7),
            Period::Month => Some(30),
            Period::Ever => None,
        }
    }
}

/// Trabajo y ejecuciones de un agente dentro de un periodo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PeriodWork {
    /// Suma del trabajo de los días locales del periodo (hasta `now` si hay
    /// alguna en curso).
    pub worked: TimeDelta,
    /// Ejecuciones que empezaron dentro del periodo.
    pub runs: usize,
}

/// Trabajo del agente en el periodo: los días locales desde hoy - días + 1
/// hasta hoy. Una ejecución que cruza el inicio aporta solo sus días de dentro
/// y no cuenta como ejecución del periodo. `Period::Ever` es todo, igual que
/// `work_of`.
pub fn work_in<Tz: TimeZone>(
    agent: &Agent,
    runs: &[Run],
    now: DateTime<Utc>,
    tz: &Tz,
    period: Period,
) -> PeriodWork {
    let mine = runs_of(agent, runs);
    let Some(n) = period.days() else {
        return PeriodWork {
            worked: mine.iter().map(|r| r.worked_until(now)).sum(),
            runs: mine.len(),
        };
    };
    let from = today(now, tz) - TimeDelta::days(i64::from(n) - 1);
    PeriodWork {
        worked: mine
            .iter()
            .flat_map(|r| r.worked_by_day_until(now, tz))
            .filter(|(day, _)| *day >= from)
            .map(|(_, worked)| worked)
            .sum(),
        runs: mine
            .iter()
            .filter(|r| r.start_local(tz).is_some_and(|s| s.date_naive() >= from))
            .count(),
    }
}

/// Trabajo, ejecuciones y tokens de un agente en un periodo: lo de `work_in`
/// más los tokens. Es la fila del ranking; se suma con `+=` para su total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PeriodStats {
    /// Como `PeriodWork::worked`.
    pub worked: TimeDelta,
    /// Como `PeriodWork::runs`: ejecuciones que empezaron en el periodo.
    pub runs: usize,
    /// `Tokens::total` de esas mismas ejecuciones (las que empezaron en el
    /// periodo, enteras: no hay tokens por día). Las en curso, lo que llevan.
    pub tokens: u64,
}

impl AddAssign for PeriodStats {
    fn add_assign(&mut self, o: Self) {
        self.worked += o.worked;
        self.runs += o.runs;
        self.tokens += o.tokens;
    }
}

/// Por qué se ordena el ranking. Por defecto, tiempo trabajado.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RankKey {
    #[default]
    Worked,
    Tokens,
    Runs,
}

impl RankKey {
    /// El siguiente al pulsar `s`: trabajado → tokens → ejecuciones → trabajado.
    pub fn next(self) -> Self {
        match self {
            RankKey::Worked => RankKey::Tokens,
            RankKey::Tokens => RankKey::Runs,
            RankKey::Runs => RankKey::Worked,
        }
    }

    /// Orden del ranking para `sort_by`: de más a menos por la clave y, a
    /// igualdad, por las otras dos (trabajado, tokens, ejecuciones en ese
    /// orden de preferencia). El desempate final (nombre, sección) es de quien
    /// ordena.
    pub fn cmp(self, a: &PeriodStats, b: &PeriodStats) -> Ordering {
        let worked = b.worked.cmp(&a.worked);
        let tokens = b.tokens.cmp(&a.tokens);
        let runs = b.runs.cmp(&a.runs);
        match self {
            RankKey::Worked => worked.then(tokens).then(runs),
            RankKey::Tokens => tokens.then(worked).then(runs),
            RankKey::Runs => runs.then(worked).then(tokens),
        }
    }
}

/// `work_in` más los tokens de las ejecuciones que empezaron en el periodo.
pub fn stats_in<Tz: TimeZone>(
    agent: &Agent,
    runs: &[Run],
    now: DateTime<Utc>,
    tz: &Tz,
    period: Period,
) -> PeriodStats {
    let work = work_in(agent, runs, now, tz, period);
    let from = period
        .days()
        .map(|n| today(now, tz) - TimeDelta::days(i64::from(n) - 1));
    let tokens = runs_of(agent, runs)
        .iter()
        .filter(|r| {
            from.is_none_or(|from| r.start_local(tz).is_some_and(|s| s.date_naive() >= from))
        })
        .map(|r| r.usage.tokens.total())
        .sum();
    PeriodStats {
        worked: work.worked,
        runs: work.runs,
        tokens,
    }
}

/// Cuántas ejecuciones salen solo del historial guardado (`Run::from_cache`):
/// las que Claude Code ya borró. Con `Runs::runs()` es lo mismo que
/// `Runs::cached_only()`, sin tener que mandarlo aparte.
pub fn cached_only(runs: &[Run]) -> usize {
    runs.iter().filter(|r| r.from_cache).count()
}

/// Lo trabajado por un grupo: la suma del total de sus agentes.
pub fn total_worked<'a>(works: impl IntoIterator<Item = &'a Work>) -> TimeDelta {
    works.into_iter().map(|w| w.total).sum()
}

/// Se ha usado alguna vez: tiene alguna llamada o alguna ejecución, aunque sea
/// de hace años o de 0 min. Un grupo sin ningún agente usado va plegado.
pub fn ever_used(agent: &Agent, calls: &[Invocation], runs: &[Run]) -> bool {
    calls
        .iter()
        .any(|c| belongs_to(agent, &c.agent, Some(call_dir(c))))
        || runs
            .iter()
            .any(|r| belongs_to(agent, &r.agent_type, run_dir(r)))
}

/// Llamadas del agente que no enlazan con ninguna ejecución (las antiguas, sin
/// `id`, o cuya transcripción ya no está): «+ N llamadas antiguas sin registro
/// de duración».
pub fn calls_without_run(agent: &Agent, calls: &[Invocation], runs: &[Run]) -> usize {
    invocations_of(agent, calls)
        .into_iter()
        .filter(|c| run_of(c, runs).is_none())
        .count()
}

/// De dónde viene una ejecución en curso.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// Un agente definido (`.claude/agents`) lanzado directamente.
    Defined,
    /// Un tipo que no es un agente definido (`general-purpose`, `Explore`…).
    BuiltIn,
    /// Un paso de workflow (`workflow-subagent`) cuya `description` es un
    /// agente definido: es ese agente, lanzado por un workflow.
    Workflow,
    /// Un paso de workflow cuya `description` no es ningún agente definido (o
    /// que no trae `description`): no es un agente propio.
    WorkflowStep,
}

/// En qué punto está una línea de activos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// En curso (`Run::in_progress`).
    Running,
    /// Terminada dentro de la ventana: `ended` es el último paso de la
    /// ejecución o, sin ejecución (llamada sin enlazar), la hora de la llamada.
    Done { ended: DateTime<Utc> },
}

/// Posición de una fila dentro del racimo (el bloque) de su agente en un
/// proyecto: sus ejecuciones en curso contiguas, la que empezó más tarde
/// arriba, y al final su fila terminada si la tiene. La vista pinta el nombre
/// en una línea encima del bloque y `├`/`└` en sus filas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chain {
    /// Sola: el bloque del agente en el grupo tiene una única fila (una en
    /// curso sin terminada, o solo su terminada).
    Single,
    /// La primera de un racimo de N ≥ 2 filas (N = el campo): en curso más,
    /// si la hay, la terminada. Ojo: la terminada puede fundir varias
    /// ejecuciones (`NowRun::runs`), así que N cuenta filas, no ejecuciones.
    Head(usize),
    /// Ni la primera ni la última de su racimo.
    Middle,
    /// La última de su racimo: la terminada, si el agente la tiene.
    Last,
}

/// Contexto ocupado por el último mensaje de una ejecución sobre su ventana.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    /// `Usage::context`: entrada con y sin caché del último mensaje.
    pub used: u64,
    /// `Usage::context_window` (200k o 1M).
    pub window: u64,
}

impl Context {
    /// El de `usage`; `None` si aún no hay ningún mensaje con uso.
    pub fn of(usage: &Usage) -> Option<Self> {
        usage.context.map(|used| Context {
            used,
            window: usage.context_window,
        })
    }

    /// Porcentaje ocupado, redondeado; puede pasar de 100. 0 sin ventana (una
    /// línea vieja del historial sin `context_window`).
    pub fn percent(&self) -> u64 {
        if self.window == 0 {
            return 0;
        }
        (self.used.saturating_mul(100) + self.window / 2) / self.window
    }
}

/// Una línea de activos de cualquier tipo (en curso o terminada en la
/// ventana), con su identidad ya resuelta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowRun<'a> {
    /// Identificador estable de la ejecución (`run_id`): el mismo entre
    /// refrescos aunque cambie el orden o aparezcan otras.
    pub id: String,
    /// Nombre resuelto: el agente definido, el `agentType` si es built-in, o
    /// en un workflow su `description` («workflow» si no la tiene).
    pub name: &'a str,
    pub kind: RunKind,
    /// Índice del agente definido al que corresponde (también en un workflow
    /// cuyo agente real es uno definido): lo que abre la ficha.
    pub owner: Option<usize>,
    /// El `cwd` real del subagente (donde estaba el agente principal al lanzarlo).
    pub cwd: Option<&'a Path>,
    /// La carpeta del grupo y del dueño: donde se lanzó la sesión de Claude Code
    /// (`launch_dir`) o, si no se sabe, `cwd`. Igual a `cwd` si no hubo `cd`.
    pub group_dir: Option<&'a Path>,
    pub since: Option<DateTime<Utc>>,
    /// Lo que lleva trabajado hasta `now`.
    pub worked: TimeDelta,
    pub tools: usize,
    /// El encargo: `Run::task` o, sin él, la `description` salvo que esta sea
    /// el nombre del agente de un workflow.
    pub task: Option<&'a str>,
    /// Última herramienta usada (`Bash · git diff --stat`).
    pub last_tool: Option<&'a str>,
    /// Última skill invocada, tal cual (`superpowers:brainstorming`; `Run::skill`).
    /// En una fila que funde varias ejecuciones, la de la representante.
    pub skill: Option<&'a str>,
    /// El encargo completo, con sus saltos de línea (`Run::task_full`).
    pub task_full: Option<&'a str>,
    /// Los últimos pasos, del más antiguo al más reciente (`Run::steps`).
    pub steps: &'a [ToolStep],
    /// Usos por herramienta sobre toda la ejecución, de más a menos usada.
    pub tool_counts: &'a [(String, usize)],
    /// En curso o terminada (y cuándo terminó).
    pub state: RunState,
    /// Cuántas ejecuciones (o llamadas sin ejecución) representa la fila (≥ 1).
    /// Una fila en curso es siempre UNA ejecución (1); solo las terminadas de
    /// `active_groups` funden las de su agente. Fuera de `active_groups` vale 1.
    pub runs: usize,
    /// Los ids de las ejecuciones de la fila, de la más reciente a la más
    /// antigua; la primera es `id`. En una fila en curso, solo `[id]`.
    pub run_ids: Vec<String>,
    /// `Tokens::total` sumado sobre las ejecuciones de la fila (las de
    /// `run_ids`; en curso, la suya); las llamadas sin ejecución suman 0.
    pub tokens: u64,
    /// Contexto de la ejecución mientras está en curso; `None` si la fila
    /// está terminada o aún no hay ningún mensaje con uso.
    pub context: Option<Context>,
    /// Posición en el racimo de su agente (`Chain`); `Single` fuera de
    /// `active_groups`.
    pub chain: Chain,
    /// Es una llamada `Agent` sin ejecución enlazada (`lone_call_line`): no
    /// tiene cifras que enseñar (0 trabajado, 0 tokens, sin herramientas ni
    /// skill) y `ended` es el instante de la llamada. En una fila que funde
    /// varias, lo de la representante.
    pub lone_call: bool,
}

/// Identificador estable de una ejecución: su `tool_use_id` si lo tiene; los
/// pasos de workflow no lo llevan, así que se compone con sesión, inicio, tipo
/// y descripción, que no cambian mientras la ejecución avanza.
pub fn run_id(r: &Run) -> String {
    if let Some(id) = r.tool_use_id.as_deref().filter(|i| !i.is_empty()) {
        return id.to_string();
    }
    format!(
        "{}@{}|{}|{}",
        r.session_id.as_deref().unwrap_or(""),
        r.start.map(|s| s.to_rfc3339()).unwrap_or_default(),
        r.agent_type,
        r.description.as_deref().unwrap_or("")
    )
}

/// La ficha de una ejecución, con datos propios: la pantalla la conserva y,
/// si la ejecución termina o desaparece, sigue mostrando la última foto. No
/// depende de `owner`: sirve igual para built-in y workflows sin agente definido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDetail {
    pub id: String,
    pub name: String,
    pub kind: RunKind,
    pub owner: Option<usize>,
    /// El `cwd` real del subagente.
    pub cwd: Option<PathBuf>,
    /// Donde se lanzó la sesión de Claude Code (`Run::launch_dir`,
    /// `Invocation::launch_dir`); `None` si no se sabe.
    pub launch_dir: Option<PathBuf>,
    pub since: Option<DateTime<Utc>>,
    pub worked: TimeDelta,
    pub tools: usize,
    pub task: Option<String>,
    pub task_full: Option<String>,
    pub last_tool: Option<String>,
    pub steps: Vec<ToolStep>,
    /// Todas las invocaciones de skill con su hora (`Run::skills`), de la más
    /// antigua a la más reciente, como `steps`: la sección `skills · N` las
    /// pinta al revés. Vacía si no invocó ninguna, si viene del historial
    /// guardado (`from_cache`) o en una llamada sin ejecución.
    pub skills: Vec<SkillUse>,
    pub tool_counts: Vec<(String, usize)>,
    /// `Some(instante de su última línea)` si ya no está en curso; la pantalla
    /// muestra «terminó hace N min» con él.
    pub ended: Option<DateTime<Utc>>,
    /// El `cwd` temporal de Claude con el que se lanzó la ejecución (`Run::temp_cwd`);
    /// `None` si nunca fue temporal y en las llamadas sin ejecución.
    pub temp_cwd: Option<PathBuf>,
    /// `true` si `temp_cwd` es `Some` y `cwd` ya es otro: se tomó el proyecto
    /// real de la sesión. Con `temp_cwd` `Some` y `false`, sigue siendo el temporal.
    pub cwd_resolved: bool,
    /// Tokens (desglosados), modelo y contexto (`Run::usage`); vacío en una
    /// llamada sin ejecución.
    pub usage: Usage,
    /// `usage.tokens.total()`.
    pub tokens: u64,
    /// Contexto del último mensaje sobre la ventana, en curso o no (`Context::of`).
    pub context: Option<Context>,
    /// Sale solo del historial guardado (`Run::from_cache`): sin pasos.
    pub from_cache: bool,
    /// Quién la lanzó; `None` si no se sabe (sin sesión, o una llamada suelta).
    pub launched_by: Option<LaunchedBy>,
    /// Las ejecuciones que lanzó ella y siguen en los datos, en orden de
    /// lanzamiento (inicio; a igualdad, id).
    pub launched: Vec<RunLink>,
}

/// Otra ejecución vista desde una ficha (su madre o una hija): lo justo para
/// una línea y su `id` para abrir su ficha con `activity_detail`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLink {
    /// `run_id`: el id que abre su ficha (no `Run::id`).
    pub id: String,
    pub name: String,
    pub kind: RunKind,
    pub owner: Option<usize>,
    /// En curso (`Run::in_progress`).
    pub running: bool,
    pub since: Option<DateTime<Utc>>,
    /// Instante de su última línea si ya no está en curso, como `RunDetail::ended`.
    pub ended: Option<DateTime<Utc>>,
    /// Trabajado hasta `now`.
    pub worked: TimeDelta,
    pub tools: usize,
    /// `Tokens::total`.
    pub tokens: u64,
    /// El encargo, como `NowRun::task`.
    pub task: Option<String>,
}

/// Quién lanzó una ejecución.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchedBy {
    /// La sesión directamente: su id entero (`Run::session_id`) y su primer
    /// prompt en una línea (`Run::session_prompt`).
    Session { id: String, prompt: Option<String> },
    /// Otra ejecución que sigue en los datos (la tecla `p` abre su ficha).
    Run(RunLink),
    /// Otra ejecución (`Run::parent_run`, su `Run::id`) que ya no está.
    GoneRun { id: String },
}

impl RunDetail {
    /// Pone al día la ficha con `run_detail`: con `Some` toma la lectura nueva
    /// (que trae `ended` si la ejecución terminó); con `None` (ya no está en
    /// disco) conserva la última foto.
    pub fn refresh(&mut self, new: Option<RunDetail>) {
        if let Some(new) = new {
            *self = new;
        }
    }

    /// La carpeta del grupo, la de `running in`: donde se lanzó la sesión
    /// (`launch_dir`) o, si no se sabe, el `cwd`. La misma regla que agrupa
    /// activos (`NowRun::group_dir`, `AgentBlock::dir`).
    pub fn group_dir(&self) -> Option<&Path> {
        self.launch_dir.as_deref().or(self.cwd.as_deref())
    }
}

/// Localiza una ejecución por su `run_id` (en curso o ya terminada) y devuelve
/// su detalle. `None` si ya no está en disco.
pub fn run_detail(
    agents: &[Agent],
    runs: &[Run],
    id: &str,
    now: DateTime<Utc>,
) -> Option<RunDetail> {
    let r = runs.iter().find(|r| run_id(r) == id)?;
    let n = now_run(agents, r, now);
    let ended = (!r.in_progress(now)).then(|| r.last.unwrap_or(r.modified));
    Some(RunDetail {
        id: n.id,
        name: n.name.to_string(),
        kind: n.kind,
        owner: n.owner,
        cwd: r.cwd.clone(),
        launch_dir: r.launch_dir.clone(),
        since: n.since,
        worked: n.worked,
        tools: n.tools,
        task: n.task.map(str::to_string),
        task_full: r.task_full.clone(),
        last_tool: r.last_tool.clone(),
        steps: r.steps.to_vec(),
        skills: r.skills.to_vec(),
        tool_counts: r.tool_counts.to_vec(),
        ended,
        temp_cwd: r.temp_cwd.clone(),
        cwd_resolved: r.temp_cwd.is_some() && r.temp_cwd != r.cwd,
        usage: r.usage.clone(),
        tokens: r.usage.tokens.total(),
        context: Context::of(&r.usage),
        from_cache: r.from_cache,
        launched_by: launched_by(agents, runs, r, now),
        launched: launched(agents, runs, r, now),
    })
}

/// La línea de otra ejecución en una ficha.
fn run_link(agents: &[Agent], r: &Run, now: DateTime<Utc>) -> RunLink {
    let n = now_run(agents, r, now);
    let running = r.in_progress(now);
    RunLink {
        id: n.id,
        name: n.name.to_string(),
        kind: n.kind,
        owner: n.owner,
        running,
        since: n.since,
        ended: (!running).then(|| r.last.unwrap_or(r.modified)),
        worked: n.worked,
        tools: n.tools,
        tokens: n.tokens,
        task: n.task.map(str::to_string),
    }
}

/// La ejecución de `runs` con ese `Run::id`.
fn by_data_id<'a>(runs: &'a [Run], id: &str) -> Option<&'a Run> {
    runs.iter().find(|r| !r.id.is_empty() && r.id == id)
}

/// Su madre si la lanzó otra ejecución; si no, su sesión.
fn launched_by(agents: &[Agent], runs: &[Run], r: &Run, now: DateTime<Utc>) -> Option<LaunchedBy> {
    if let Some(parent) = &r.parent_run {
        return Some(match by_data_id(runs, parent) {
            Some(p) => LaunchedBy::Run(run_link(agents, p, now)),
            None => LaunchedBy::GoneRun { id: parent.clone() },
        });
    }
    r.session_id.as_ref().map(|id| LaunchedBy::Session {
        id: id.clone(),
        prompt: r.session_prompt.clone(),
    })
}

/// Sus hijas que siguen en los datos, en orden de lanzamiento.
fn launched(agents: &[Agent], runs: &[Run], r: &Run, now: DateTime<Utc>) -> Vec<RunLink> {
    let mut kids: Vec<&Run> = r
        .children
        .iter()
        .filter_map(|c| by_data_id(runs, c))
        .collect();
    kids.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    kids.into_iter().map(|k| run_link(agents, k, now)).collect()
}

/// Quién es de verdad una ejecución: `(nombre, tipo, agente definido)`.
fn identity<'a>(agents: &'a [Agent], r: &'a Run) -> (&'a str, RunKind, Option<usize>) {
    let cwd = run_dir(r);
    if r.agent_type == "workflow-subagent" {
        let desc = r.description.as_deref().filter(|d| !d.is_empty());
        return match desc {
            Some(d) => match owner_of(agents, d, cwd) {
                Some(i) => (agents[i].name.as_str(), RunKind::Workflow, Some(i)),
                None => (d, RunKind::WorkflowStep, None),
            },
            None => ("workflow", RunKind::WorkflowStep, None),
        };
    }
    match owner_of(agents, &r.agent_type, cwd) {
        Some(i) => (agents[i].name.as_str(), RunKind::Defined, Some(i)),
        None => (r.agent_type.as_str(), RunKind::BuiltIn, None),
    }
}

fn now_run<'a>(agents: &'a [Agent], r: &'a Run, now: DateTime<Utc>) -> NowRun<'a> {
    let (name, kind, owner) = identity(agents, r);
    let task = r.task.as_deref().or(match kind {
        // en un workflow la description es el nombre, no el encargo
        RunKind::Workflow | RunKind::WorkflowStep => None,
        RunKind::Defined | RunKind::BuiltIn => r.description.as_deref(),
    });
    NowRun {
        id: run_id(r),
        name,
        kind,
        owner,
        cwd: r.cwd.as_deref(),
        group_dir: run_dir(r),
        since: r.start,
        worked: r.worked_until(now),
        tools: r.tools,
        task,
        last_tool: r.last_tool.as_deref(),
        skill: r.skill.as_deref(),
        task_full: r.task_full.as_deref(),
        steps: &r.steps,
        tool_counts: &r.tool_counts,
        state: RunState::Running,
        runs: 1,
        run_ids: vec![run_id(r)],
        tokens: r.usage.tokens.total(),
        context: Context::of(&r.usage),
        chain: Chain::Single,
        lone_call: false,
    }
}

/// Lo activo de un proyecto: en curso y terminado en la ventana.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveGroup<'a> {
    /// Clave del grupo, como `Row::section`: «global» sin carpeta, o
    /// `NowRun::group_dir` (donde se lanzó la sesión; si no se sabe, el `cwd`)
    /// acortado con `~`. Su nombre corto y la desambiguación entre carpetas
    /// iguales son de la vista (`short_project`/`group_title`).
    pub section: String,
    /// Cuántas filas en curso hay en el grupo: una por ejecución en curso.
    pub running: usize,
    /// Cuántas filas terminadas en la ventana (una por agente):
    /// `running + recent == lines.len()`.
    pub recent: usize,
    /// Un bloque contiguo por agente (`NowRun::chain`). Primero los agentes
    /// con algo en curso (el de inicio más tardío arriba): sus en curso, una
    /// por ejecución y la más tardía primero, y detrás su terminada si la
    /// tiene. Luego los agentes solo con terminada, una fila cada uno, por
    /// `ended` (la más tardía primero).
    pub lines: Vec<NowRun<'a>>,
    /// Suma de `NowRun::tokens` de sus filas.
    pub tokens: u64,
}

/// Tokens de toda la ventana de activos: la suma de sus grupos. Si la pantalla
/// filtra filas, el total de lo que se ve lo suma ella con `NowRun::tokens`.
pub fn window_tokens(groups: &[LiveGroup]) -> u64 {
    groups.iter().map(|g| g.tokens).sum()
}

/// Qué agente es una fila dentro de un proyecto. Un agente definido (lanzado
/// directo o como paso de workflow que resuelve a él) se identifica por su
/// índice en `agents`, no por la etiqueta `vía workflow`; los demás (built-in
/// y pasos de workflow sin agente definido, `WorkflowStep`), por tipo
/// y nombre: `visual [propio]` y `visual [claude]` son filas distintas.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum AgentKey<'a> {
    Defined(usize),
    BuiltIn(&'a str),
    Workflow(&'a str),
}

fn agent_key<'a>(l: &NowRun<'a>) -> AgentKey<'a> {
    match (l.owner, l.kind) {
        (Some(i), _) => AgentKey::Defined(i),
        (None, RunKind::WorkflowStep) => AgentKey::Workflow(l.name),
        (None, _) => AgentKey::BuiltIn(l.name),
    }
}

/// Los ids entre los que salta la ficha (←→) desde la fila que contiene `id`:
/// todas las ejecuciones del bloque de su agente en el grupo (`Chain`), sea
/// `id` una en curso o una terminada. Primero las en curso, en el orden en que
/// salen en el bloque; luego las fundidas en su fila terminada (`run_ids`), de
/// la más reciente a la más antigua. Vacío si ninguna fila lo contiene.
pub fn sibling_ids(groups: &[LiveGroup], id: &str) -> Vec<String> {
    for g in groups {
        let Some(line) = g.lines.iter().find(|l| l.run_ids.iter().any(|r| r == id)) else {
            continue;
        };
        // el bloque ya viene en ese orden: las en curso (cada una su id) y,
        // al final, la terminada con sus fundidas
        let key = agent_key(line);
        return g
            .lines
            .iter()
            .filter(|l| agent_key(l) == key)
            .flat_map(|l| l.run_ids.iter().cloned())
            .collect();
    }
    Vec::new()
}

/// El bloque de un agente en activos «desplegado»: lo que pinta la pantalla
/// del agente. Las mismas ejecuciones que su bloque en `active_groups`, pero
/// UNA POR FILA: las terminadas no se funden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentBlock<'a> {
    /// Nombre resuelto del agente (`NowRun::name` de la primera fila del bloque).
    pub name: &'a str,
    /// Origen de la primera fila del bloque, el mismo que rotula el bloque en
    /// activos. Un agente definido puede mezclar `Defined` y `Workflow`: cada
    /// ejecución lleva el suyo en `runs`.
    pub kind: RunKind,
    /// Índice del agente definido; `None` en built-in y pasos de workflow sin
    /// agente definido.
    pub owner: Option<usize>,
    /// Ruta absoluta del `.md` que lo define (`Agent::path`); `None` sin `owner`.
    pub definition: Option<&'a Path>,
    /// Clave del grupo (`LiveGroup::section`): «global» o la carpeta con `~`.
    pub section: String,
    /// Carpeta del grupo sin acortar (`NowRun::group_dir`); `None` en «global».
    pub dir: Option<&'a Path>,
    /// Cuántas de `runs` están en curso.
    pub running: usize,
    /// Cuántas de `runs` terminaron en la ventana (llamadas sin ejecución
    /// incluidas): `running + finished == runs.len()`.
    pub finished: usize,
    /// Suma de `NowRun::tokens` de `runs`: lo mismo que suman las filas del
    /// bloque en activos.
    pub tokens: u64,
    /// Sus ejecuciones, una por fila y en el orden de `sibling_ids`: primero
    /// las en curso como salen en el bloque (la de inicio más tardío arriba) y
    /// luego las terminadas en la ventana, de la más reciente a la más
    /// antigua. Cada una con `runs == 1`, `run_ids == [id]`, sus propias
    /// cifras y `chain == Single` (aquí no significa nada). Las terminadas
    /// llevan `state: Done { ended }` y `context: None`.
    ///
    /// Una llamada sin ejecución enlazada sale como una terminada más, en su
    /// sitio por la hora de la llamada, con `lone_call: true` y sin cifras
    /// (0 trabajado, 0 tokens, sin herramienta ni skill): cuenta en
    /// `finished` y en el total, y su `id` (`call_id`) abre la ficha mínima
    /// de `activity_detail`.
    pub runs: Vec<NowRun<'a>>,
}

/// El bloque desplegado del agente al que pertenece `id` (el de cualquiera de
/// sus ejecuciones; el de su primera fila sirve de id del bloque). `groups`
/// son los de `active_groups` con estos mismos `agents`, `calls`, `runs` y
/// `now`. `None` si ninguna fila de la ventana contiene `id`: el bloque ya no
/// está en activos.
pub fn agent_block<'a>(
    agents: &'a [Agent],
    calls: &'a [Invocation],
    runs: &'a [Run],
    groups: &[LiveGroup<'a>],
    id: &str,
    now: DateTime<Utc>,
) -> Option<AgentBlock<'a>> {
    let has = |l: &NowRun| l.run_ids.iter().any(|r| r == id);
    let (g, line) = groups
        .iter()
        .find_map(|g| g.lines.iter().find(|l| has(l)).map(|l| (g, l)))?;
    let key = agent_key(line);
    let block: Vec<&NowRun<'a>> = g.lines.iter().filter(|l| agent_key(l) == key).collect();
    let head = block[0];

    // las fundidas en una fila terminada, cada una con sus datos; solo se
    // buscan si hay alguna fila que funda. Por id puede haber más de una (dos
    // ejecuciones con el mismo `tool_use_id`, dos llamadas antiguas con el
    // mismo id compuesto): se guardan todas, para que cada id de `run_ids`
    // (que las cuenta una a una) encuentre la suya
    let fused: Vec<&str> = block
        .iter()
        .filter(|l| l.run_ids.len() > 1)
        .flat_map(|l| l.run_ids.iter().map(String::as_str))
        .collect();
    let mut apart: HashMap<String, Vec<(DateTime<Utc>, NowRun<'a>)>> = HashMap::new();
    if !fused.is_empty() {
        for r in runs {
            // las en curso son su propia fila, nunca una fundida
            if r.in_progress(now) {
                continue;
            }
            let rid = run_id(r);
            if fused.contains(&rid.as_str()) {
                if let Some(ended) = r.last.or(r.start) {
                    let row = NowRun {
                        state: RunState::Done { ended },
                        context: None,
                        ..now_run(agents, r, now)
                    };
                    apart.entry(rid).or_default().push((ended, row));
                }
            }
        }
        for c in calls {
            let cid = call_id(c);
            if fused.contains(&cid.as_str()) && run_of(c, runs).is_none() {
                apart
                    .entry(cid)
                    .or_default()
                    .push((c.at, lone_call_line(agents, c)));
            }
        }
        // de la más antigua a la más reciente: `pop` saca primero la más
        // reciente, como las fundió `active_groups`, y las que comparten id
        // pero quedaron fuera de la ventana (más antiguas) no se llegan a sacar
        for same in apart.values_mut() {
            same.sort_by_key(|(ended, _)| *ended);
        }
    }

    let mut rows: Vec<NowRun<'a>> = Vec::new();
    for l in block {
        if l.run_ids.len() <= 1 {
            rows.push(NowRun {
                chain: Chain::Single,
                ..l.clone()
            });
        } else {
            rows.extend(
                l.run_ids
                    .iter()
                    .filter_map(|rid| apart.get_mut(rid).and_then(Vec::pop).map(|(_, row)| row)),
            );
        }
    }
    let running = rows.iter().filter(|r| r.state == RunState::Running).count();
    Some(AgentBlock {
        name: head.name,
        kind: head.kind,
        owner: head.owner,
        definition: head.owner.map(|i| agents[i].path.as_path()),
        section: g.section.clone(),
        dir: head.group_dir,
        running,
        finished: rows.len() - running,
        tokens: rows.iter().map(|r| r.tokens).sum(),
        runs: rows,
    })
}

/// Clave de grupo de una línea (`NowRun::group_dir`): como `Row::section`.
fn section_of(dir: Option<&Path>, home: &Path) -> String {
    dir.map_or_else(|| "global".to_string(), |c| crate::report::shorten(c, home))
}

/// Identificador estable de una llamada sin ejecución: su `id` si lo tiene; las
/// antiguas, sin `id`, se componen con instante, agente y `cwd`.
pub fn call_id(c: &Invocation) -> String {
    match c.id.as_deref().filter(|i| !i.is_empty()) {
        Some(id) => id.to_string(),
        None => format!("call@{}|{}|{}", c.at.to_rfc3339(), c.agent, c.cwd.display()),
    }
}

/// La línea de una llamada sin ejecución enlazada: sin datos de ejecución
/// (duración 0, sin herramientas ni pasos), terminada en el instante de la
/// llamada. El encargo es la primera línea del prompt o, si no hay, la descripción.
fn lone_call_line<'a>(agents: &'a [Agent], c: &'a Invocation) -> NowRun<'a> {
    let owner = owner_of(agents, &c.agent, Some(call_dir(c)));
    let (name, kind) = match owner {
        Some(i) => (agents[i].name.as_str(), RunKind::Defined),
        None => (c.agent.as_str(), RunKind::BuiltIn),
    };
    NowRun {
        id: call_id(c),
        name,
        kind,
        owner,
        cwd: Some(&c.cwd),
        group_dir: Some(call_dir(c)),
        since: Some(c.at),
        worked: TimeDelta::zero(),
        tools: 0,
        task: c.prompt_line.as_deref().or(c.description.as_deref()),
        last_tool: None,
        skill: None,
        task_full: None,
        steps: &[],
        tool_counts: &[],
        state: RunState::Done { ended: c.at },
        runs: 1,
        run_ids: vec![call_id(c)],
        tokens: 0,
        context: None,
        chain: Chain::Single,
        lone_call: true,
    }
}

/// La vista de activos: un grupo por proyecto con lo en curso y, debajo, lo
/// terminado dentro de la ventana (desde `since`). Entran todas las clases
/// (definidas, built-in, workflow) y las llamadas `Agent` de la ventana sin
/// ejecución enlazada (`run_of`); una llamada y su ejecución son una sola línea.
///
/// Cada ejecución en curso es su propia fila, nunca fundida; las terminadas
/// (y llamadas sin ejecución) de un mismo agente en un proyecto se funden en
/// una fila. Un agente con ejecuciones en curso y terminadas tiene ambas en
/// el mismo bloque: la terminada cierra su racimo.
///
/// Orden garantizado dentro de cada grupo, un bloque contiguo por agente
/// (`Chain`): primero los agentes con algo en curso, por el inicio más tardío
/// de sus ejecuciones; dentro, sus en curso por inicio (la más tardía
/// primero; a igualdad, por id) y al final su terminada. Luego los agentes
/// solo con terminada, por `ended` (la más tardía primero). Los grupos,
/// por su actividad más reciente (inicio de la en curso más nueva o `ended` de
/// la terminada más nueva); a igualdad, por `section`.
pub fn active_groups<'a>(
    agents: &'a [Agent],
    calls: &'a [Invocation],
    runs: &'a [Run],
    since: DateTime<Utc>,
    now: DateTime<Utc>,
    home: &Path,
) -> Vec<LiveGroup<'a>> {
    let mut live: Vec<NowRun<'a>> = Vec::new();
    let mut done: Vec<NowRun<'a>> = Vec::new();
    for r in runs {
        if r.in_progress(now) {
            live.push(now_run(agents, r, now));
        } else if let Some(ended) = r.last.or(r.start).filter(|at| *at >= since) {
            done.push(NowRun {
                state: RunState::Done { ended },
                context: None,
                ..now_run(agents, r, now)
            });
        }
    }
    done.extend(
        calls
            .iter()
            .filter(|c| c.at >= since && run_of(c, runs).is_none())
            .map(|c| lone_call_line(agents, c)),
    );
    let ended_of = |l: &NowRun| match l.state {
        RunState::Done { ended } => Some(ended),
        RunState::Running => l.since,
    };

    // por proyecto: racimos en curso por agente, filas terminadas y la
    // actividad más reciente
    struct Acc<'a> {
        clusters: Vec<(AgentKey<'a>, Vec<NowRun<'a>>)>,
        done: Vec<NowRun<'a>>,
        latest: Option<DateTime<Utc>>,
    }
    /// El índice del proyecto `section` en `acc`, creándolo si no está.
    fn slot(acc: &mut Vec<(String, Acc<'_>)>, section: String) -> usize {
        match acc.iter().position(|(s, _)| *s == section) {
            Some(i) => i,
            None => {
                acc.push((
                    section,
                    Acc {
                        clusters: Vec::new(),
                        done: Vec::new(),
                        latest: None,
                    },
                ));
                acc.len() - 1
            }
        }
    }
    let mut acc: Vec<(String, Acc<'a>)> = Vec::new();

    for line in live {
        let gi = slot(&mut acc, section_of(line.group_dir, home));
        let a = &mut acc[gi].1;
        a.latest = a.latest.max(line.since);
        let key = agent_key(&line);
        match a.clusters.iter_mut().find(|(k, _)| *k == key) {
            Some((_, c)) => c.push(line),
            None => a.clusters.push((key, vec![line])),
        }
    }

    // las terminadas, de la más reciente a la más antigua: la primera de cada
    // agente representa a la fila y las demás solo suman a `runs`/`run_ids`
    done.sort_by_key(|l| std::cmp::Reverse(ended_of(l)));
    for line in done {
        let gi = slot(&mut acc, section_of(line.group_dir, home));
        let a = &mut acc[gi].1;
        a.latest = a.latest.max(ended_of(&line));
        let key = agent_key(&line);
        match a.done.iter_mut().find(|row| agent_key(row) == key) {
            Some(row) => {
                row.runs += 1;
                row.tokens += line.tokens;
                row.run_ids.extend(line.run_ids);
            }
            None => a.done.push(line),
        }
    }

    let mut groups: Vec<(LiveGroup<'a>, Option<DateTime<Utc>>)> = acc
        .into_iter()
        .map(|(section, mut a)| {
            for (_, c) in &mut a.clusters {
                c.sort_by(|x, y| y.since.cmp(&x.since).then_with(|| x.id.cmp(&y.id)));
            }
            // tras ordenar cada racimo, su primera es la de inicio más tardío
            a.clusters.sort_by(|(_, x), (_, y)| {
                y[0].since
                    .cmp(&x[0].since)
                    .then_with(|| x[0].id.cmp(&y[0].id))
            });
            // cada racimo en curso se lleva detrás la terminada de su agente;
            // las que quedan (agentes solo con terminadas) van después, cada
            // una su bloque, en el orden de `ended` que ya traen
            let recent = a.done.len();
            let mut done = a.done;
            let mut blocks: Vec<Vec<NowRun<'a>>> = Vec::new();
            for (key, mut c) in a.clusters {
                if let Some(i) = done.iter().position(|row| agent_key(row) == key) {
                    c.push(done.remove(i));
                }
                blocks.push(c);
            }
            blocks.extend(done.into_iter().map(|row| vec![row]));
            let mut lines: Vec<NowRun<'a>> = Vec::new();
            for c in blocks {
                let n = c.len();
                for (i, mut l) in c.into_iter().enumerate() {
                    l.chain = match (n, i) {
                        (1, _) => Chain::Single,
                        (_, 0) => Chain::Head(n),
                        (_, i) if i + 1 == n => Chain::Last,
                        _ => Chain::Middle,
                    };
                    lines.push(l);
                }
            }
            let running = lines.len() - recent;
            let tokens = lines.iter().map(|l| l.tokens).sum();
            (
                LiveGroup {
                    section,
                    running,
                    recent,
                    lines,
                    tokens,
                },
                a.latest,
            )
        })
        .collect();
    groups.sort_by(|(a, la), (b, lb)| lb.cmp(la).then_with(|| a.section.cmp(&b.section)));
    groups.into_iter().map(|(g, _)| g).collect()
}

/// Como `run_detail`, pero también resuelve el id de una llamada sin
/// ejecución (`call_id`): ficha mínima, sin pasos, herramientas ni encargo, y
/// con `ended` en el instante de la llamada. `None` si no hay ni ejecución ni
/// llamada con ese id.
pub fn activity_detail(
    agents: &[Agent],
    calls: &[Invocation],
    runs: &[Run],
    id: &str,
    now: DateTime<Utc>,
) -> Option<RunDetail> {
    if let Some(d) = run_detail(agents, runs, id, now) {
        return Some(d);
    }
    let c = calls
        .iter()
        .find(|c| call_id(c) == id && run_of(c, runs).is_none())?;
    let n = lone_call_line(agents, c);
    Some(RunDetail {
        id: n.id,
        name: n.name.to_string(),
        kind: n.kind,
        owner: n.owner,
        cwd: Some(c.cwd.clone()),
        launch_dir: c.launch_dir.clone(),
        since: n.since,
        worked: n.worked,
        tools: 0,
        task: None,
        task_full: None,
        last_tool: None,
        steps: Vec::new(),
        skills: Vec::new(),
        tool_counts: Vec::new(),
        ended: Some(c.at),
        temp_cwd: None,
        cwd_resolved: false,
        usage: Usage::default(),
        tokens: 0,
        context: None,
        from_cache: false,
        launched_by: None,
        launched: Vec::new(),
    })
}

/// El agente definido al que pertenece un nombre lanzado desde `dir` (la
/// carpeta de lanzamiento de la sesión o, sin ella, el `cwd`): si
/// encajan varios (uno global y otro de proyecto con el mismo nombre), el más
/// específico, el del proyecto más hondo; a igualdad, el primero.
pub fn owner_of(agents: &[Agent], name: &str, dir: Option<&Path>) -> Option<usize> {
    agents
        .iter()
        .enumerate()
        .filter(|(_, a)| belongs_to(a, name, dir))
        .rev()
        .max_by_key(|(_, a)| match &a.scope {
            Scope::Global => 0,
            Scope::Project(dir) => dir.components().count() + 1,
        })
        .map(|(i, _)| i)
}

/// El último uso de cualquier agente definido (`last_use`): su índice y cuándo.
/// Para el monitor vacío: «último uso: <agente> de <proyecto> hace X».
pub fn last_use_overall(
    agents: &[Agent],
    calls: &[Invocation],
    runs: &[Run],
) -> Option<(usize, DateTime<Utc>)> {
    agents
        .iter()
        .enumerate()
        .filter_map(|(i, a)| last_use(a, calls, runs).map(|at| (i, at)))
        .max_by_key(|(_, at)| *at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::usage::Tokens;
    use crate::runs::{Run, SkillUse, ToolStep};
    use chrono::{DurationRound, FixedOffset, TimeDelta};
    use std::path::PathBuf;

    fn agent(name: &str, scope: Scope) -> Agent {
        Agent {
            name: name.into(),
            path: PathBuf::from("/x.md"),
            scope,
            description: None,
        }
    }

    fn call(agent: &str, cwd: &str, at: &str) -> Invocation {
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

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    /// Zona de desfase fijo `h` horas, para los tests de días locales.
    fn tz(h: i32) -> FixedOffset {
        FixedOffset::east_opt(h * 3600).unwrap()
    }

    fn mins(n: i64) -> TimeDelta {
        TimeDelta::minutes(n)
    }

    /// La ejecución con su trabajo repartido en franjas: `(inicio de la franja, minutos)`.
    fn with_slots(r: Run, slots: &[(&str, i64)]) -> Run {
        Run {
            worked: slots.iter().map(|(_, m)| mins(*m)).sum(),
            worked_slots: slots.iter().map(|(s, m)| (t(s), mins(*m))).collect(),
            ..r
        }
    }

    #[test]
    fn days_of_spreads_work_over_local_days_oldest_first() {
        let a = agent("docs", Scope::Global);
        // +01:00: son las 13:00 del 5 de noviembre
        let now = t("2026-11-05T12:00:00Z");
        let all = [
            run("docs", Some("/p"), "2026-11-05T09:00:00Z", 50),
            // 00:30 local del día 5, aunque en UTC sea el 4
            run("docs", Some("/p"), "2026-11-04T23:30:00Z", 20),
            // hace 13 días: la primera columna
            run("docs", Some("/p"), "2026-10-23T10:00:00Z", 7),
            // hace 14 días: fuera
            run("docs", Some("/p"), "2026-10-22T10:00:00Z", 99),
            run("backend", Some("/p"), "2026-11-05T09:00:00Z", 99),
        ];
        let days = days_of(&a, &all, now, &tz(1), 14);
        assert_eq!(days.len(), 14);
        assert_eq!(days[0], mins(7));
        assert_eq!(days[13], mins(70));
        assert_eq!(days[1..13].iter().copied().sum::<TimeDelta>(), mins(0));
        // en UTC, la de las 23:30 cae el día 4
        let days = days_of(&a, &all, now, &tz(0), 14);
        assert_eq!((days[12], days[13]), (mins(20), mins(50)));
    }

    #[test]
    fn days_of_counts_a_running_run_until_now() {
        let a = agent("docs", Scope::Global);
        // empezó a las 11:50, última línea a las 11:57; son las 11:59
        let all = [running("docs", "2026-11-05T11:50:00Z", 7)];
        let days = days_of(&a, &all, t("2026-11-05T11:59:00Z"), &tz(0), 3);
        assert_eq!(days, vec![mins(0), mins(0), mins(9)]);
    }

    #[test]
    fn days_of_without_runs_is_all_zero() {
        let a = agent("docs", Scope::Global);
        let days = days_of(&a, &[], t("2026-11-05T12:00:00Z"), &tz(1), 14);
        assert_eq!(days, vec![TimeDelta::zero(); 14]);
    }

    #[test]
    fn period_cycles_week_month_ever_and_starts_at_ever() {
        assert_eq!(Period::default(), Period::Ever);
        assert_eq!(Period::Ever.next(), Period::Week);
        assert_eq!(Period::Week.next(), Period::Month);
        assert_eq!(Period::Month.next(), Period::Ever);
        assert_eq!(Period::Week.days(), Some(7));
        assert_eq!(Period::Month.days(), Some(30));
        assert_eq!(Period::Ever.days(), None);
    }

    #[test]
    fn work_in_period_adds_the_days_inside_and_the_runs_started_inside() {
        let a = agent("docs", Scope::Global);
        // +01:00: hoy es el 5; 7 días van del 30 oct al 5 nov
        let now = t("2026-11-05T12:00:00Z");
        let straddling = with_slots(
            run("docs", Some("/p"), "2026-10-29T22:45:00Z", 0),
            // 23:45 del 29 local y 00:00 del 30 local
            &[("2026-10-29T22:45:00Z", 10), ("2026-10-29T23:00:00Z", 4)],
        );
        let all = [
            run("docs", Some("/p"), "2026-11-05T09:00:00Z", 50),
            straddling,
            run("docs", Some("/p"), "2026-10-10T09:00:00Z", 30),
            run("docs", Some("/p"), "2026-09-01T09:00:00Z", 100),
            run("backend", Some("/p"), "2026-11-05T09:00:00Z", 99),
        ];
        let week = work_in(&a, &all, now, &tz(1), Period::Week);
        // del que cruza solo cuentan los 4 min del día 30; empezó el 29, fuera
        assert_eq!(
            week,
            PeriodWork {
                worked: mins(54),
                runs: 1
            }
        );
        let month = work_in(&a, &all, now, &tz(1), Period::Month);
        assert_eq!(
            month,
            PeriodWork {
                worked: mins(94),
                runs: 3
            }
        );
        let ever = work_in(&a, &all, now, &tz(1), Period::Ever);
        assert_eq!(
            ever,
            PeriodWork {
                worked: mins(194),
                runs: 4
            }
        );
    }

    #[test]
    fn work_in_period_counts_a_running_run_until_now() {
        let a = agent("docs", Scope::Global);
        let all = [running("docs", "2026-11-05T11:50:00Z", 7)];
        let now = t("2026-11-05T11:59:00Z");
        for p in [Period::Week, Period::Month, Period::Ever] {
            assert_eq!(
                work_in(&a, &all, now, &tz(0), p),
                PeriodWork {
                    worked: mins(9),
                    runs: 1
                }
            );
        }
    }

    #[test]
    fn group_total_adds_the_work_of_its_agents() {
        let works = [
            Work {
                total: mins(30),
                runs: 2,
                tokens: 0,
                running: None,
            },
            Work::default(),
            Work {
                total: mins(12),
                runs: 1,
                tokens: 0,
                running: None,
            },
        ];
        assert_eq!(total_worked(&works), mins(42));
        assert_eq!(total_worked(&[]), TimeDelta::zero());
    }

    #[test]
    fn ever_used_with_a_call_or_a_run_of_its_own() {
        let project = agent("docs", Scope::Project(PathBuf::from("/Users/x/app")));
        let call_in = [call("docs", "/Users/x/app", "2020-01-01T10:00:00Z")];
        let run_in = [run(
            "docs",
            Some("/Users/x/app/front"),
            "2020-01-01T10:00:00Z",
            0,
        )];
        let elsewhere_call = [call("docs", "/Users/x/otro", "2026-11-05T10:00:00Z")];
        let elsewhere_run = [run(
            "docs",
            Some("/Users/x/otro"),
            "2026-11-05T10:00:00Z",
            3,
        )];
        assert!(ever_used(&project, &call_in, &[]));
        assert!(ever_used(&project, &[], &run_in));
        assert!(!ever_used(&project, &elsewhere_call, &elsewhere_run));
        assert!(!ever_used(&project, &[], &[]));
    }

    #[test]
    fn running_now_lists_every_run_in_progress_newest_first() {
        let now = t("2026-11-05T11:59:00Z");
        let mut wf = running("workflow-subagent", "2026-11-05T11:55:00Z", 3);
        wf.description = Some("aplica el plan".into());
        wf.tools = 4;
        wf.session_id = Some("s".into());
        wf.task = Some("pintar".into());
        wf.last_tool = Some("Edit · README.md".into());
        wf.skill = Some("superpowers:brainstorming".into());
        let all = [
            running("general-purpose", "2026-11-05T11:40:00Z", 15),
            run("docs", Some("/p"), "2026-11-05T11:50:00Z", 8),
            wf,
            // sin tocar desde hace más de 10 min: interrumpida
            running("docs", "2026-11-05T11:00:00Z", 5),
        ];
        let got = running_now(&[], &all, now);
        let names: Vec<_> = got.iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["aplica el plan", "general-purpose"]);
        assert_eq!(
            got[0],
            NowRun {
                id: "s@2026-11-05T11:55:00+00:00|workflow-subagent|aplica el plan".into(),
                name: "aplica el plan",
                kind: RunKind::WorkflowStep,
                owner: None,
                cwd: Some(Path::new("/p")),
                group_dir: Some(Path::new("/p")),
                since: Some(t("2026-11-05T11:55:00Z")),
                worked: mins(4),
                tools: 4,
                task: Some("pintar"),
                last_tool: Some("Edit · README.md"),
                skill: Some("superpowers:brainstorming"),
                task_full: None,
                steps: &[],
                tool_counts: &[],
                state: RunState::Running,
                runs: 1,
                run_ids: vec![
                    "s@2026-11-05T11:55:00+00:00|workflow-subagent|aplica el plan".into()
                ],
                tokens: 0,
                context: None,
                chain: Chain::Single,
                lone_call: false,
            }
        );
        // 15 min hasta su última línea a las 11:55 y 4 más hasta ahora
        assert_eq!(got[1].worked, mins(19));
        assert!(running_now(&[], &all, t("2026-11-05T13:00:00Z")).is_empty());
    }

    #[test]
    fn run_id_is_the_tool_use_id_or_a_stable_composite() {
        let mut r = running("docs", "2026-11-05T11:55:00Z", 3);
        r.tool_use_id = Some("toolu_9".into());
        assert_eq!(run_id(&r), "toolu_9");
        let mut wf = running("workflow-subagent", "2026-11-05T11:55:00Z", 3);
        wf.description = Some("x".into());
        let before = run_id(&wf);
        // avanzar la ejecución no cambia el id
        wf.last = wf.last.map(|l| l + mins(5));
        wf.tools += 3;
        assert_eq!(run_id(&wf), before);
        // otra con otro inicio, otro id
        let other = running("workflow-subagent", "2026-11-05T11:56:00Z", 3);
        assert_ne!(run_id(&other), before);
    }

    #[test]
    fn run_detail_follows_the_same_run_whatever_the_order_and_owner() {
        let now = t("2026-11-05T11:59:00Z");
        let mut a = running("general-purpose", "2026-11-05T11:50:00Z", 8);
        a.tool_use_id = Some("toolu_a".into());
        a.task_full = Some("línea 1\nlínea 2".into());
        a.tool_counts = vec![("Bash".to_string(), 2), ("Edit".to_string(), 1)].into();
        a.steps = vec![ToolStep {
            at: t("2026-11-05T11:57:00Z"),
            name: "Bash".into(),
            arg: Some("ls".into()),
        }]
        .into();
        let mut b = running("docs", "2026-11-05T11:55:00Z", 3);
        b.tool_use_id = Some("toolu_b".into());
        let d = run_detail(&[], &[b.clone(), a.clone()], "toolu_a", now).unwrap();
        // otro orden, otra ejecución nueva: la misma ficha
        let c = running("docs", "2026-11-05T11:58:00Z", 1);
        let d2 = run_detail(&[], &[c, a, b], "toolu_a", now).unwrap();
        assert_eq!(d, d2);
        assert_eq!(d.name, "general-purpose");
        assert_eq!(d.owner, None);
        assert_eq!(d.kind, RunKind::BuiltIn);
        assert_eq!(d.task_full.as_deref(), Some("línea 1\nlínea 2"));
        assert_eq!(d.steps.len(), 1);
        assert_eq!(d.tool_counts[0], ("Bash".to_string(), 2));
        assert_eq!(d.ended, None);
        assert!(run_detail(&[], &[], "toolu_a", now).is_none());
    }

    #[test]
    fn run_detail_of_a_finished_run_says_when_it_ended() {
        let mut r = run("docs", Some("/p"), "2026-11-05T11:00:00Z", 10);
        r.tool_use_id = Some("toolu_z".into());
        let now = t("2026-11-05T11:30:00Z");
        let d = run_detail(&[], &[r], "toolu_z", now).unwrap();
        assert_eq!(d.ended, Some(t("2026-11-05T11:10:00Z")));
    }

    #[test]
    fn refresh_replaces_the_detail_and_keeps_it_when_the_run_is_gone() {
        let now = t("2026-11-05T11:59:00Z");
        let mut r = running("docs", "2026-11-05T11:55:00Z", 3);
        r.tool_use_id = Some("toolu_r".into());
        r.tools = 5;
        let mut d = run_detail(&[], std::slice::from_ref(&r), "toolu_r", now).unwrap();
        r.tools = 6;
        d.refresh(run_detail(&[], &[r], "toolu_r", now));
        assert_eq!(d.tools, 6);
        d.refresh(run_detail(&[], &[], "toolu_r", now));
        assert_eq!(d.tools, 6);
    }

    #[test]
    fn a_workflow_step_resolves_to_the_defined_agent_in_its_description() {
        let agents = defined();
        let now = t("2026-11-05T11:59:00Z");
        let mut wf = running("workflow-subagent", "2026-11-05T11:55:00Z", 3);
        wf.description = Some("docs".into());
        let mut other = running("workflow-subagent", "2026-11-05T11:56:00Z", 3);
        other.description = Some("revisor".into());
        let bare = running("workflow-subagent", "2026-11-05T11:57:00Z", 3);
        let all = [wf, other, bare];
        let got = running_now(&agents, &all, now);
        assert_eq!(
            (got[2].name, got[2].kind, got[2].owner),
            ("docs", RunKind::Workflow, Some(0))
        );
        assert_eq!(
            (got[1].name, got[1].kind, got[1].owner),
            ("revisor", RunKind::WorkflowStep, None)
        );
        assert_eq!(
            (got[0].name, got[0].kind, got[0].owner),
            ("workflow", RunKind::WorkflowStep, None)
        );
        // la description de un workflow es el nombre, nunca el encargo
        assert!(got.iter().all(|r| r.task.is_none()));
    }

    #[test]
    fn built_in_and_defined_runs_are_told_apart() {
        let agents = defined();
        let now = t("2026-11-05T11:59:00Z");
        let mut gp = running("general-purpose", "2026-11-05T11:50:00Z", 3);
        gp.description = Some("Revisión".into());
        let mut d = running("docs", "2026-11-05T11:51:00Z", 3);
        d.description = Some("pon al día".into());
        d.task = Some("el encargo".into());
        // agente de proyecto lanzado fuera del proyecto: no es el definido
        let outside = running("backend", "2026-11-05T11:52:00Z", 3);
        let all = [gp, d, outside];
        let got = running_now(&agents, &all, now);
        assert_eq!(
            (got[2].kind, got[2].owner, got[2].task),
            (RunKind::BuiltIn, None, Some("Revisión"))
        );
        assert_eq!(
            (got[1].kind, got[1].owner, got[1].task),
            (RunKind::Defined, Some(0), Some("el encargo"))
        );
        assert_eq!(got[0].kind, RunKind::BuiltIn);
    }

    /// Las ejecuciones en curso, la que empezó más tarde primero.
    fn running_now<'a>(
        agents: &'a [Agent],
        runs: &'a [Run],
        now: DateTime<Utc>,
    ) -> Vec<NowRun<'a>> {
        let mut found: Vec<&Run> = runs.iter().filter(|r| r.in_progress(now)).collect();
        found.sort_by_key(|r| std::cmp::Reverse(r.start));
        found.into_iter().map(|r| now_run(agents, r, now)).collect()
    }

    /// Agentes definidos de los tests de la ventana: 0 global, 1 de proyecto.
    fn defined() -> Vec<Agent> {
        vec![
            agent("docs", Scope::Global),
            agent("backend", Scope::Project(PathBuf::from("/Users/x/app"))),
        ]
    }

    fn home() -> &'static Path {
        Path::new("/Users/x")
    }

    #[test]
    fn active_groups_put_running_first_then_finished_per_project() {
        let agents = defined();
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let mut live_old = running("docs", "2026-11-05T11:50:00Z", 5);
        live_old.cwd = Some("/Users/x/app".into());
        let mut live_new = running("general-purpose", "2026-11-05T11:55:00Z", 3);
        live_new.cwd = Some("/Users/x/app".into());
        let mut done_old = run("docs", Some("/Users/x/app"), "2026-11-05T11:10:00Z", 5);
        done_old.tool_use_id = Some("toolu_old".into());
        let done_new = run("Explore", Some("/Users/x/app"), "2026-11-05T11:20:00Z", 10);
        let mut other = run("docs", Some("/Users/x/otro"), "2026-11-05T11:45:00Z", 8);
        other.description = Some("revisa".into());
        let before = run("docs", Some("/Users/x/app"), "2026-11-05T10:00:00Z", 30);
        let runs = [done_old, live_old, other, done_new, live_new, before];
        let groups = active_groups(&agents, &[], &runs, since, now, home());
        // el grupo con la actividad más reciente (la en curso de las 11:55) primero
        let got: Vec<_> = groups
            .iter()
            .map(|g| (g.section.as_str(), g.running, g.recent))
            .collect();
        // el `docs` en curso y el terminado de ~/app son dos filas
        assert_eq!(got, vec![("~/app", 2, 2), ("~/otro", 0, 1)]);
        let lines: Vec<_> = groups[0]
            .lines
            .iter()
            .map(|l| (l.name, l.state, l.runs))
            .collect();
        // la terminada de `docs` va pegada a su en curso, aunque `Explore`
        // terminase más tarde
        assert_eq!(
            lines,
            vec![
                ("general-purpose", RunState::Running, 1),
                ("docs", RunState::Running, 1),
                (
                    "docs",
                    RunState::Done {
                        ended: t("2026-11-05T11:15:00Z")
                    },
                    1
                ),
                (
                    "Explore",
                    RunState::Done {
                        ended: t("2026-11-05T11:30:00Z")
                    },
                    1
                ),
            ]
        );
        assert_eq!(groups[0].lines[2].run_ids, vec!["toolu_old"]);
        assert_eq!(groups[0].lines[3].kind, RunKind::BuiltIn);
        assert_eq!(groups[1].lines[0].task, Some("revisa"));
    }

    #[test]
    fn groups_are_ordered_by_their_latest_activity() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        // a: una en curso desde las 11:40; b: una terminada a las 11:50
        let mut a = running("docs", "2026-11-05T11:40:00Z", 1);
        a.cwd = Some("/Users/x/a".into());
        let b = run("docs", Some("/Users/x/b"), "2026-11-05T11:45:00Z", 5);
        let runs = [a, b];
        let groups = active_groups(&[], &[], &runs, since, now, home());
        let names: Vec<_> = groups.iter().map(|g| g.section.as_str()).collect();
        assert_eq!(names, vec!["~/b", "~/a"]);
    }

    #[test]
    fn finished_workflow_and_built_in_runs_enter_the_window() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let mut wf = run("workflow-subagent", Some("/p"), "2026-11-05T11:10:00Z", 4);
        wf.description = Some("docs".into());
        let gp = run("general-purpose", Some("/p"), "2026-11-05T11:20:00Z", 4);
        // terminó antes de la ventana
        let old = run("docs", Some("/p"), "2026-11-05T10:00:00Z", 30);
        let agents = defined();
        let runs = [wf, gp, old];
        let groups = active_groups(&agents, &[], &runs, since, now, home());
        assert_eq!(groups.len(), 1);
        let l = &groups[0].lines;
        assert_eq!(
            (l[0].kind, l[0].owner, l[1].kind, l[1].owner),
            (RunKind::BuiltIn, None, RunKind::Workflow, Some(0))
        );
        assert_eq!((groups[0].running, groups[0].recent), (0, 2));
    }

    #[test]
    fn a_call_and_its_run_are_one_line_and_a_lone_call_joins_its_project() {
        let agents = defined();
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let mut linked = call("docs", "/Users/x/app", "2026-11-05T11:30:00Z");
        linked.id = Some("toolu_1".into());
        let mut r = run("docs", Some("/Users/x/app"), "2026-11-05T11:30:00Z", 12);
        r.tool_use_id = Some("toolu_1".into());
        let mut lost = call("backend", "/Users/x/app", "2026-11-05T11:20:00Z");
        lost.id = Some("toolu_perdido".into());
        // llamada de una en curso: no es una línea aparte
        let mut live_call = call("docs", "/Users/x/app", "2026-11-05T11:50:00Z");
        live_call.id = Some("toolu_vivo".into());
        let mut live = running("docs", "2026-11-05T11:50:00Z", 7);
        live.cwd = Some("/Users/x/app".into());
        live.tool_use_id = Some("toolu_vivo".into());
        let calls = [linked, lost, live_call];
        let runs = [r, live];
        let groups = active_groups(&agents, &calls, &runs, since, now, home());
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        // la en curso arriba; la terminada de `docs` (11:42) y la llamada suelta debajo
        assert_eq!((g.running, g.recent, g.lines.len()), (1, 2, 3));
        assert_eq!((g.lines[0].runs, g.lines[0].name), (1, "docs"));
        assert_eq!(g.lines[1].run_ids, vec!["toolu_1"]);
        let lone = &g.lines[2];
        assert_eq!(lone.id, "toolu_perdido");
        assert_eq!(
            (lone.name, lone.kind, lone.owner),
            ("backend", RunKind::Defined, Some(1))
        );
        assert_eq!(
            lone.state,
            RunState::Done {
                ended: t("2026-11-05T11:20:00Z")
            }
        );
        assert_eq!(
            (
                lone.worked,
                lone.tools,
                lone.task,
                lone.last_tool,
                lone.steps.len()
            ),
            (TimeDelta::zero(), 0, None, None, 0)
        );
        assert_eq!(lone.cwd, Some(Path::new("/Users/x/app")));
        assert_eq!(g.lines[0].run_ids, vec!["toolu_vivo"]);
    }

    #[test]
    fn a_lone_call_outside_the_window_stays_out_and_an_old_one_gets_a_stable_id() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let calls = [
            call("general-purpose", "/p", "2026-11-05T11:30:00Z"),
            call("docs", "/p", "2026-11-05T10:59:00Z"),
        ];
        let agents = defined();
        let a = active_groups(&agents, &calls, &[], since, now, home());
        let b = active_groups(&agents, &calls, &[], since, now, home());
        assert_eq!(a[0].lines.len(), 1);
        assert_eq!(a[0].lines[0].kind, RunKind::BuiltIn);
        assert_eq!(a[0].lines[0].id, b[0].lines[0].id);
        assert!(!a[0].lines[0].id.is_empty());
    }

    #[test]
    fn activity_detail_falls_back_to_a_minimal_detail_for_a_lone_call() {
        let agents = defined();
        let now = t("2026-11-05T12:00:00Z");
        let mut lost = call("docs", "/p", "2026-11-05T11:20:00Z");
        lost.id = Some("toolu_perdido".into());
        let calls = [lost];
        let d = activity_detail(&agents, &calls, &[], "toolu_perdido", now).unwrap();
        assert_eq!(
            (d.name.as_str(), d.owner, d.ended),
            ("docs", Some(0), Some(t("2026-11-05T11:20:00Z")))
        );
        assert_eq!((d.tools, d.steps.len(), d.task_full), (0, 0, None));
        assert!(activity_detail(&agents, &calls, &[], "nope", now).is_none());
        // con ejecución, igual que run_detail
        let mut r = run("docs", Some("/p"), "2026-11-05T11:00:00Z", 10);
        r.tool_use_id = Some("toolu_z".into());
        assert_eq!(
            activity_detail(&agents, &[], std::slice::from_ref(&r), "toolu_z", now),
            run_detail(&agents, &[r], "toolu_z", now)
        );
    }

    #[test]
    fn recent_count_is_the_finished_lines_in_the_window() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let runs = [
            run("docs", Some("/a"), "2026-11-05T11:10:00Z", 4),
            run("docs", Some("/b"), "2026-11-05T11:20:00Z", 4),
            running("docs", "2026-11-05T11:50:00Z", 3),
        ];
        let calls = [call("docs", "/c", "2026-11-05T11:30:00Z")];
        let groups = active_groups(&[], &calls, &runs, since, now, home());
        assert_eq!(groups.iter().map(|g| g.recent).sum::<usize>(), 3);
        assert_eq!(groups.iter().map(|g| g.running).sum::<usize>(), 1);
    }

    #[test]
    fn last_use_overall_is_the_latest_of_any_defined_agent() {
        let agents = defined();
        let calls = [
            call("docs", "/p", "2026-11-04T09:00:00Z"),
            // built-in, más reciente pero no cuenta
            call("general-purpose", "/p", "2026-11-05T09:00:00Z"),
        ];
        let runs = [run(
            "backend",
            Some("/Users/x/app"),
            "2026-11-04T10:00:00Z",
            15,
        )];
        assert_eq!(
            last_use_overall(&agents, &calls, &runs),
            Some((1, t("2026-11-04T10:15:00Z")))
        );
        assert_eq!(last_use_overall(&agents, &calls[1..], &[]), None);
        assert_eq!(last_use_overall(&[], &calls, &runs), None);
    }

    #[test]
    fn old_calls_without_a_run_are_counted() {
        let a = agent("docs", Scope::Global);
        let mut linked = call("docs", "/p", "2026-11-05T10:00:00Z");
        linked.id = Some("toolu_1".into());
        let mut unlinked = call("docs", "/p", "2026-11-05T10:05:00Z");
        unlinked.id = Some("toolu_perdido".into());
        let calls = [
            linked,
            unlinked,
            call("docs", "/p", "2026-01-01T10:00:00Z"),
            call("backend", "/p", "2026-01-01T10:00:00Z"),
        ];
        let mut r = run("docs", Some("/p"), "2026-11-05T10:00:00Z", 3);
        r.tool_use_id = Some("toolu_1".into());
        assert_eq!(calls_without_run(&a, &calls, &[r]), 2);
        assert_eq!(calls_without_run(&a, &[], &[]), 0);
    }

    #[test]
    fn last_use_counts_runs_without_a_call() {
        // Un agente lanzado desde un workflow tiene ejecución pero no llamada a Agent.
        let a = agent("backend", Scope::Global);
        let runs = [run("backend", Some("/p"), "2026-11-05T10:00:00Z", 30)];
        assert_eq!(last_use(&a, &[], &runs), Some(t("2026-11-05T10:30:00Z")));
    }

    #[test]
    fn last_use_is_the_latest_of_calls_and_runs() {
        let a = agent("docs", Scope::Global);
        let calls = [call("docs", "/p", "2026-11-05T11:00:00Z")];
        let runs = [run("docs", Some("/p"), "2026-11-05T09:00:00Z", 5)];
        assert_eq!(last_use(&a, &calls, &runs), Some(t("2026-11-05T11:00:00Z")));
        assert_eq!(last_use(&agent("otro", Scope::Global), &calls, &runs), None);
    }

    /// Una ejecución terminada de `agent_type` en `cwd` que empezó en `start`
    /// y trabajó `mins` minutos.
    fn run(agent_type: &str, cwd: Option<&str>, start: &str, mins: i64) -> Run {
        let start = t(start);
        Run {
            agent_type: agent_type.into(),
            description: None,
            tool_use_id: None,
            session_id: None,
            cwd: cwd.map(PathBuf::from),
            temp_cwd: None,
            start: Some(start),
            last: Some(start + TimeDelta::minutes(mins)),
            worked: TimeDelta::minutes(mins),
            // todo en la franja donde empieza, para que los días sumen `worked`
            worked_slots: [(
                start.duration_trunc(crate::runs::SLOT).unwrap(),
                TimeDelta::minutes(mins),
            )]
            .into_iter()
            .filter(|_| mins > 0)
            .collect(),
            tools: 0,
            task: None,
            task_full: None,
            last_tool: None,
            steps: Default::default(),
            tool_counts: Default::default(),
            finished: Some(true),
            modified: start + TimeDelta::minutes(mins),
            ..Default::default()
        }
    }

    /// La misma, sin terminar y con la transcripción tocada en su última línea.
    fn running(agent_type: &str, start: &str, mins: i64) -> Run {
        Run {
            finished: Some(false),
            ..run(agent_type, Some("/p"), start, mins)
        }
    }

    #[test]
    fn runs_follow_the_same_rule_as_calls() {
        let global = agent("docs", Scope::Global);
        let project = agent("docs", Scope::Project(PathBuf::from("/Users/x/app")));
        let inside = run(
            "docs",
            Some("/Users/x/app/front"),
            "2026-11-05T09:00:00Z",
            3,
        );
        let outside = run("docs", Some("/Users/x/app-v2"), "2026-11-05T10:00:00Z", 3);
        let no_cwd = run("docs", None, "2026-11-05T11:00:00Z", 3);
        let other = run("backend", Some("/Users/x/app"), "2026-11-05T11:30:00Z", 3);
        let all = [inside.clone(), outside, no_cwd.clone(), other];
        let starts = |a: &Agent| -> Vec<_> { runs_of(a, &all).iter().map(|r| r.start).collect() };
        // el más reciente primero; sin cwd solo encaja un agente global
        assert_eq!(starts(&project), vec![inside.start]);
        assert_eq!(starts(&global).len(), 3);
        assert_eq!(starts(&global)[0], no_cwd.start);
    }

    #[test]
    fn work_adds_up_every_run_of_the_agent() {
        let a = agent("docs", Scope::Global);
        let now = t("2026-11-05T12:00:00Z");
        let all = [
            run("docs", Some("/p"), "2026-11-05T09:00:00Z", 50),
            run("docs", Some("/p"), "2026-11-04T09:00:00Z", 22),
            run("backend", Some("/p"), "2026-11-05T10:00:00Z", 99),
        ];
        let w = work_of(&a, &all, now);
        assert_eq!(w.total, TimeDelta::minutes(72));
        assert_eq!(w.runs, 2);
        assert_eq!(w.running, None);
    }

    #[test]
    fn work_adds_up_the_tokens_of_every_run_of_the_agent() {
        let a = agent("docs", Scope::Global);
        let now = t("2026-11-05T12:00:00Z");
        let mut done = run("docs", Some("/p"), "2026-11-05T09:00:00Z", 10);
        done.usage = usage(100, None);
        // una de hace semanas cuenta igual: es el total de siempre
        let mut old = run("docs", Some("/p"), "2026-10-10T09:00:00Z", 5);
        old.usage = usage(7, None);
        // la en curso suma lo que lleva
        let mut live = running("docs", "2026-11-05T11:55:00Z", 2);
        live.usage = usage(30, Some(1));
        // la de otro agente no entra
        let mut other = run("backend", Some("/p"), "2026-11-05T10:00:00Z", 99);
        other.usage = usage(5000, None);
        let w = work_of(&a, &[done, old, live, other], now);
        // las mismas ejecuciones que cuentan en `runs`
        assert_eq!((w.runs, w.tokens), (3, 137));
    }

    #[test]
    fn no_runs_is_zero_work() {
        let a = agent("docs", Scope::Global);
        let w = work_of(&a, &[], t("2026-11-05T12:00:00Z"));
        assert_eq!(w, Work::default());
        assert_eq!(w.runs, 0);
        assert_eq!(w.tokens, 0);
    }

    #[test]
    fn a_fresh_unfinished_run_is_in_progress_and_counts_until_now() {
        let a = agent("docs", Scope::Global);
        // empezó a las 11:50, última línea a las 11:57; son las 11:59
        let all = [
            running("docs", "2026-11-05T11:50:00Z", 7),
            run("docs", Some("/p"), "2026-11-05T09:00:00Z", 10),
        ];
        let now = t("2026-11-05T11:59:00Z");
        let w = work_of(&a, &all, now);
        assert_eq!(
            w.running,
            Some(Running {
                since: Some(t("2026-11-05T11:50:00Z")),
                worked: TimeDelta::minutes(9),
            })
        );
        assert_eq!(w.total, TimeDelta::minutes(19));
        assert_eq!(w.runs, 2);
    }

    #[test]
    fn an_unfinished_run_untouched_for_ten_minutes_is_interrupted() {
        let a = agent("docs", Scope::Global);
        let all = [running("docs", "2026-11-05T11:00:00Z", 7)];
        // última línea y mtime a las 11:07; a las 11:17 ya no está en curso
        let w = work_of(&a, &all, t("2026-11-05T11:17:00Z"));
        assert_eq!(w.running, None);
        assert_eq!(w.total, TimeDelta::minutes(7));
        let w = work_of(&a, &all, t("2026-11-05T11:16:59Z"));
        assert!(w.running.is_some());
    }

    #[test]
    fn with_two_runs_in_progress_the_oldest_one_shows() {
        let a = agent("docs", Scope::Global);
        let all = [
            running("docs", "2026-11-05T11:55:00Z", 1),
            running("docs", "2026-11-05T11:50:00Z", 5),
        ];
        let w = work_of(&a, &all, t("2026-11-05T11:56:00Z"));
        assert_eq!(w.running.unwrap().since, Some(t("2026-11-05T11:50:00Z")));
    }

    #[test]
    fn a_call_links_to_its_run_by_tool_use_id() {
        let mut linked = call("docs", "/p", "2026-11-05T10:00:00Z");
        linked.id = Some("toolu_1".into());
        let mut r1 = run("docs", Some("/p"), "2026-11-05T10:00:00Z", 3);
        r1.tool_use_id = Some("toolu_1".into());
        let mut r2 = run("docs", Some("/p"), "2026-11-05T10:00:00Z", 4);
        r2.tool_use_id = Some("toolu_2".into());
        let no_id = run("docs", Some("/p"), "2026-11-05T10:00:00Z", 5);
        let all = [no_id, r2, r1];
        assert_eq!(
            run_of(&linked, &all).map(|r| r.worked),
            Some(TimeDelta::minutes(3))
        );
        // una llamada sin id nunca enlaza, ni con una ejecución sin id
        let old = call("docs", "/p", "2026-11-05T10:00:00Z");
        assert_eq!(run_of(&old, &all), None);
    }

    #[test]
    fn global_agent_matches_any_cwd() {
        let a = agent("rev", Scope::Global);
        let calls = [call("rev", "/Users/x/Offing/app", "2026-11-05T10:00:00Z")];
        assert_eq!(last_invocation(&a, &calls), Some(calls[0].at));
    }

    #[test]
    fn project_agent_matches_only_sessions_inside_its_project() {
        let a = agent(
            "backend",
            Scope::Project(PathBuf::from("/Users/x/Offing/app")),
        );
        let inside = call(
            "backend",
            "/Users/x/Offing/app/front",
            "2026-11-05T10:00:00Z",
        );
        let other = call("backend", "/Users/x/Offing/otro", "2026-11-05T11:00:00Z");
        assert_eq!(last_invocation(&a, std::slice::from_ref(&other)), None);
        assert_eq!(
            last_invocation(&a, &[other, inside.clone()]),
            Some(inside.at)
        );
    }

    #[test]
    fn project_prefix_must_be_a_whole_path_component() {
        let a = agent("backend", Scope::Project(PathBuf::from("/Users/x/app")));
        let lookalike = call("backend", "/Users/x/app-v2", "2026-11-05T10:00:00Z");
        assert_eq!(last_invocation(&a, &[lookalike]), None);
    }

    #[test]
    fn different_name_never_matches() {
        let a = agent("docs", Scope::Global);
        assert_eq!(
            last_invocation(&a, &[call("backend", "/p", "2026-11-05T10:00:00Z")]),
            None
        );
    }

    #[test]
    fn invocations_are_listed_most_recent_first() {
        let a = agent("docs", Scope::Global);
        let old = call("docs", "/p", "2026-11-05T09:00:00Z");
        let new = call("docs", "/p", "2026-11-05T10:30:00Z");
        let other = call("backend", "/p", "2026-11-05T11:00:00Z");
        let got: Vec<_> = invocations_of(&a, &[old.clone(), other, new.clone()])
            .into_iter()
            .map(|c| c.at)
            .collect();
        assert_eq!(got, vec![new.at, old.at]);
    }

    #[test]
    fn keeps_the_most_recent_call() {
        let a = agent("docs", Scope::Global);
        let old = call("docs", "/p", "2026-11-05T09:00:00Z");
        let new = call("docs", "/p", "2026-11-05T10:30:00Z");
        assert_eq!(last_invocation(&a, &[new.clone(), old]), Some(new.at));
    }

    fn at_app(mut r: Run, id: &str) -> Run {
        r.cwd = Some("/Users/x/app".into());
        r.tool_use_id = Some(id.into());
        r
    }

    #[test]
    fn two_finished_runs_of_one_agent_are_one_row_with_the_latest_data() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined();
        let mut old = at_app(run("docs", None, "2026-11-05T11:10:00Z", 5), "toolu_old");
        old.tools = 3;
        let mut new = at_app(run("docs", None, "2026-11-05T11:30:00Z", 9), "toolu_new");
        new.tools = 7;
        let runs = [old, new];
        let groups = active_groups(&agents, &[], &runs, since, now, home());
        let g = &groups[0];
        assert_eq!((g.running, g.recent, g.lines.len()), (0, 1, 1));
        let l = &g.lines[0];
        assert_eq!((l.runs, l.id.as_str(), l.tools), (2, "toolu_new", 7));
        assert_eq!(l.run_ids, vec!["toolu_new", "toolu_old"]);
        assert_eq!(
            l.state,
            RunState::Done {
                ended: t("2026-11-05T11:39:00Z")
            }
        );
    }

    #[test]
    fn a_running_and_a_finished_run_of_one_agent_are_two_rows() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined();
        // la terminada es más reciente por `ended`, pero la en curso va arriba
        let done = at_app(run("docs", None, "2026-11-05T11:20:00Z", 38), "toolu_done");
        let mut live = at_app(running("docs", "2026-11-05T11:10:00Z", 45), "toolu_live");
        live.tools = 9;
        live.skill = Some("superpowers:brainstorming".into());
        let mut done = done;
        done.skill = Some("superpowers:writing-plans".into());
        let runs = [done, live];
        let groups = active_groups(&agents, &[], &runs, since, now, home());
        let l = &groups[0].lines[0];
        // cada fila con su propia skill
        assert_eq!(l.skill, Some("superpowers:brainstorming"));
        assert_eq!(groups[0].lines.len(), 2);
        assert_eq!((groups[0].running, groups[0].recent), (1, 1));
        assert_eq!(
            (l.state, l.runs, l.id.as_str(), l.tools, l.chain),
            (RunState::Running, 1, "toolu_live", 9, Chain::Head(2))
        );
        assert_eq!(l.run_ids, vec!["toolu_live"]);
        let d = &groups[0].lines[1];
        assert_eq!(
            (d.runs, d.id.as_str(), d.skill, d.chain),
            (
                1,
                "toolu_done",
                Some("superpowers:writing-plans"),
                Chain::Last
            )
        );
        assert_eq!(d.run_ids, vec!["toolu_done"]);
    }

    #[test]
    fn same_name_with_different_kind_are_two_rows_and_workflow_joins_its_owner() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined();
        // `docs` definido (global) y un built-in con el mismo nombre en otro proyecto
        let defined_run = at_app(run("docs", None, "2026-11-05T11:10:00Z", 5), "toolu_a");
        let mut wf = at_app(
            run("workflow-subagent", None, "2026-11-05T11:20:00Z", 5),
            "toolu_w",
        );
        wf.description = Some("docs".into());
        // sin agentes definidos: `visual` built-in y `visual` de workflow sin dueño
        let builtin = at_app(run("visual", None, "2026-11-05T11:30:00Z", 5), "toolu_b");
        let mut wf_visual = at_app(
            run("workflow-subagent", None, "2026-11-05T11:40:00Z", 5),
            "toolu_v",
        );
        wf_visual.description = Some("visual".into());
        let runs = [defined_run, wf, builtin, wf_visual];
        let groups = active_groups(&agents, &[], &runs, since, now, home());
        let rows: Vec<_> = groups[0]
            .lines
            .iter()
            .map(|l| (l.name, l.kind, l.runs))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("visual", RunKind::WorkflowStep, 1),
                ("visual", RunKind::BuiltIn, 1),
                ("docs", RunKind::Workflow, 2),
            ]
        );
    }

    #[test]
    fn the_same_agent_in_two_projects_is_two_rows_in_two_groups() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined();
        let a = run("docs", Some("/Users/x/a"), "2026-11-05T11:10:00Z", 5);
        let b = run("docs", Some("/Users/x/b"), "2026-11-05T11:20:00Z", 5);
        let runs = [a, b];
        let groups = active_groups(&agents, &[], &runs, since, now, home());
        assert_eq!(groups.len(), 2);
        assert!(groups
            .iter()
            .all(|g| g.lines.len() == 1 && g.lines[0].runs == 1));
    }

    #[test]
    fn counters_count_rows_and_sibling_ids_come_from_the_agent_block() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined();
        let calls = [call("Explore", "/Users/x/app", "2026-11-05T11:45:00Z")];
        let runs = [
            at_app(running("docs", "2026-11-05T11:50:00Z", 5), "toolu_1"),
            at_app(run("docs", None, "2026-11-05T11:10:00Z", 5), "toolu_2"),
            at_app(run("docs", None, "2026-11-05T11:20:00Z", 5), "toolu_3"),
            at_app(run("backend", None, "2026-11-05T11:30:00Z", 5), "toolu_4"),
        ];
        let groups = active_groups(&agents, &calls, &runs, since, now, home());
        let g = &groups[0];
        // docs en curso; terminadas: Explore (llamada), backend, docs (2 fundidas)
        assert_eq!((g.running, g.recent, g.lines.len()), (1, 3, 4));
        assert_eq!(g.running + g.recent, g.lines.len());
        // todo el bloque de `docs`: la en curso y luego sus terminadas fundidas,
        // de la más reciente a la más antigua; da igual desde qué id se pregunte
        // (también uno que no representa a su fila)
        let docs = vec!["toolu_1", "toolu_3", "toolu_2"];
        assert_eq!(sibling_ids(&groups, "toolu_2"), docs);
        assert_eq!(sibling_ids(&groups, "toolu_3"), docs);
        assert_eq!(sibling_ids(&groups, "toolu_1"), docs);
        // un bloque de una sola fila y una sola ejecución
        assert_eq!(sibling_ids(&groups, "toolu_4"), vec!["toolu_4"]);
        assert!(sibling_ids(&groups, "nope").is_empty());
    }

    /// Una ejecución en curso en ~/app con su id, encargo, skill y uso propios.
    fn live_at_app(agent_type: &str, start: &str, mins: i64, id: &str, n: u64) -> Run {
        let mut r = at_app(running(agent_type, start, mins), id);
        r.task = Some(format!("encargo {id}"));
        r.skill = Some(format!("skill-{id}"));
        r.last_tool = Some(format!("Bash · {id}"));
        r.tools = n as usize;
        r.usage = usage(n * 100, Some(n * 1_000));
        r
    }

    #[test]
    fn running_runs_are_one_row_each_in_contiguous_clusters_per_agent() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined();
        let mut d3 = live_at_app("docs", "2026-11-05T11:46:00Z", 10, "d3", 3);
        // otro `cwd` (un `cd` dentro de la sesión): sigue en el grupo de su lanzamiento
        d3.launch_dir = Some("/Users/x/app".into());
        d3.cwd = Some("/Users/x/app/front".into());
        let runs = [
            live_at_app("docs", "2026-11-05T11:40:00Z", 15, "d1", 1),
            live_at_app("general-purpose", "2026-11-05T11:55:00Z", 1, "g1", 4),
            live_at_app("docs", "2026-11-05T11:50:00Z", 5, "d2", 2),
            live_at_app("Explore", "2026-11-05T11:20:00Z", 35, "e2", 5),
            d3,
            live_at_app("backend", "2026-11-05T11:30:00Z", 25, "b1", 6),
            live_at_app("Explore", "2026-11-05T11:52:00Z", 3, "e1", 7),
            // dos terminadas de `docs`: una sola fila debajo
            at_app(run("docs", None, "2026-11-05T11:01:00Z", 4), "dd1"),
            at_app(run("docs", None, "2026-11-05T11:06:00Z", 2), "dd2"),
        ];
        let groups = active_groups(&agents, &[], &runs, since, now, home());
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        let rows: Vec<_> = g
            .lines
            .iter()
            .map(|l| {
                (
                    l.id.as_str(),
                    l.name,
                    l.state == RunState::Running,
                    l.chain,
                    l.runs,
                )
            })
            .collect();
        // racimos por su inicio más tardío: g1 11:55, Explore 11:52, docs 11:50, backend 11:30
        assert_eq!(
            rows,
            vec![
                ("g1", "general-purpose", true, Chain::Single, 1),
                ("e1", "Explore", true, Chain::Head(2), 1),
                ("e2", "Explore", true, Chain::Last, 1),
                ("d2", "docs", true, Chain::Head(4), 1),
                ("d3", "docs", true, Chain::Middle, 1),
                ("d1", "docs", true, Chain::Middle, 1),
                // la terminada cierra el racimo de su agente
                ("dd2", "docs", false, Chain::Last, 2),
                ("b1", "backend", true, Chain::Single, 1),
            ]
        );
        // cada en curso con lo suyo
        let d3 = &g.lines[4];
        assert_eq!(d3.run_ids, vec!["d3"]);
        assert_eq!(
            (d3.task, d3.skill, d3.last_tool, d3.tools, d3.tokens),
            (
                Some("encargo d3"),
                Some("skill-d3"),
                Some("Bash · d3"),
                3,
                300
            )
        );
        assert_eq!(d3.context.map(|c| c.used), Some(3_000));
        assert_eq!(d3.cwd, Some(Path::new("/Users/x/app/front")));
        // 10 min hasta su última línea a las 11:56 y 4 más hasta ahora
        assert_eq!(d3.worked, mins(14));
        assert!(g
            .lines
            .iter()
            .filter(|l| l.state == RunState::Running)
            .all(|l| l.run_ids == vec![l.id.clone()]));
        // la terminada funde las dos
        assert_eq!(g.lines[6].run_ids, vec!["dd2", "dd1"]);
        assert_eq!((g.running, g.recent), (7, 1));
        assert_eq!(g.running + g.recent, g.lines.len());
        assert_eq!(g.tokens, g.lines.iter().map(|l| l.tokens).sum::<u64>());
        assert_eq!(g.tokens, 2_800);
        // ←→ en la ficha: todo el bloque del agente, las en curso en su orden
        // y luego las fundidas de la terminada
        let docs = vec!["d2", "d3", "d1", "dd2", "dd1"];
        assert_eq!(sibling_ids(&groups, "d1"), docs);
        assert_eq!(sibling_ids(&groups, "dd1"), docs);
        assert_eq!(sibling_ids(&groups, "e2"), vec!["e1", "e2"]);
        assert_eq!(sibling_ids(&groups, "g1"), vec!["g1"]);
    }

    #[test]
    fn running_rows_of_one_agent_in_two_projects_are_two_singles() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined();
        let mut a = running("docs", "2026-11-05T11:50:00Z", 5);
        a.cwd = Some("/Users/x/a".into());
        a.tool_use_id = Some("a".into());
        let mut b = running("docs", "2026-11-05T11:55:00Z", 1);
        b.cwd = Some("/Users/x/b".into());
        b.tool_use_id = Some("b".into());
        let runs = [a, b];
        let groups = active_groups(&agents, &[], &runs, since, now, home());
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(|g| g.lines[0].chain == Chain::Single));
        assert_eq!(sibling_ids(&groups, "a"), vec!["a"]);
    }

    #[test]
    fn finished_rows_join_their_agent_block_and_finished_only_agents_follow() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined();
        let runs = [
            // `docs`: dos en curso y una terminada antigua (11:05)
            live_at_app("docs", "2026-11-05T11:50:00Z", 9, "d1", 1),
            live_at_app("docs", "2026-11-05T11:40:00Z", 19, "d2", 1),
            at_app(run("docs", None, "2026-11-05T11:00:00Z", 5), "dd"),
            // `backend`: una en curso y una terminada
            live_at_app("backend", "2026-11-05T11:30:00Z", 29, "b1", 1),
            at_app(run("backend", None, "2026-11-05T11:10:00Z", 5), "bd"),
            // solo terminadas: Explore a las 11:45, Plan a las 11:35
            at_app(run("Plan", None, "2026-11-05T11:30:00Z", 5), "pd"),
            at_app(run("Explore", None, "2026-11-05T11:40:00Z", 5), "ed"),
        ];
        let groups = active_groups(&agents, &[], &runs, since, now, home());
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        let rows: Vec<_> = g.lines.iter().map(|l| (l.id.as_str(), l.chain)).collect();
        assert_eq!(
            rows,
            vec![
                ("d1", Chain::Head(3)),
                ("d2", Chain::Middle),
                ("dd", Chain::Last),
                ("b1", Chain::Head(2)),
                ("bd", Chain::Last),
                ("ed", Chain::Single),
                ("pd", Chain::Single),
            ]
        );
        // los contadores no cambian de significado
        assert_eq!((g.running, g.recent), (3, 4));
        // ←→ en la ficha: el bloque entero, desde una en curso o desde la terminada
        assert_eq!(sibling_ids(&groups, "d2"), vec!["d1", "d2", "dd"]);
        assert_eq!(sibling_ids(&groups, "dd"), vec!["d1", "d2", "dd"]);
        assert_eq!(sibling_ids(&groups, "bd"), vec!["b1", "bd"]);
        // un agente solo con terminada no arrastra a los demás
        assert_eq!(sibling_ids(&groups, "ed"), vec!["ed"]);
    }

    /// Uso con `total` tokens de entrada, `context` de contexto y modelo opus 5.5.
    fn usage(total: u64, context: Option<u64>) -> Usage {
        Usage {
            tokens: Tokens {
                input: total,
                ..Tokens::default()
            },
            model: Some("claude-opus-5-5".into()),
            context,
            context_window: 200_000,
        }
    }

    #[test]
    fn context_percent_rounds_and_survives_an_empty_window() {
        let c = Context {
            used: 50_000,
            window: 200_000,
        };
        assert_eq!(c.percent(), 25);
        assert_eq!(
            Context {
                used: 1_999,
                window: 200_000
            }
            .percent(),
            1
        );
        assert_eq!(Context { used: 5, window: 0 }.percent(), 0);
        assert_eq!(Context::of(&usage(10, None)), None);
        assert_eq!(
            Context::of(&usage(10, Some(30_000))),
            Some(Context {
                used: 30_000,
                window: 200_000
            })
        );
    }

    #[test]
    fn a_row_adds_the_tokens_of_its_runs_and_shows_context_only_while_running() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined();
        let mut live = at_app(running("docs", "2026-11-05T11:50:00Z", 5), "toolu_1");
        live.usage = usage(1_000, Some(60_000));
        let mut done = at_app(run("docs", None, "2026-11-05T11:10:00Z", 5), "toolu_2");
        done.usage = usage(300, Some(90_000));
        let mut other = at_app(run("backend", None, "2026-11-05T11:20:00Z", 5), "toolu_3");
        other.usage = usage(20, Some(10_000));
        let lone = [call("Explore", "/Users/x/app", "2026-11-05T11:45:00Z")];
        let runs = [live, done, other];
        let groups = active_groups(&agents, &lone, &runs, since, now, home());
        let g = &groups[0];
        let rows: Vec<_> = g
            .lines
            .iter()
            .map(|l| (l.name, l.tokens, l.context))
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    "docs",
                    1_000,
                    Some(Context {
                        used: 60_000,
                        window: 200_000
                    })
                ),
                // la terminada de `docs` es su propia fila, sin contexto, pegada
                // a la en curso de su agente
                ("docs", 300, None),
                ("Explore", 0, None),
                ("backend", 20, None),
            ]
        );
        assert_eq!(g.tokens, 1_320);
        assert_eq!(window_tokens(&groups), 1_320);
        assert_eq!(window_tokens(&[]), 0);
    }

    #[test]
    fn detail_carries_usage_and_who_launched_it() {
        let now = t("2026-11-05T12:00:00Z");
        let agents = defined();
        let mut parent = at_app(run("docs", None, "2026-11-05T11:00:00Z", 30), "toolu_p");
        parent.id = "p".into();
        parent.session_id = Some("sess-1234".into());
        parent.session_prompt = Some("arregla el login".into());
        parent.usage = usage(1_000_000, Some(80_000));
        parent.children = vec!["c2".into(), "c1".into(), "gone".into()];
        // c2 empezó antes que c1: el orden es el de lanzamiento
        let mut c1 = at_app(running("backend", "2026-11-05T11:50:00Z", 5), "toolu_c1");
        c1.id = "c1".into();
        c1.parent_run = Some("p".into());
        c1.task = Some("pinta".into());
        c1.tools = 4;
        c1.usage = usage(70, Some(5_000));
        let mut c2 = at_app(
            run("workflow-subagent", None, "2026-11-05T11:05:00Z", 3),
            "",
        );
        c2.tool_use_id = None;
        c2.id = "c2".into();
        c2.description = Some("revisor".into());
        c2.parent_run = Some("p".into());
        let mut orphan = at_app(run("docs", None, "2026-11-05T11:40:00Z", 1), "toolu_o");
        orphan.parent_run = Some("borrada".into());
        let runs = [parent, c1, c2.clone(), orphan];

        let d = run_detail(&agents, &runs, "toolu_p", now).unwrap();
        assert_eq!(d.tokens, 1_000_000);
        assert_eq!(d.usage.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(
            d.context,
            Some(Context {
                used: 80_000,
                window: 200_000
            })
        );
        assert!(!d.from_cache);
        assert_eq!(
            d.launched_by,
            Some(LaunchedBy::Session {
                id: "sess-1234".into(),
                prompt: Some("arregla el login".into()),
            })
        );
        let kids: Vec<_> = d
            .launched
            .iter()
            .map(|k| (k.id.as_str(), k.name.as_str(), k.kind, k.running))
            .collect();
        assert_eq!(
            kids,
            vec![
                (
                    run_id(&c2).as_str(),
                    "revisor",
                    RunKind::WorkflowStep,
                    false
                ),
                ("toolu_c1", "backend", RunKind::Defined, true),
            ]
        );
        let c1 = &d.launched[1];
        assert_eq!(
            (c1.tools, c1.tokens, c1.task.as_deref(), c1.ended),
            (4, 70, Some("pinta"), None)
        );
        // 5 min hasta su última línea; el hueco hasta ahora no es menor que IDLE_GAP
        assert_eq!(c1.worked, mins(5));
        assert_eq!(d.launched[0].ended, Some(t("2026-11-05T11:08:00Z")));

        // la hija apunta a su madre, con el id que abre su ficha
        let k = run_detail(&agents, &runs, "toolu_c1", now).unwrap();
        match k.launched_by {
            Some(LaunchedBy::Run(p)) => {
                assert_eq!((p.id.as_str(), p.name.as_str()), ("toolu_p", "docs"));
                assert_eq!(p.owner, Some(0));
            }
            other => panic!("{other:?}"),
        }
        assert!(k.launched.is_empty());
        let o = run_detail(&agents, &runs, "toolu_o", now).unwrap();
        assert_eq!(
            o.launched_by,
            Some(LaunchedBy::GoneRun {
                id: "borrada".into()
            })
        );
        // sin sesión ni madre, no se sabe
        let lone = [call("docs", "/p", "2026-11-05T11:20:00Z")];
        let l = activity_detail(&agents, &lone, &[], &call_id(&lone[0]), now).unwrap();
        assert_eq!(
            (l.usage.model.as_deref(), l.launched_by, l.tokens),
            (None, None, 0)
        );
    }

    #[test]
    fn a_cached_run_resolves_like_a_live_one_and_counts_in_history() {
        let agents = defined();
        let now = t("2026-11-05T12:00:00Z");
        let cached = |r: Run| Run {
            from_cache: true,
            steps: Default::default(),
            last_tool: None,
            temp_cwd: None,
            ..r
        };
        let mut wf = cached(run(
            "workflow-subagent",
            Some("/Users/x/app"),
            "2026-11-04T10:00:00Z",
            20,
        ));
        wf.description = Some("backend".into());
        wf.tool_use_id = Some("toolu_wf".into());
        wf.usage = usage(500, Some(1));
        let mut direct = cached(run(
            "docs",
            Some("/Users/x/otro"),
            "2026-10-01T10:00:00Z",
            40,
        ));
        direct.tool_use_id = Some("toolu_d".into());
        let runs = [wf.clone(), direct];
        let d = run_detail(&agents, &runs, "toolu_wf", now).unwrap();
        assert_eq!(
            (d.name.as_str(), d.kind, d.owner, d.from_cache),
            ("backend", RunKind::Workflow, Some(1), true)
        );
        let d = run_detail(&agents, &runs, "toolu_d", now).unwrap();
        assert_eq!(
            (d.name.as_str(), d.kind, d.owner),
            ("docs", RunKind::Defined, Some(0))
        );
        // cuentan como cualquier otra en trabajo, días, último uso y periodo
        let docs = &agents[0];
        let w = work_of(docs, &runs, now);
        assert_eq!((w.total, w.runs), (mins(40), 1));
        assert_eq!(last_use(docs, &[], &runs), Some(t("2026-10-01T10:40:00Z")));
        assert_eq!(days_of(docs, &runs, now, &tz(0), 40)[4], mins(40));
        assert_eq!(cached_only(&runs), 2);
        assert_eq!(
            cached_only(&[run("docs", None, "2026-11-04T10:00:00Z", 1)]),
            0
        );
        // y en el ranking, con sus tokens
        let mut named = wf;
        named.agent_type = "backend".into();
        let s = stats_in(&agents[1], &[named], now, &tz(0), Period::Week);
        assert_eq!(
            s,
            PeriodStats {
                worked: mins(20),
                runs: 1,
                tokens: 500
            }
        );
    }

    #[test]
    fn stats_in_add_the_tokens_of_the_runs_started_in_the_period() {
        let a = agent("docs", Scope::Global);
        let now = t("2026-11-05T12:00:00Z");
        let mut today = run("docs", Some("/p"), "2026-11-05T09:00:00Z", 10);
        today.usage = usage(100, None);
        let mut old = run("docs", Some("/p"), "2026-10-10T09:00:00Z", 5);
        old.usage = usage(7, None);
        let mut live = running("docs", "2026-11-05T11:55:00Z", 2);
        live.usage = usage(30, Some(1));
        let all = [today, old, live];
        let week = stats_in(&a, &all, now, &tz(0), Period::Week);
        assert_eq!(
            week,
            PeriodStats {
                worked: mins(15),
                runs: 2,
                tokens: 130
            }
        );
        let ever = stats_in(&a, &all, now, &tz(0), Period::Ever);
        assert_eq!((ever.runs, ever.tokens), (3, 137));
        // la parte de trabajo es la de `work_in`
        let w = work_in(&a, &all, now, &tz(0), Period::Week);
        assert_eq!((w.worked, w.runs), (week.worked, week.runs));
        let mut sum = PeriodStats::default();
        sum += week;
        sum += ever;
        assert_eq!(sum.tokens, 267);
    }

    #[test]
    fn rank_key_cycles_and_orders_descending_with_ties_on_the_other_keys() {
        assert_eq!(RankKey::default(), RankKey::Worked);
        assert_eq!(RankKey::Worked.next(), RankKey::Tokens);
        assert_eq!(RankKey::Tokens.next(), RankKey::Runs);
        assert_eq!(RankKey::Runs.next(), RankKey::Worked);
        let s = |m, runs, tokens| PeriodStats {
            worked: mins(m),
            runs,
            tokens,
        };
        let mut v = [s(10, 1, 500), s(30, 2, 100), s(20, 5, 100), s(20, 1, 900)];
        let order = |v: &mut [PeriodStats], k: RankKey| {
            v.sort_by(|a, b| k.cmp(a, b));
            v.iter().map(|x| x.worked.num_minutes()).collect::<Vec<_>>()
        };
        assert_eq!(order(&mut v, RankKey::Worked), vec![30, 20, 20, 10]);
        // a igualdad de trabajo, más tokens primero
        assert_eq!(v[1].tokens, 900);
        assert_eq!(order(&mut v, RankKey::Tokens), vec![20, 10, 30, 20]);
        assert_eq!(order(&mut v, RankKey::Runs), vec![20, 30, 20, 10]);
    }

    #[test]
    fn detail_carries_the_temp_cwd_and_whether_it_was_resolved() {
        let now = t("2026-11-05T12:00:00Z");
        let mut resolved = at_app(run("docs", None, "2026-11-05T11:10:00Z", 5), "toolu_r");
        resolved.temp_cwd = Some("/tmp/claude-1".into());
        let mut unresolved = run("docs", Some("/tmp/claude-1"), "2026-11-05T11:10:00Z", 5);
        unresolved.tool_use_id = Some("toolu_u".into());
        unresolved.temp_cwd = Some("/tmp/claude-1".into());
        let plain = at_app(run("docs", None, "2026-11-05T11:10:00Z", 5), "toolu_p");
        let all = [resolved, unresolved, plain];
        let d = |id| run_detail(&[], &all, id, now).unwrap();
        assert_eq!(
            (d("toolu_r").temp_cwd, d("toolu_r").cwd_resolved),
            (Some(PathBuf::from("/tmp/claude-1")), true)
        );
        assert!(!d("toolu_u").cwd_resolved && d("toolu_u").temp_cwd.is_some());
        assert_eq!(
            (d("toolu_p").temp_cwd, d("toolu_p").cwd_resolved),
            (None, false)
        );
        let lone = [call("docs", "/Users/x/app", "2026-11-05T11:10:00Z")];
        let l = activity_detail(&[], &lone, &[], &call_id(&lone[0]), now).unwrap();
        assert_eq!((l.temp_cwd, l.cwd_resolved), (None, false));
    }

    /// Una ejecución terminada en `cwd` de una sesión lanzada en `launch`.
    fn launched_in(agent_type: &str, cwd: &str, launch: &str, start: &str, id: &str) -> Run {
        Run {
            launch_dir: Some(launch.into()),
            tool_use_id: Some(id.into()),
            ..run(agent_type, Some(cwd), start, 5)
        }
    }

    #[test]
    fn a_session_is_one_group_by_its_launch_dir_whatever_the_cwd_of_its_subagents() {
        let agents = defined();
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let app = "/Users/x/app";
        let runs = [
            launched_in(
                "docs",
                "/Users/x/app/src",
                app,
                "2026-11-05T11:10:00Z",
                "t1",
            ),
            launched_in(
                "Explore",
                "/Users/x/app/src/css",
                app,
                "2026-11-05T11:20:00Z",
                "t2",
            ),
            // tras un `cd /tmp`: sigue siendo de la sesión y el agente es el del proyecto
            launched_in("backend", "/private/tmp", app, "2026-11-05T11:30:00Z", "t3"),
        ];
        let mut lone = call("general-purpose", "/private/tmp", "2026-11-05T11:40:00Z");
        lone.id = Some("t4".into());
        lone.launch_dir = Some(app.into());
        let calls = [lone];
        let groups = active_groups(&agents, &calls, &runs, since, now, home());
        let sections: Vec<_> = groups.iter().map(|g| g.section.as_str()).collect();
        assert_eq!(sections, vec!["~/app"]);
        let lines = &groups[0].lines;
        assert_eq!(lines.len(), 4);
        let backend = lines.iter().find(|l| l.id == "t3").unwrap();
        assert_eq!(
            (backend.name, backend.kind, backend.owner),
            ("backend", RunKind::Defined, Some(1))
        );
        // el cwd real del subagente se conserva aparte de la carpeta del grupo
        assert_eq!(backend.cwd, Some(Path::new("/private/tmp")));
        assert_eq!(backend.group_dir, Some(Path::new(app)));
        let l = lines.iter().find(|l| l.id == "t4").unwrap();
        assert_eq!(
            (l.cwd, l.group_dir),
            (Some(Path::new("/private/tmp")), Some(Path::new(app)))
        );
    }

    #[test]
    fn without_launch_dir_the_group_and_owner_fall_back_to_the_cwd() {
        let agents = defined();
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let mut a = run("docs", Some("/Users/x/app/src"), "2026-11-05T11:10:00Z", 5);
        a.tool_use_id = Some("t1".into());
        let mut tmp = run("backend", Some("/private/tmp"), "2026-11-05T11:20:00Z", 5);
        tmp.tool_use_id = Some("t2".into());
        let runs = [a, tmp];
        let groups = active_groups(&agents, &[], &runs, since, now, home());
        let got: Vec<_> = groups
            .iter()
            .map(|g| (g.section.as_str(), g.lines[0].kind, g.lines[0].group_dir))
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    "/private/tmp",
                    RunKind::BuiltIn,
                    Some(Path::new("/private/tmp"))
                ),
                (
                    "~/app/src",
                    RunKind::Defined,
                    Some(Path::new("/Users/x/app/src"))
                ),
            ]
        );
    }

    #[test]
    fn a_project_agent_launched_after_a_cd_out_of_the_project_is_still_its_own() {
        let backend = &defined()[1];
        let r = launched_in(
            "backend",
            "/private/tmp",
            "/Users/x/app",
            "2026-11-05T11:10:00Z",
            "t1",
        );
        let mut c = call("backend", "/private/tmp", "2026-11-05T11:10:00Z");
        c.launch_dir = Some("/Users/x/app".into());
        // y al revés: lanzada fuera del proyecto, un cwd dentro no la hace suya
        let mut outside = call("backend", "/Users/x/app", "2026-11-05T11:50:00Z");
        outside.launch_dir = Some("/Users/x/otro".into());
        let calls = [c.clone(), outside.clone()];
        let runs = [r];
        assert_eq!(last_invocation(backend, &calls), Some(c.at));
        assert_eq!(runs_of(backend, &runs).len(), 1);
        assert!(ever_used(backend, std::slice::from_ref(&c), &[]));
        assert!(!ever_used(backend, &[outside], &[]));
        assert_eq!(work_of(backend, &runs, t("2026-11-05T12:00:00Z")).runs, 1);
        let d = run_detail(&defined(), &runs, "t1", t("2026-11-05T12:00:00Z")).unwrap();
        assert_eq!(
            (d.owner, d.cwd, d.launch_dir),
            (
                Some(1),
                Some(PathBuf::from("/private/tmp")),
                Some(PathBuf::from("/Users/x/app"))
            )
        );
        let l = activity_detail(
            &defined(),
            &calls,
            &[],
            &call_id(&calls[0]),
            t("2026-11-05T12:00:00Z"),
        )
        .unwrap();
        assert_eq!(
            (l.owner, l.launch_dir),
            (Some(1), Some(PathBuf::from("/Users/x/app")))
        );
    }

    /// `defined()` con la ruta de definición de `docs` propia.
    fn defined_with_paths() -> Vec<Agent> {
        let mut agents = defined();
        agents[0].path = PathBuf::from("/Users/x/.claude/agents/docs.md");
        agents
    }

    #[test]
    fn agent_block_unfolds_every_run_of_the_agent_with_its_header() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined_with_paths();
        let mut dd1 = at_app(run("docs", None, "2026-11-05T11:10:00Z", 5), "dd1");
        dd1.usage = usage(1_000, Some(9_000));
        let mut dd2 = at_app(run("docs", None, "2026-11-05T11:20:00Z", 5), "dd2");
        dd2.usage = usage(2_000, Some(9_000));
        dd2.task = Some("revisa".into());
        dd2.skill = Some("superpowers:brainstorming".into());
        dd2.last_tool = Some("Read · a.rs".into());
        let runs = [
            live_at_app("docs", "2026-11-05T11:40:00Z", 15, "d1", 1),
            dd1,
            live_at_app("general-purpose", "2026-11-05T11:55:00Z", 1, "g1", 4),
            live_at_app("docs", "2026-11-05T11:50:00Z", 5, "d2", 2),
            dd2,
        ];
        // una llamada a docs sin ejecución enlazada: la terminada más reciente
        let mut lone = call("docs", "/Users/x/app", "2026-11-05T11:30:00Z");
        lone.id = Some("toolu_lone".into());
        lone.prompt_line = Some("mira esto".into());
        let calls = [lone];
        let groups = active_groups(&agents, &calls, &runs, since, now, home());
        // en la lista, las tres terminadas son UNA fila
        let fused = groups[0].lines.iter().find(|l| l.runs == 3).unwrap();
        assert_eq!(fused.run_ids, vec!["toolu_lone", "dd2", "dd1"]);

        let b = agent_block(&agents, &calls, &runs, &groups, "dd1", now).unwrap();
        assert_eq!(
            (b.name, b.kind, b.owner),
            ("docs", RunKind::Defined, Some(0))
        );
        assert_eq!(
            b.definition,
            Some(Path::new("/Users/x/.claude/agents/docs.md"))
        );
        assert_eq!(b.section, "~/app");
        assert_eq!(b.dir, Some(Path::new("/Users/x/app")));
        assert_eq!((b.running, b.finished, b.runs.len()), (2, 3, 5));
        assert_eq!(b.tokens, 100 + 200 + 2_000 + 1_000);
        // las en curso en el orden del bloque y luego las terminadas, una a
        // una, de la más reciente a la más antigua: el orden de `sibling_ids`
        let ids: Vec<&str> = b.runs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["d2", "d1", "toolu_lone", "dd2", "dd1"]);
        assert_eq!(ids, sibling_ids(&groups, "dd1"));
        // cada fila es UNA ejecución con sus propias cifras
        assert!(b
            .runs
            .iter()
            .all(|r| r.runs == 1 && r.run_ids == [r.id.clone()] && r.chain == Chain::Single));
        let d2 = &b.runs[0];
        assert_eq!(d2.state, RunState::Running);
        assert_eq!(
            (d2.task, d2.last_tool, d2.skill, d2.tokens, d2.worked),
            (
                Some("encargo d2"),
                Some("Bash · d2"),
                Some("skill-d2"),
                200,
                mins(5)
            )
        );
        assert_eq!(d2.context.map(|c| c.used), Some(2_000));
        let done = &b.runs[3];
        assert_eq!(
            done.state,
            RunState::Done {
                ended: t("2026-11-05T11:25:00Z")
            }
        );
        assert_eq!(
            (done.task, done.last_tool, done.skill, done.tokens),
            (
                Some("revisa"),
                Some("Read · a.rs"),
                Some("superpowers:brainstorming"),
                2_000
            )
        );
        // terminada: sin contexto, aunque su uso lo traiga
        assert_eq!((done.context, done.worked), (None, mins(5)));
        assert_eq!((b.runs[4].tokens, b.runs[4].lone_call), (1_000, false));
        // la llamada sin ejecución: fila terminada en su instante, sin cifras
        // y marcada como tal
        let l = &b.runs[2];
        assert!(l.lone_call);
        assert_eq!(
            l.state,
            RunState::Done {
                ended: t("2026-11-05T11:30:00Z")
            }
        );
        assert_eq!(
            (l.task, l.tokens, l.worked, l.tools, l.last_tool, l.skill),
            (Some("mira esto"), 0, TimeDelta::zero(), 0, None, None)
        );
        assert!(b.runs.iter().filter(|r| r.lone_call).count() == 1);

        // da igual desde qué ejecución del bloque se pregunte
        for id in ["d2", "d1", "toolu_lone", "dd2"] {
            assert_eq!(
                agent_block(&agents, &calls, &runs, &groups, id, now).as_ref(),
                Some(&b)
            );
        }
        // un built-in: sin agente definido no hay ruta de definición
        let g = agent_block(&agents, &calls, &runs, &groups, "g1", now).unwrap();
        assert_eq!(
            (g.name, g.kind, g.owner, g.definition),
            ("general-purpose", RunKind::BuiltIn, None, None)
        );
        assert_eq!(
            (g.running, g.finished, g.runs.len(), g.tokens),
            (1, 0, 1, 400)
        );
        // el bloque ya no está en la ventana
        assert_eq!(
            agent_block(&agents, &calls, &runs, &groups, "nope", now),
            None
        );
    }

    #[test]
    fn agent_block_of_a_lone_call_alone_is_one_finished_row() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined_with_paths();
        let calls = [call("Explore", "/Users/x/app", "2026-11-05T11:45:00Z")];
        let groups = active_groups(&agents, &calls, &[], since, now, home());
        let id = call_id(&calls[0]);
        let b = agent_block(&agents, &calls, &[], &groups, &id, now).unwrap();
        assert_eq!(
            (b.name, b.kind, b.definition),
            ("Explore", RunKind::BuiltIn, None)
        );
        assert_eq!((b.running, b.finished, b.tokens), (0, 1, 0));
        assert!(b.runs[0].lone_call);
        // en la lista de activos la fila también lo dice
        assert!(groups[0].lines[0].lone_call);
    }

    /// Las tres cifras del bloque de `id`: lo que suman sus filas en activos
    /// (`runs` de cada una; el `×N` de la línea del agente ya no es esto, solo
    /// cuenta las en curso), `sibling_ids` y la pantalla del agente
    /// (`agent_block`, que además debe cuadrar con `running + finished`).
    fn block_counts(
        agents: &[Agent],
        calls: &[Invocation],
        runs: &[Run],
        since: DateTime<Utc>,
        now: DateTime<Utc>,
        id: &str,
    ) -> (usize, usize, usize) {
        let groups = active_groups(agents, calls, runs, since, now, home());
        let (g, line) = groups
            .iter()
            .find_map(|g| {
                let has = |l: &&NowRun| l.run_ids.iter().any(|r| r == id);
                g.lines.iter().find(has).map(|l| (g, l))
            })
            .unwrap();
        let key = agent_key(line);
        let listed: usize = g
            .lines
            .iter()
            .filter(|l| agent_key(l) == key)
            .map(|l| l.runs)
            .sum();
        let b = agent_block(agents, calls, runs, &groups, id, now).unwrap();
        assert_eq!(b.running + b.finished, b.runs.len());
        (listed, sibling_ids(&groups, id).len(), b.runs.len())
    }

    #[test]
    fn the_block_total_is_the_same_in_the_list_the_siblings_and_the_agent_screen() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined();
        // un paso de workflow que resuelve a `docs`: mismo bloque que las definidas
        let mut wf = at_app(
            run("workflow-subagent", None, "2026-11-05T11:15:00Z", 5),
            "wf",
        );
        wf.description = Some("docs".into());
        let runs = [
            live_at_app("docs", "2026-11-05T11:50:00Z", 5, "d1", 1),
            live_at_app("docs", "2026-11-05T11:55:00Z", 2, "d2", 2),
            at_app(run("docs", None, "2026-11-05T11:10:00Z", 5), "dd1"),
            at_app(run("docs", None, "2026-11-05T11:20:00Z", 5), "dd2"),
            wf,
            // enlazada con su llamada: una sola vez, no dos
            at_app(run("docs", None, "2026-11-05T11:30:00Z", 5), "linked"),
            // terminada fuera de la ventana: no entra en ninguna cifra
            at_app(run("docs", None, "2026-11-05T10:00:00Z", 5), "old"),
            // otro agente del mismo proyecto: otro bloque
            at_app(run("backend", None, "2026-11-05T11:40:00Z", 5), "b1"),
        ];
        let mut linked = call("docs", "/Users/x/app", "2026-11-05T11:30:00Z");
        linked.id = Some("linked".into());
        let mut lone = call("docs", "/Users/x/app", "2026-11-05T11:35:00Z");
        lone.id = Some("lone".into());
        let calls = [
            linked,
            lone,
            // una antigua, sin id, y otra fuera de la ventana
            call("docs", "/Users/x/app", "2026-11-05T11:05:00Z"),
            call("docs", "/Users/x/app", "2026-11-05T10:30:00Z"),
        ];
        // 2 en curso + dd1, dd2, wf, linked + 2 llamadas sin ejecución
        for id in ["d1", "dd1", "wf", "linked", "lone"] {
            assert_eq!(
                block_counts(&agents, &calls, &runs, since, now, id),
                (8, 8, 8),
                "desde {id}"
            );
        }
        assert_eq!(
            block_counts(&agents, &calls, &runs, since, now, "b1"),
            (1, 1, 1)
        );
    }

    #[test]
    fn runs_and_calls_that_share_an_id_still_count_the_same_everywhere() {
        let now = t("2026-11-05T12:00:00Z");
        let since = t("2026-11-05T11:00:00Z");
        let agents = defined();
        // la misma llamada leída de dos transcripciones (sesión reanudada)
        let mut lone = call("docs", "/Users/x/app", "2026-11-05T11:35:00Z");
        lone.id = Some("lone".into());
        let calls = [lone.clone(), lone];
        // dos terminadas con el mismo `tool_use_id`
        let runs = [
            live_at_app("docs", "2026-11-05T11:50:00Z", 5, "d1", 1),
            at_app(run("docs", None, "2026-11-05T11:10:00Z", 5), "dup"),
            at_app(run("docs", None, "2026-11-05T11:20:00Z", 5), "dup"),
            at_app(run("docs", None, "2026-11-05T11:25:00Z", 5), "dd"),
        ];
        let (listed, siblings, screen) = block_counts(&agents, &calls, &runs, since, now, "dd");
        assert_eq!((listed, siblings), (screen, screen));
        // los tokens de la pantalla, los mismos que suman las filas de la lista
        let groups = active_groups(&agents, &calls, &runs, since, now, home());
        let b = agent_block(&agents, &calls, &runs, &groups, "dd", now).unwrap();
        assert_eq!(b.tokens, groups[0].tokens);
    }

    #[test]
    fn detail_carries_every_skill_and_the_group_folder() {
        let now = t("2026-11-05T12:00:00Z");
        let agents = defined();
        let mut r = at_app(running("docs", "2026-11-05T11:50:00Z", 5), "toolu_s");
        r.launch_dir = Some("/Users/x/app".into());
        r.cwd = Some("/Users/x/app/front".into());
        let skills = vec![
            SkillUse {
                at: t("2026-11-05T11:51:00Z"),
                name: "superpowers:verification-before-completion".into(),
            },
            SkillUse {
                at: t("2026-11-05T11:53:00Z"),
                name: "frontend-design".into(),
            },
        ];
        r.skills = skills.clone().into();
        r.skill = Some("frontend-design".into());
        // sin `launch_dir`, la carpeta del grupo es el `cwd`
        let plain = at_app(run("docs", None, "2026-11-05T11:00:00Z", 5), "toolu_n");
        let runs = [r, plain];
        let d = run_detail(&agents, &runs, "toolu_s", now).unwrap();
        // todas, de la más antigua a la más reciente, como `steps`
        assert_eq!(d.skills, skills);
        assert_eq!(d.group_dir(), Some(Path::new("/Users/x/app")));
        let n = run_detail(&agents, &runs, "toolu_n", now).unwrap();
        assert!(n.skills.is_empty());
        assert_eq!(n.group_dir(), Some(Path::new("/Users/x/app")));
        // una llamada sin ejecución: sin skills; su carpeta, la de la llamada
        let calls = [call("Explore", "/Users/x/otro", "2026-11-05T11:45:00Z")];
        let c = activity_detail(&agents, &calls, &runs, &call_id(&calls[0]), now).unwrap();
        assert!(c.skills.is_empty());
        assert_eq!(c.group_dir(), Some(Path::new("/Users/x/otro")));
    }
}
