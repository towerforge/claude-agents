//! Lista los subagentes de Claude Code que hay en disco y marca cuáles se han
//! invocado hace poco, cruzando las definiciones con las transcripciones.

mod agents;
mod report;
mod runs;
mod sessions;
mod status;
mod transcripts;
mod tui;
mod update;

use agents::Agent;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use clap::{Args, Parser, Subcommand};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Lists Claude Code agents and marks which ones are active"
)]
struct Cli {
    /// Directories to search for `.claude/agents/*.md` (repeatable). Defaults to the home directory.
    #[arg(short, long = "root", value_name = "DIR")]
    roots: Vec<PathBuf>,

    /// Window in minutes: an agent is active if it was used within it or has a run in progress.
    #[arg(short, long, default_value_t = 10, value_name = "N")]
    minutes: i64,

    /// Also shows open Claude sessions, with pid and command (only with --plain).
    #[arg(short, long)]
    verbose: bool,

    /// Prints the table and exits, instead of opening the interactive screen.
    /// This also happens when the output is not a terminal.
    #[arg(short, long)]
    plain: bool,

    /// Sin subcomando: la pantalla o la tabla de siempre.
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug, PartialEq)]
enum Cmd {
    /// Update claude-agents: checks GitHub and installs the new release
    Update(UpdateArgs),
}

/// Opciones de `claude-agents update`, las mismas que `moon update`. Es lo
/// que `main()` pasa a la lógica de actualización (backend).
#[derive(Args, Debug, PartialEq)]
struct UpdateArgs {
    /// Say what there is and install nothing
    #[arg(long)]
    check: bool,
    /// Install without asking
    #[arg(long, short = 'y')]
    yes: bool,
    /// Install over a cargo install or a build under target/, and
    /// reinstall a version that is already there
    #[arg(long)]
    force: bool,
    /// A specific version instead of the latest one
    #[arg(long, value_name = "VERSION")]
    to: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(Cmd::Update(args)) = &cli.cmd {
        return update_cmd(args);
    }
    let home = std::env::home_dir().context("cannot determine the home directory")?;
    let claude_dir = home.join(".claude");

    // El directorio global de agentes entra siempre, aunque se acote con --root.
    let mut roots = if cli.roots.is_empty() {
        vec![home.clone()]
    } else {
        cli.roots.clone()
    };
    roots.push(claude_dir.join("agents"));
    roots.retain(|r| r.is_dir());

    if cli.plain || !std::io::stdout().is_terminal() {
        return print_plain(&cli, &roots, &home, &claude_dir.join("projects"));
    }
    tui::run(tui::Config {
        roots,
        home,
        projects: claude_dir.join("projects"),
        history: runs::history::default_path(),
        minutes: cli.minutes,
        version: env!("CARGO_PKG_VERSION"),
        // Solo la pantalla avisa de versiones; --plain no pregunta nada.
        update_cache: tui::update_check_enabled(std::env::var_os(tui::NO_UPDATE_CHECK).as_deref())
            .then(update::cache::default_dir)
            .flatten(),
    })
}

/// `claude-agents update`, como `moon update`: la lógica en `update::flow`,
/// lo que se ve en `tui::update` (el panel) o en `update::plain` (sin
/// terminal). Sale con 1 solo si falla; lo que fue mal ya lo ha dicho la vista.
fn update_cmd(args: &UpdateArgs) -> Result<()> {
    let to = args
        .to
        .as_deref()
        .map(str::parse::<update::Version>)
        .transpose()
        .context("--to")?;
    let interactive = std::io::stdout().is_terminal() && std::io::stdin().is_terminal();
    let opts = update::flow::Options {
        current: env!("CARGO_PKG_VERSION")
            .parse()
            .context("CARGO_PKG_VERSION is not a version")?,
        to,
        check_only: args.check,
        yes: args.yes,
        force: args.force,
        interactive,
        state_dir: update::cache::default_dir(),
    };
    let mut state = update::flow::State::new(
        opts.current.clone(),
        update::Target::current()?,
        update::install::current_exe()?,
    );
    // con terminal, el panel; sin ella (o si no se deja abrir), unas líneas
    let panel = interactive
        .then(|| tui::update::run(&opts, &mut state))
        .flatten();
    let outcome = panel.unwrap_or_else(|| {
        update::flow::drive(
            &opts,
            &update::Client::new(),
            &mut state,
            &mut update::plain::Plain::new(interactive),
        )
    });
    match outcome.exit_code() {
        0 => Ok(()),
        code => std::process::exit(code),
    }
}

/// Una fila de `--plain`: el agente, su última llamada de todo el historial,
/// lo que ha trabajado y si está activo (en curso o con una llamada dentro de
/// la ventana).
struct PlainRow<'a> {
    agent: &'a Agent,
    last: Option<DateTime<Utc>>,
    work: status::Work,
    active: bool,
}

/// Cruza agentes, invocaciones y ejecuciones del historial completo. Orden:
/// en curso, después los activos (el más reciente arriba), después inactivos
/// por su última llamada de siempre y al final los que nunca se usaron;
/// empates por nombre y ruta.
fn plain_rows<'a>(
    agents: &'a [Agent],
    calls: &[transcripts::Invocation],
    runs: &[runs::Run],
    since: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Vec<PlainRow<'a>> {
    let mut rows: Vec<PlainRow> = agents
        .iter()
        .map(|agent| {
            let last = status::last_use(agent, calls, runs);
            let work = status::work_of(agent, runs, now);
            PlainRow {
                agent,
                last,
                active: work.running.is_some() || last.is_some_and(|t| t >= since),
                work,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.work
            .running
            .is_some()
            .cmp(&a.work.running.is_some())
            .then_with(|| b.active.cmp(&a.active))
            .then_with(|| b.last.cmp(&a.last))
            .then_with(|| a.agent.name.cmp(&b.agent.name))
            .then_with(|| a.agent.path.cmp(&b.agent.path))
    });
    rows
}

/// La tabla de siempre: una pasada por el disco y a stdout.
fn print_plain(cli: &Cli, roots: &[PathBuf], home: &Path, projects: &Path) -> Result<()> {
    let now = Utc::now();
    let since = now - Duration::minutes(cli.minutes);
    let agents = agents::find_agents(roots, home)?;
    let mut history = transcripts::History::default();
    // Con el historial guardado: las ejecuciones que Claude Code ya borró
    // siguen contando, y las terminadas de esta pasada quedan guardadas.
    let mut found = runs::Runs::with_history(runs::history::default_path());
    // Historial y ejecuciones a la vez: en serie la primera pasada se va a ~2 s.
    let (read_history, read_runs) = std::thread::scope(|s| {
        let runs = s.spawn(|| found.refresh(projects));
        let history = history.refresh(projects);
        (history, runs.join())
    });
    read_history
        .and(read_runs.unwrap_or_else(|_| Err(anyhow::anyhow!("reader thread panicked"))))
        .with_context(|| format!("cannot read {}", projects.display()))?;
    let calls: Vec<_> = history.invocations().cloned().collect();
    let rows = plain_rows(&agents, &calls, &found.runs(), since, now);

    let name_w = rows
        .iter()
        .map(|r| r.agent.name.len())
        .max()
        .unwrap_or(6)
        .max(6);
    // Cada columna tan ancha como su dato más largo o su cabecera; la
    // duración va a la derecha para que se comparen de un vistazo.
    let lines: Vec<_> = rows
        .iter()
        .map(|row| {
            let state = match (row.work.running, row.active) {
                (Some(_), _) => "RUNNING",
                (None, true) => "ACTIVE",
                (None, false) => "inactive",
            };
            let when = report::last_seen(row.last, now);
            let worked = if row.work.runs == 0 {
                String::new()
            } else {
                report::worked(row.work.total)
            };
            (state, row.agent, when, worked)
        })
        .collect();
    let when_w = lines
        .iter()
        .map(|l| l.2.chars().count())
        .max()
        .unwrap_or(0)
        .max("LAST USED".chars().count());
    let worked_w = lines
        .iter()
        .map(|l| l.3.len())
        .max()
        .unwrap_or(0)
        .max("WORKED".len());
    println!(
        "{:<8}  {:<name_w$}  {:<when_w$}  {:>worked_w$}  PATH",
        "STATE", "NAME", "LAST USED", "WORKED"
    );
    for (state, agent, when, worked) in &lines {
        println!(
            "{state:<8}  {:<name_w$}  {when:<when_w$}  {worked:>worked_w$}  {}",
            agent.name,
            report::shorten(&agent.path, home)
        );
    }

    let active = rows.iter().filter(|r| r.active).count();
    let sessions = sessions::open_sessions()?;
    println!();
    println!(
        "{} agents, {active} active (window: {} min). Open Claude sessions: {}",
        rows.len(),
        cli.minutes,
        sessions.len()
    );
    if cli.verbose {
        for s in &sessions {
            println!("  {:>6}  {}", s.pid, s.command);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agents::Scope;

    fn agent(name: &str) -> Agent {
        Agent {
            name: name.into(),
            path: PathBuf::from(format!("/Users/x/.claude/agents/{name}.md")),
            scope: Scope::Global,
            description: None,
        }
    }

    fn call(agent: &str, at: &str) -> transcripts::Invocation {
        transcripts::Invocation {
            agent: agent.into(),
            cwd: PathBuf::from("/p"),
            at: at.parse().unwrap(),
            id: None,
            description: None,
            prompt_line: None,
            launch_dir: None,
        }
    }

    #[test]
    fn plain_rows_keep_the_last_call_ever_but_only_mark_the_window_active() {
        let agents = [
            agent("viejo"),
            agent("nunca"),
            agent("reciente"),
            agent("ayer"),
        ];
        let calls = [
            call("viejo", "2026-09-01T10:00:00Z"),
            call("reciente", "2026-11-05T11:58:00Z"),
            call("reciente", "2026-10-01T10:00:00Z"),
            call("ayer", "2026-11-04T12:00:00Z"),
        ];
        let since = "2026-11-05T11:50:00Z".parse().unwrap();
        let rows = plain_rows(&agents, &calls, &[], since, since);
        let got: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    r.agent.name.as_str(),
                    r.last.map(|t| t.to_rfc3339()),
                    r.active,
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("reciente", Some("2026-11-05T11:58:00+00:00".into()), true),
                ("ayer", Some("2026-11-04T12:00:00+00:00".into()), false),
                ("viejo", Some("2026-09-01T10:00:00+00:00".into()), false),
                ("nunca", None, false),
            ]
        );
    }

    #[test]
    fn plain_rows_never_used_go_by_name() {
        let agents = [agent("zeta"), agent("alfa")];
        let since = "2026-11-05T11:50:00Z".parse().unwrap();
        let rows = plain_rows(&agents, &[], &[], since, since);
        let names: Vec<_> = rows.iter().map(|r| r.agent.name.as_str()).collect();
        assert_eq!(names, vec!["alfa", "zeta"]);
    }

    fn run(agent_type: &str, start: &str, mins: i64, finished: bool) -> runs::Run {
        let start: DateTime<Utc> = start.parse().unwrap();
        let last = start + Duration::minutes(mins);
        runs::Run {
            agent_type: agent_type.into(),
            description: None,
            tool_use_id: None,
            session_id: None,
            cwd: Some(PathBuf::from("/p")),
            temp_cwd: None,
            start: Some(start),
            last: Some(last),
            worked: Duration::minutes(mins),
            worked_slots: Default::default(),
            tools: 0,
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

    #[test]
    fn plain_rows_add_up_the_work_and_put_runs_in_progress_first() {
        let agents = [agent("reciente"), agent("trabajando"), agent("parado")];
        let calls = [
            call("reciente", "2026-11-05T11:58:00Z"),
            call("trabajando", "2026-11-04T10:00:00Z"),
        ];
        let runs = [
            run("reciente", "2026-11-05T11:00:00Z", 30, true),
            run("reciente", "2026-11-04T11:00:00Z", 45, true),
            // sin terminar y tocada hace 2 min: en curso, aunque su llamada
            // quedó fuera de la ventana
            run("trabajando", "2026-11-05T11:50:00Z", 8, false),
            // sin terminar pero parada desde hace 20 min: interrumpida
            run("parado", "2026-11-05T11:30:00Z", 10, false),
        ];
        let now = "2026-11-05T12:00:00Z".parse().unwrap();
        let since = "2026-11-05T11:50:00Z".parse().unwrap();
        let rows = plain_rows(&agents, &calls, &runs, since, now);
        let got: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    r.agent.name.as_str(),
                    r.active,
                    r.work.running.is_some(),
                    r.work.total.num_minutes(),
                    r.work.runs,
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("trabajando", true, true, 10, 1),
                ("reciente", true, false, 75, 2),
                ("parado", false, false, 10, 1),
            ]
        );
    }

    #[test]
    fn no_subcommand_keeps_the_screen_and_plain_as_today() {
        let cli = Cli::try_parse_from(["claude-agents", "-p", "-m", "5", "-v"]).unwrap();
        assert!(cli.cmd.is_none());
        assert!(cli.plain && cli.verbose);
        assert_eq!(cli.minutes, 5);
    }

    #[test]
    fn update_subcommand_defaults_to_asking_before_installing_the_latest() {
        let cli = Cli::try_parse_from(["claude-agents", "update"]).unwrap();
        assert_eq!(
            cli.cmd,
            Some(Cmd::Update(UpdateArgs {
                check: false,
                yes: false,
                force: false,
                to: None,
            }))
        );
    }

    #[test]
    fn update_subcommand_parses_the_same_flags_as_moon() {
        let cli = Cli::try_parse_from([
            "claude-agents",
            "update",
            "--check",
            "-y",
            "--force",
            "--to",
            "1.0.0",
        ])
        .unwrap();
        assert_eq!(
            cli.cmd,
            Some(Cmd::Update(UpdateArgs {
                check: true,
                yes: true,
                force: true,
                to: Some("1.0.0".into()),
            }))
        );
        let long = Cli::try_parse_from(["claude-agents", "update", "--yes"]).unwrap();
        assert!(matches!(
            long.cmd,
            Some(Cmd::Update(UpdateArgs { yes: true, .. }))
        ));
        assert!(Cli::try_parse_from(["claude-agents", "update", "--to"]).is_err());
    }

    #[test]
    fn help_lists_the_update_subcommand() {
        use clap::CommandFactory;
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("update"), "{help}");
        assert!(help.contains("Update claude-agents"), "{help}");
    }

    #[test]
    fn plain_flag_is_off_by_default_and_parses() {
        assert!(!Cli::try_parse_from(["claude-agents"]).unwrap().plain);
        assert!(
            Cli::try_parse_from(["claude-agents", "--plain"])
                .unwrap()
                .plain
        );
        assert!(
            Cli::try_parse_from(["claude-agents", "-p", "-m", "5"])
                .unwrap()
                .plain
        );
    }
}
