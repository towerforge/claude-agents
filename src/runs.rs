//! Ejecuciones de subagentes. Cada una tiene su transcripción propia,
//! `<proyecto>/<sesión>/subagents/**/agent-<id>.jsonl`, y al lado su
//! `agent-<id>.meta.json` con el tipo de agente, la descripción y el id de la
//! llamada (`toolUseId`) que la lanzó desde la transcripción padre.

use anyhow::Result;
use chrono::{DateTime, DurationRound, NaiveDate, TimeDelta, TimeZone, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use walkdir::WalkDir;

pub mod cwd;
mod ending;
pub mod history;
mod links;
mod steps;
mod text;
pub mod usage;
use ending::{turn_of, Ending, Turn};
use history::Kept;
use steps::Steps;
pub use steps::{SkillUse, ToolStep};
pub use text::first_task_line;
use text::{task_full_of, task_of, tool_parts};
use usage::{usage_of, LineUsage, Usage, UsageAcc};

/// Un hueco entre dos líneas mayor que esto es espera (una ejecución retomada
/// horas después), no trabajo: no cuenta.
pub const IDLE_GAP: TimeDelta = TimeDelta::minutes(5);

/// Una ejecución sin terminar cuya transcripción lleva más que esto sin
/// tocarse se da por interrumpida, no en curso.
pub const STALE_AFTER: TimeDelta = TimeDelta::minutes(10);
/// Sin `finished` en el meta (las ejecuciones de workflow), si la transcripción
/// no acaba en un mensaje final (`Run::closed`) no hay forma de saber si
/// terminó: cuenta como en curso solo mientras se escribe.
pub const QUIET_AFTER: TimeDelta = TimeDelta::minutes(2);

/// Ancho de las franjas UTC en que se guarda el trabajo. Todas las zonas
/// horarias en uso tienen un desfase múltiplo de 15 min, así que una franja
/// nunca cruza una medianoche local y el reparto por días es exacto en
/// cualquier zona sin releer nada.
pub const SLOT: TimeDelta = TimeDelta::minutes(15);

/// Inicio de la franja de `SLOT` donde cae `at`.
fn slot_of(at: DateTime<Utc>) -> DateTime<Utc> {
    at.duration_trunc(SLOT).unwrap_or(at)
}

/// Una ejecución de un subagente, sacada de su `.jsonl` y su `.meta.json`
/// o, si Claude Code ya la borró, del historial guardado (`from_cache`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Run {
    /// Id del agente en Claude Code: el `<id>` de `agent-<id>.jsonl`.
    pub id: String,
    /// `agentType` del meta: el nombre del agente, igual que `subagent_type`.
    /// Ojo: en los pasos de un workflow vale `workflow-subagent` y el agente
    /// real (`visual`, `docs`…) va en `description`; resolverlo contra los
    /// agentes definidos es cosa del cruce (`status.rs`), no de la lectura.
    pub agent_type: String,
    pub description: Option<String>,
    /// `toolUseId` del meta: coincide con `Invocation::id` de la llamada.
    pub tool_use_id: Option<String>,
    pub session_id: Option<String>,
    pub cwd: Option<PathBuf>,
    /// Carpeta donde se lanzó la sesión de Claude Code a la que pertenece:
    /// primer `cwd` no temporal de la transcripción principal
    /// (`<proyecto>/<sesión>.jsonl`), igual para los subagentes anidados y los
    /// pasos de workflow. Distinta de `cwd` si el principal hizo `cd` antes de
    /// lanzarla. `None` si aún no se ha podido leer (se reintenta) o si viene de
    /// una entrada del historial anterior a este campo.
    pub launch_dir: Option<PathBuf>,
    /// `cwd` temporal de Claude con el que se lanzó (resuelto o no); `None` si nunca lo fue.
    pub temp_cwd: Option<PathBuf>,
    /// Primer y último `timestamp` de la transcripción.
    pub start: Option<DateTime<Utc>>,
    pub last: Option<DateTime<Utc>>,
    /// Suma de los huecos entre líneas consecutivas, sin los de más de `IDLE_GAP`.
    pub worked: TimeDelta,
    /// `worked` por franjas UTC de `SLOT` (cada hueco, en la franja donde empieza);
    /// suma lo mismo que `worked`. Por días locales: `worked_by_day`.
    pub worked_slots: BTreeMap<DateTime<Utc>, TimeDelta>,
    /// Bloques `tool_use` en los mensajes de la transcripción.
    pub tools: usize,
    /// El encargo: primera línea con contenido del primer mensaje de usuario
    /// (el prompt), en una línea y sin la cabecera `# ENCARGO (de …):`. Se fija
    /// con la primera lectura y no se recorta al ancho. `None` si no hay prompt.
    pub task: Option<String>,
    /// El encargo completo: el primer mensaje de usuario con saltos de línea,
    /// sin la cabecera `ENCARGO (de …):`, recortado a 2000 caracteres (con `…`).
    pub task_full: Option<String>,
    /// La última herramienta usada, `Nombre · argumento corto` (`Bash · git diff
    /// --stat`), o solo el nombre. Una línea, sin recortar; cambia con cada línea.
    pub last_tool: Option<String>,
    /// La skill que está usando: la última que invocó (`Skill` con
    /// `input.skill`, p. ej. `superpowers:brainstorming`), tal cual; sigue
    /// mandando hasta que carga otra. `None` si no invocó ninguna o si la
    /// ejecución viene del historial guardado. Es la última de `skills`.
    pub skill: Option<String>,
    /// Todas las invocaciones de skill con su hora, de la más antigua a la más
    /// reciente (las últimas 50, `MAX_SKILLS`). Vacía si no invocó ninguna o
    /// si la ejecución viene del historial guardado.
    pub skills: Arc<[SkillUse]>,
    /// Los últimos 200 `tool_use` (`MAX_STEPS`), del más antiguo, con su hora.
    pub steps: Arc<[ToolStep]>,
    /// Usos por herramienta sobre TODA la ejecución, de más a menos; suman `tools`.
    pub tool_counts: Arc<[(String, usize)]>,
    /// `finished` del meta; `None` si no lo trae (las de workflows no lo llevan).
    pub finished: Option<bool>,
    /// La transcripción acaba en un mensaje final (ver `ending`): la entrega
    /// del informe o un turno del asistente sin herramientas. Vuelve a `false`
    /// si después se escribe algo más.
    pub closed: bool,
    /// Última modificación del `.jsonl`.
    pub modified: DateTime<Utc>,
    /// Tokens, modelo y contexto (ver `usage`).
    pub usage: Usage,
    /// Primer prompt de la sesión que la lanzó (`<sesión>.jsonl`), en una
    /// línea; o su primer comando de barra si no hay prompt. `session_id` dice
    /// cuál es la sesión.
    pub session_prompt: Option<String>,
    /// `id` de la ejecución que la lanzó, si la lanzó otro subagente y no la
    /// sesión directamente.
    pub parent_run: Option<String>,
    /// `id` de las ejecuciones que lanzó ella, ordenados.
    pub children: Vec<String>,
    /// Solo está en el historial guardado: su transcripción ya no existe. No
    /// tiene `steps`, `last_tool`, `skill`, `skills` ni `temp_cwd`.
    pub from_cache: bool,
}

impl Run {
    /// En curso: el meta dice `finished: false` y la transcripción se tocó hace
    /// menos de `STALE_AFTER`; o el meta no dice nada (workflows), la
    /// transcripción no está cerrada (`closed`) y se tocó hace menos de
    /// `QUIET_AFTER`. `finished: true`, nunca.
    ///
    /// Con `finished: false` no se mira `closed`: el meta es explícito y Claude
    /// Code lo pone a `true` en el mismo segundo que la última línea, así que
    /// si aún dice `false` tras un mensaje final es que la ejecución sigue
    /// abierta (le van a pedir que continúe) o que se cortó, y eso ya lo
    /// resuelve `STALE_AFTER`.
    pub fn in_progress(&self, now: DateTime<Utc>) -> bool {
        let idle = now - self.modified;
        match self.finished {
            Some(false) => idle < STALE_AFTER,
            None => !self.closed && idle < QUIET_AFTER,
            Some(true) => false,
        }
    }

    /// Tiempo trabajado a fecha de `now`: el de la transcripción y, si está en
    /// curso, además lo que va desde la última línea hasta ahora, si es menos
    /// de `IDLE_GAP`.
    pub fn worked_until(&self, now: DateTime<Utc>) -> TimeDelta {
        if !self.in_progress(now) {
            return self.worked;
        }
        let extra = self
            .last
            .map(|l| now - l)
            .filter(|d| *d > TimeDelta::zero() && *d < IDLE_GAP)
            .unwrap_or_default();
        self.worked + extra
    }

    /// `worked` repartido por fecha local en `tz` (`chrono::Local` en
    /// producción): cada hueco va entero al día donde empieza. La suma de los
    /// días es `worked`.
    pub fn worked_by_day<Tz: TimeZone>(&self, tz: &Tz) -> BTreeMap<NaiveDate, TimeDelta> {
        let mut days = BTreeMap::new();
        for (slot, d) in &self.worked_slots {
            *days
                .entry(slot.with_timezone(tz).date_naive())
                .or_insert_with(TimeDelta::zero) += *d;
        }
        days
    }

    /// Como `worked_by_day`, a fecha de `now`: si está en curso, la cola desde
    /// la última línea va al día local de esa línea. La suma es `worked_until`.
    pub fn worked_by_day_until<Tz: TimeZone>(
        &self,
        now: DateTime<Utc>,
        tz: &Tz,
    ) -> BTreeMap<NaiveDate, TimeDelta> {
        let mut days = self.worked_by_day(tz);
        let tail = self.worked_until(now) - self.worked;
        if let Some(last) = self.last.filter(|_| tail > TimeDelta::zero()) {
            *days
                .entry(last.with_timezone(tz).date_naive())
                .or_insert_with(TimeDelta::zero) += tail;
        }
        days
    }

    /// Inicio de la ejecución en la hora local de `tz`.
    pub fn start_local<Tz: TimeZone>(&self, tz: &Tz) -> Option<DateTime<Tz>> {
        self.start.map(|s| s.with_timezone(tz))
    }
}

/// Lo que interesa de un `.meta.json`. Solo `agentType` es obligatorio.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Meta {
    agent_type: String,
    description: Option<String>,
    tool_use_id: Option<String>,
    finished: Option<bool>,
}

fn parse_meta(text: &str) -> Option<Meta> {
    let doc: Value = serde_json::from_str(text).ok()?;
    let text_of = |k: &str| doc.get(k).and_then(Value::as_str).map(str::to_string);
    Some(Meta {
        agent_type: text_of("agentType")?,
        description: text_of("description"),
        tool_use_id: text_of("toolUseId"),
        finished: doc.get("finished").and_then(Value::as_bool),
    })
}

/// Lo que se saca de una línea de la transcripción de una ejecución.
#[derive(Debug, Default, PartialEq, Eq)]
struct LineInfo {
    at: Option<DateTime<Utc>>,
    cwd: Option<String>,
    session_id: Option<String>,
    tools: usize,
    /// Encargo si la línea es un mensaje de usuario con texto.
    task: Option<String>,
    task_full: Option<String>,
    /// Nombre y argumento de cada `tool_use` de la línea, en orden.
    tool_uses: Vec<(String, Option<String>)>,
    /// Uso del mensaje si es una línea de asistente.
    usage: Option<LineUsage>,
    /// Ids de las llamadas `Agent`/`Task` de la línea.
    launches: Vec<String>,
    /// Lo que dice sobre el final, si es un mensaje `assistant` o `user`.
    turn: Option<Turn>,
}

fn parse_run_line(line: &str) -> Option<LineInfo> {
    let doc: Value = serde_json::from_str(line).ok()?;
    let text_of = |k: &str| doc.get(k).and_then(Value::as_str);
    // Las líneas `api-request*` (Claude Code 2.1.319+) son la petición a la API,
    // no un mensaje de la ejecución: su `message` repite bloques ya escritos en
    // líneas `assistant`. De ellas solo vale la hora.
    if text_of("type").is_some_and(|t| t.starts_with("api-request")) {
        return Some(LineInfo {
            at: text_of("timestamp").and_then(|t| t.parse().ok()),
            session_id: text_of("sessionId").map(str::to_string),
            ..LineInfo::default()
        });
    }
    let cwd = text_of("cwd");
    // Un bloque sin nombre cuenta igual (como `?`) para que la suma cuadre.
    let tool_uses: Vec<_> = doc
        .pointer("/message/content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                .map(|b| tool_parts(b, cwd).unwrap_or_else(|| ("?".into(), None)))
                .collect()
        })
        .unwrap_or_default();
    Some(LineInfo {
        at: text_of("timestamp").and_then(|t| t.parse().ok()),
        cwd: cwd.map(str::to_string),
        session_id: text_of("sessionId").map(str::to_string),
        tools: tool_uses.len(),
        task: task_of(&doc),
        task_full: task_full_of(&doc),
        tool_uses,
        usage: usage_of(&doc),
        launches: links::launch_ids(&doc),
        turn: turn_of(&doc),
    })
}

/// Lo acumulado de un `.jsonl` hasta `offset`.
#[derive(Debug, Default)]
struct Transcript {
    offset: u64,
    cwd: Option<PathBuf>,
    pub(super) temp_cwd: Option<PathBuf>,
    /// De la sesión principal, puesto por `resolve_sessions`.
    launch_dir: Option<PathBuf>,
    session_id: Option<String>,
    start: Option<DateTime<Utc>>,
    /// Último `timestamp` en orden de fichero: de él se mide el siguiente hueco.
    prev: Option<DateTime<Utc>>,
    last: Option<DateTime<Utc>>,
    worked: TimeDelta,
    /// `worked` por franjas de `SLOT`, como `Run::worked_slots`.
    slots: BTreeMap<DateTime<Utc>, TimeDelta>,
    tools: usize,
    /// Fijado por el primer mensaje de usuario con texto.
    task: Option<String>,
    task_full: Option<String>,
    steps: Steps,
    /// `toolUseId` del meta con que se leyó: la llamada que la lanzó.
    own_call: Option<String>,
    usage: UsageAcc,
    /// Llamadas `Agent`/`Task` que hizo (sin la suya propia).
    launches: Vec<String>,
    /// De la sesión padre, puesto por `resolve_sessions`.
    session_prompt: Option<String>,
    /// Si el último mensaje la cierra.
    ending: Ending,
}

impl Transcript {
    fn add(&mut self, mut info: LineInfo) {
        // Un fork empieza con la copia del mensaje del padre que lo lanzó: ni
        // su uso ni su llamada son del fork.
        let copied = self
            .own_call
            .as_ref()
            .is_some_and(|own| info.launches.contains(own));
        if copied {
            info.launches.retain(|c| Some(c) != self.own_call.as_ref());
        } else if let Some(u) = info.usage.take() {
            self.usage.add(u);
        }
        self.launches.append(&mut info.launches);
        if let Some(turn) = info.turn.take() {
            self.ending.add(turn);
        }
        self.tools += info.tools;
        if self.task.is_none() {
            self.task = info.task;
        }
        if self.task_full.is_none() {
            self.task_full = info.task_full;
        }
        // Sin `timestamp` en la línea, la hora del paso es la última conocida.
        let step_at = info.at.or(self.prev).unwrap_or_default();
        for (name, arg) in info.tool_uses {
            self.steps.push(ToolStep {
                at: step_at,
                name,
                arg,
            });
        }
        if self.cwd.is_none() {
            self.cwd = info.cwd.map(PathBuf::from);
        }
        if self.session_id.is_none() {
            self.session_id = info.session_id;
        }
        let Some(at) = info.at else { return };
        if let Some(prev) = self.prev {
            let gap = at - prev;
            if gap > TimeDelta::zero() && gap <= IDLE_GAP {
                self.worked += gap;
                *self
                    .slots
                    .entry(slot_of(prev))
                    .or_insert_with(TimeDelta::zero) += gap;
            }
        }
        self.prev = Some(at);
        self.start = Some(self.start.map_or(at, |s| s.min(at)));
        self.last = Some(self.last.map_or(at, |l| l.max(at)));
    }
}

/// Estado de una ejecución entre pasadas.
#[derive(Debug, Default)]
struct RunFile {
    transcript: Transcript,
    modified: Option<SystemTime>,
    /// mtime y tamaño del meta la última vez que se leyó.
    meta_key: Option<(SystemTime, u64)>,
    meta: Option<Meta>,
}

/// Todas las ejecuciones con transcripción propia, leídas de forma incremental
/// como `History`: la primera pasada lee cada `.jsonl` entero y las siguientes
/// solo lo añadido; el meta se relee si cambia su mtime o su tamaño. Una
/// ejecución sin meta (o con un meta sin `agentType`) no sale hasta que lo tenga.
#[derive(Debug, Default)]
pub struct Runs {
    files: HashMap<PathBuf, RunFile>,
    session_cwds: cwd::SessionCwds,
    /// Historial guardado en disco; `None` (por defecto) no lee ni escribe nada.
    kept: Option<Kept>,
}

/// `agent-*.jsonl` con un directorio `subagents` por encima. Los enlaces
/// simbólicos no cuentan: apuntan a la misma ejecución desde otra sesión.
fn is_run_transcript(path: &Path) -> bool {
    let name_ok = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("agent-") && n.ends_with(".jsonl"));
    name_ok && path.components().any(|c| c.as_os_str() == "subagents")
}

fn meta_path(transcript: &Path) -> PathBuf {
    transcript.with_extension("meta.json")
}

/// `<id>` de `agent-<id>.jsonl`.
fn run_id(transcript: &Path) -> String {
    let name = transcript
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let name = name.strip_suffix(".jsonl").unwrap_or(name);
    name.strip_prefix("agent-").unwrap_or(name).to_string()
}

impl Runs {
    /// Como `default`, pero guardando las ejecuciones terminadas en el
    /// historial de `path` (`history::default_path()` en producción) y
    /// sumando las que solo siguen allí. Sin ruta, igual que `default`.
    pub fn with_history(path: Option<PathBuf>) -> Self {
        Runs {
            kept: path.map(Kept::new),
            ..Runs::default()
        }
    }

    /// Ejecuciones que salen solo del historial guardado.
    // Sin uso hasta que frontend lo muestre; quitar este allow entonces.
    #[allow(dead_code)]
    pub fn cached_only(&self) -> usize {
        self.kept
            .as_ref()
            .map_or(0, |k| k.only_cached(&self.files_ids()))
    }

    /// Pone al día las ejecuciones con lo escrito en `projects_dir` desde la
    /// última llamada. Lo que no se puede abrir se salta y se reintenta.
    pub fn refresh(&mut self, projects_dir: &Path) -> Result<()> {
        let mut seen = HashSet::new();
        for entry in WalkDir::new(projects_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !entry.file_type().is_file() || !is_run_transcript(path) {
                continue;
            }
            seen.insert(path.to_path_buf());
            let Ok(md) = entry.metadata() else { continue };
            let state = self.files.entry(path.to_path_buf()).or_default();
            state.modified = md.modified().ok();
            refresh_meta(&meta_path(path), state);
            // Si cambia la llamada que la lanzó (el meta llegó después que las
            // líneas), se relee: de ella depende qué uso es suyo.
            let own = state.meta.as_ref().and_then(|m| m.tool_use_id.clone());
            if own != state.transcript.own_call {
                state.transcript = Transcript {
                    own_call: own,
                    ..Transcript::default()
                };
            }
            let len = md.len();
            if len == state.transcript.offset {
                continue;
            }
            if len < state.transcript.offset {
                state.transcript = Transcript {
                    own_call: state.transcript.own_call.take(),
                    ..Transcript::default()
                };
            }
            // Si falla a medias, lo ya leído queda y se sigue desde ahí.
            let _ = read_from(path, &mut state.transcript);
            state.transcript.steps.seal();
        }
        self.resolve_sessions();
        self.files.retain(|p, _| seen.contains(p));
        if self.kept.is_some() {
            let read = self.read_runs();
            if let Some(kept) = self.kept.as_mut() {
                kept.keep(&read, Utc::now());
            }
        }
        Ok(())
    }

    /// Las ejecuciones con meta válido, sin orden fijo, y después las que solo
    /// quedan en el historial guardado.
    pub fn runs(&self) -> Vec<Run> {
        let mut runs = self.read_runs();
        if let Some(kept) = &self.kept {
            runs.extend(kept.cached_runs(&self.files_ids()));
        }
        runs
    }

    fn files_ids(&self) -> HashSet<String> {
        self.files.keys().map(|p| run_id(p)).collect()
    }

    /// Las ejecuciones de las transcripciones, ya enlazadas entre sí.
    fn read_runs(&self) -> Vec<Run> {
        let mut launched = HashMap::new();
        for (path, f) in &self.files {
            for call in &f.transcript.launches {
                launched.insert(call.clone(), run_id(path));
            }
        }
        let mut runs: Vec<Run> = self
            .files
            .iter()
            .filter_map(|(path, f)| {
                let meta = f.meta.as_ref()?;
                let t = &f.transcript;
                Some(Run {
                    id: run_id(path),
                    agent_type: meta.agent_type.clone(),
                    description: meta.description.clone(),
                    tool_use_id: meta.tool_use_id.clone(),
                    session_id: t.session_id.clone(),
                    cwd: t.cwd.clone(),
                    temp_cwd: t.temp_cwd.clone(),
                    launch_dir: t.launch_dir.clone(),
                    start: t.start,
                    last: t.last,
                    worked: t.worked,
                    worked_slots: t.slots.clone(),
                    tools: t.tools,
                    task: t.task.clone(),
                    task_full: t.task_full.clone(),
                    last_tool: t.steps.last_label(),
                    skill: t.steps.skill().map(str::to_string),
                    skills: t.steps.skills_arc(),
                    steps: t.steps.recent_arc(),
                    tool_counts: t.steps.counts_arc(),
                    finished: meta.finished,
                    closed: t.ending.closed(),
                    modified: f.modified.map(DateTime::<Utc>::from).unwrap_or_default(),
                    usage: t.usage.usage(),
                    session_prompt: t.session_prompt.clone(),
                    parent_run: None,
                    children: Vec::new(),
                    from_cache: false,
                })
            })
            .collect();
        links::link(&mut runs, &launched);
        runs
    }
}

/// Relee el meta solo si cambió su mtime o su tamaño; si desaparece, se olvida.
fn refresh_meta(path: &Path, state: &mut RunFile) {
    let key = std::fs::metadata(path)
        .ok()
        .and_then(|m| Some((m.modified().ok()?, m.len())));
    if key.is_none() {
        state.meta_key = None;
        state.meta = None;
        return;
    }
    if key == state.meta_key {
        return;
    }
    if let Ok(text) = std::fs::read_to_string(path) {
        state.meta = parse_meta(&text);
        state.meta_key = key;
    }
}

/// Lee `path` desde `t.offset` hasta la última línea completa; la que está a
/// medio escribir se deja para la siguiente pasada.
fn read_from(path: &Path, t: &mut Transcript) -> std::io::Result<()> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(t.offset))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 || line.last() != Some(&b'\n') {
            return Ok(());
        }
        t.offset += n as u64;
        if let Some(info) = parse_run_line(&String::from_utf8_lossy(&line)) {
            t.add(info);
        }
    }
}

#[cfg(test)]
mod tests;
