//! El flujo de `claude-agents update`, sin terminal: qué fase toca tras cada
//! cosa que pasa (`apply`), el trabajo que va a la red (`fetch`,
//! `install_release`) y un conductor en un solo hilo (`drive`) que se lo
//! enseña a una `View`. Es el `moon_tui::update` de moon sin la pantalla:
//! una vista con panel puede llevar el trabajo a un hilo, mandarse los `Msg`
//! por un canal y llamar a `apply` en su bucle; la de texto usa `drive`.

use std::path::{Path, PathBuf};

use super::cache::CheckCache;
use super::Version;
use super::{install, unpack, verify_sha256, Refusal, Release, Source, Target, UpdateError};

pub struct Options {
    /// La versión en marcha.
    pub current: Version,
    /// Una versión concreta en lugar de la última (`--to`).
    pub to: Option<Version>,
    /// Mirar, decir qué hay y no instalar nada (`--check`).
    pub check_only: bool,
    /// Instalar sin preguntar (`--yes`).
    pub yes: bool,
    /// Instalar sobre un `cargo install` o una compilación bajo `target/`, y
    /// reinstalar la versión que ya está (`--force`).
    pub force: bool,
    /// Hay alguien delante para preguntarle. Sin terminal no se instala nada
    /// sin `--yes`.
    pub interactive: bool,
    /// Dónde vive la nota diaria (`update-check.json`). `None` no recuerda nada.
    pub state_dir: Option<PathBuf>,
}

impl Options {
    /// Si esa release merece instalarse: una más nueva siempre; la pedida por
    /// nombre también (bajar de versión es una decisión, no un error), y
    /// `--force` decide el resto.
    pub fn worth_installing(&self, release: &Version) -> bool {
        *release > self.current || self.force || (self.to.is_some() && *release != self.current)
    }
}

/// Cómo acabó, para el código de salida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    /// `--check` y hay algo más nuevo.
    Available(Version),
    Installed(Version),
    Cancelled,
    /// Ya se dijo en la vista: quien llama solo pone el código de salida.
    Failed,
}

impl Outcome {
    /// Como moon: 1 solo si falla. Tener una versión nueva no es un fallo.
    pub fn exit_code(&self) -> i32 {
        match self {
            Outcome::Failed => 1,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Download { got: u64, total: Option<u64> },
    Verify,
    Install,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Preguntando a GitHub.
    Checking,
    UpToDate,
    /// `--check`: hay versión nueva y no pasará nada más.
    Available,
    /// Esperando el visto bueno.
    Confirm,
    Working(Step),
    Done,
    Cancelled,
    /// Parado antes de descargar nada: este binario no es nuestro para
    /// sustituir, y la vista dice qué lo actualiza en su lugar.
    Refused(Refusal),
    Failed(String),
}

/// Todo lo que la vista necesita para pintar.
#[derive(Debug, Clone)]
pub struct State {
    pub current: Version,
    pub target: Target,
    /// El binario que se sustituiría (el que está en marcha).
    pub dest: PathBuf,
    pub release: Option<Release>,
    pub phase: Phase,
    /// Si el archivo se pudo comprobar contra `checksums.txt`.
    pub verified: bool,
}

impl State {
    pub fn new(current: Version, target: Target, dest: PathBuf) -> Self {
        Self {
            current,
            target,
            dest,
            release: None,
            phase: Phase::Checking,
            verified: true,
        }
    }

    /// La versión que se instalaría, una vez se sabe que merece la pena.
    pub fn next_version(&self) -> Option<&Version> {
        match self.phase {
            Phase::UpToDate | Phase::Checking => None,
            _ => self.release.as_ref().map(|r| &r.version),
        }
    }

    pub fn asset_name(&self) -> String {
        self.target.asset_name()
    }

    pub fn asset_size(&self) -> Option<u64> {
        self.release
            .as_ref()
            .and_then(|r| r.asset_for(&self.target).ok())
            .map(|a| a.size)
    }

    /// La ruta con el home como `~`, que es donde suele estar.
    pub fn dest_display(&self) -> String {
        let s = self.dest.display().to_string();
        match std::env::var("HOME") {
            Ok(h) if !h.is_empty() && s.starts_with(&h) => format!("~{}", &s[h.len()..]),
            _ => s,
        }
    }

    /// El visto bueno: empieza la descarga.
    pub fn confirm(&mut self) -> Next {
        self.phase = Phase::Working(Step::Download {
            got: 0,
            total: self.asset_size(),
        });
        Next::Install
    }

    /// No: nada se toca.
    pub fn cancel(&mut self) -> Outcome {
        self.phase = Phase::Cancelled;
        Outcome::Cancelled
    }

    pub fn outcome(&self) -> Outcome {
        let version = || self.release.as_ref().map(|r| r.version.clone());
        match &self.phase {
            Phase::UpToDate => Outcome::UpToDate,
            Phase::Available => version().map_or(Outcome::UpToDate, Outcome::Available),
            Phase::Done => version().map_or(Outcome::UpToDate, Outcome::Installed),
            Phase::Refused(_) | Phase::Failed(_) => Outcome::Failed,
            _ => Outcome::Cancelled,
        }
    }
}

/// Lo que el trabajo en segundo plano cuenta al flujo.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Release(Result<Release, String>),
    Progress(u64, Option<u64>),
    Step(Step),
    /// Instalado, y si el checksum se pudo comprobar.
    Finished(Result<bool, String>),
}

/// Qué hace el conductor tras `apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Seguir esperando mensajes (la descarga avanza).
    Wait,
    /// Preguntar: `State::confirm` o `State::cancel` según la respuesta.
    Ask,
    /// Arrancar `install_release` con la release de `State`.
    Install,
    /// Se acabó.
    Done(Outcome),
}

/// Aplica lo que dice el trabajo en segundo plano. Toca la nota diaria: la
/// reescribe al saber la última versión y la olvida tras instalar.
pub fn apply(msg: Msg, state: &mut State, opts: &Options) -> Next {
    match msg {
        Msg::Release(Ok(release)) => {
            // `--to` no es «la última»: no se apunta en la nota
            if opts.to.is_none() {
                remember(opts.state_dir.as_deref(), &release.version);
            }
            let worth_it = opts.worth_installing(&release.version);
            state.release = Some(release);
            if !worth_it {
                state.phase = Phase::UpToDate;
                return Next::Done(Outcome::UpToDate);
            }
            if opts.check_only {
                state.phase = Phase::Available;
                return Next::Done(state.outcome());
            }
            if let Err(blocked) = allowed(&state.dest, opts.force) {
                state.phase = match blocked {
                    Blocked::Refused(r) => Phase::Refused(r),
                    Blocked::Unwritable(e) => Phase::Failed(e.to_string()),
                };
                return Next::Done(Outcome::Failed);
            }
            if opts.yes {
                state.confirm()
            } else if opts.interactive {
                state.phase = Phase::Confirm;
                Next::Ask
            } else {
                state.phase = Phase::Failed(
                    "no interactive terminal: pass --yes to update without asking".into(),
                );
                Next::Done(Outcome::Failed)
            }
        }
        Msg::Release(Err(e)) => {
            state.phase = Phase::Failed(e);
            Next::Done(Outcome::Failed)
        }
        Msg::Progress(got, total) => {
            state.phase = Phase::Working(Step::Download { got, total });
            Next::Wait
        }
        Msg::Step(s) => {
            state.phase = Phase::Working(s);
            Next::Wait
        }
        Msg::Finished(Ok(verified)) => {
            state.verified = verified;
            state.phase = Phase::Done;
            // el aviso no debe sobrevivir a la actualización que lo respondió
            if let Some(dir) = opts.state_dir.as_deref() {
                CheckCache::forget(dir);
            }
            Next::Done(state.outcome())
        }
        Msg::Finished(Err(e)) => {
            state.phase = Phase::Failed(e);
            Next::Done(Outcome::Failed)
        }
    }
}

/// La release pedida: la última, o la de `--to`.
pub fn fetch(source: &dyn Source, to: Option<&Version>) -> Result<Release, UpdateError> {
    match to {
        Some(v) => source.release(v),
        None => source.latest(),
    }
}

/// Descarga el archivo, lo comprueba y pone el binario en su sitio. Devuelve
/// si el checksum se pudo comprobar.
pub fn install_release(
    source: &dyn Source,
    release: &Release,
    target: &Target,
    dest: &Path,
    report: &mut dyn FnMut(Msg),
) -> Result<bool, UpdateError> {
    let asset = release.asset_for(target)?.clone();
    let archive = source.download(&asset.url, &mut |got, total| {
        report(Msg::Progress(got, total))
    })?;
    report(Msg::Step(Step::Verify));
    // una release sin checksums.txt sigue siendo una descarga HTTPS del
    // repositorio: se sigue, y la vista dice que no se pudo verificar
    let verified = match release.checksums_url() {
        Some(url) => match source.text(url) {
            Ok(sums) => verify_sha256(&archive, &sums, &asset.name)?,
            Err(_) => false,
        },
        None => false,
    };
    report(Msg::Step(Step::Install));
    let binary = unpack(&archive, target)?;
    install::install_binary(&binary, dest)?;
    Ok(verified)
}

/// Lo que un conductor de un solo hilo necesita de la vista.
pub trait View {
    /// El estado ha cambiado (fase o progreso).
    fn show(&mut self, state: &State);
    /// `Phase::Confirm`: ¿se instala?
    fn confirm(&mut self, state: &State) -> bool;
}

/// Todo seguido en este hilo: pregunta, decide, pregunta a la vista si hace
/// falta, descarga e instala. Cada cambio de estado pasa por `view.show`.
pub fn drive(
    opts: &Options,
    source: &dyn Source,
    state: &mut State,
    view: &mut dyn View,
) -> Outcome {
    view.show(state);
    let first = Msg::Release(fetch(source, opts.to.as_ref()).map_err(|e| e.to_string()));
    let mut next = apply(first, state, opts);
    loop {
        match next {
            Next::Done(outcome) => {
                view.show(state);
                return outcome;
            }
            Next::Ask => {
                view.show(state);
                if view.confirm(state) {
                    next = state.confirm();
                } else {
                    let outcome = state.cancel();
                    view.show(state);
                    return outcome;
                }
            }
            Next::Install => {
                view.show(state);
                let Some(release) = state.release.clone() else {
                    return Outcome::Failed;
                };
                let (target, dest) = (state.target, state.dest.clone());
                let res = install_release(source, &release, &target, &dest, &mut |m| {
                    apply(m, state, opts);
                    view.show(state);
                });
                next = apply(Msg::Finished(res.map_err(|e| e.to_string())), state, opts);
            }
            // fuera de la instalación nadie manda progreso
            Next::Wait => return state.outcome(),
        }
    }
}

/// Por qué la actualización se para antes de descargar nada.
#[derive(Debug)]
enum Blocked {
    /// No es nuestro: de cargo, o una compilación bajo `target/`.
    Refused(Refusal),
    /// Es nuestro, pero no se puede escribir en la carpeta.
    Unwritable(UpdateError),
}

/// Si este binario es nuestro para sustituir, y si se puede escribir ahí.
fn allowed(dest: &Path, force: bool) -> Result<(), Blocked> {
    if !force {
        if let Some(r) = install::classify(dest).refusal() {
            return Err(Blocked::Refused(r));
        }
    }
    install::check_writable(dest).map_err(Blocked::Unwritable)
}

fn remember(state_dir: Option<&Path>, latest: &Version) {
    if let Some(dir) = state_dir {
        let _ = CheckCache::new(latest, chrono::Utc::now()).save(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{tar_gz, test_dir, Asset};
    use super::*;
    use sha2::{Digest, Sha256};
    use std::cell::{Cell, RefCell};

    const ASSET: &str = "claude-agents-linux-x86_64.tar.gz";

    fn v(s: &str) -> Version {
        s.parse().unwrap()
    }

    fn target() -> Target {
        Target::new("linux", "x86_64", false)
    }

    fn state_at(dest: PathBuf) -> State {
        State::new(v("1.0.0"), target(), dest)
    }

    fn release(version: &str, with_sums: bool) -> Release {
        let mut assets = vec![Asset {
            name: ASSET.into(),
            url: format!("https://example/v{version}/{ASSET}"),
            size: 2_000_000,
        }];
        if with_sums {
            assets.push(Asset {
                name: "checksums.txt".into(),
                url: format!("https://example/v{version}/checksums.txt"),
                size: 600,
            });
        }
        Release {
            version: v(version),
            tag: format!("v{version}"),
            notes: String::new(),
            url: String::new(),
            published_at: None,
            assets,
        }
    }

    fn opts(check_only: bool, yes: bool, interactive: bool) -> Options {
        Options {
            current: v("1.0.0"),
            to: None,
            check_only,
            yes,
            force: false,
            interactive,
            state_dir: None,
        }
    }

    /// GitHub de mentira: una release, su archivo y su checksums.txt.
    struct Fake {
        release: Result<Release, String>,
        archive: Vec<u8>,
        sums: Option<String>,
        downloads: Cell<usize>,
        asked_tag: RefCell<Option<String>>,
    }

    impl Fake {
        fn new(release: Release, binary: &[u8]) -> Self {
            let archive = tar_gz(&[("claude-agents", binary)]);
            let sums = format!("{:x}  {ASSET}\n", Sha256::digest(&archive));
            Self {
                release: Ok(release),
                archive,
                sums: Some(sums),
                downloads: Cell::new(0),
                asked_tag: RefCell::new(None),
            }
        }
    }

    impl Source for Fake {
        fn latest(&self) -> Result<Release, UpdateError> {
            self.release.clone().map_err(UpdateError::Http)
        }
        fn release(&self, v: &Version) -> Result<Release, UpdateError> {
            *self.asked_tag.borrow_mut() = Some(v.tag());
            self.latest()
        }
        fn text(&self, _url: &str) -> Result<String, UpdateError> {
            self.sums.clone().ok_or(UpdateError::NoRelease)
        }
        fn download(
            &self,
            _url: &str,
            on_progress: &mut dyn FnMut(u64, Option<u64>),
        ) -> Result<Vec<u8>, UpdateError> {
            self.downloads.set(self.downloads.get() + 1);
            let total = self.archive.len() as u64;
            on_progress(0, Some(total));
            on_progress(total, Some(total));
            Ok(self.archive.clone())
        }
    }

    /// Vista de mentira: apunta las fases y contesta lo que se le diga.
    struct Seen {
        phases: Vec<Phase>,
        answer: bool,
        asked: usize,
    }

    impl Seen {
        fn answering(answer: bool) -> Self {
            Self {
                phases: Vec::new(),
                answer,
                asked: 0,
            }
        }
    }

    impl View for Seen {
        fn show(&mut self, state: &State) {
            self.phases.push(state.phase.clone());
        }
        fn confirm(&mut self, _state: &State) -> bool {
            self.asked += 1;
            self.answer
        }
    }

    #[test]
    fn an_old_release_is_not_news() {
        let mut s = state_at(PathBuf::from("/usr/local/bin/claude-agents"));
        let next = apply(
            Msg::Release(Ok(release("0.9.0", true))),
            &mut s,
            &opts(false, false, true),
        );
        assert_eq!(next, Next::Done(Outcome::UpToDate));
        assert_eq!(s.phase, Phase::UpToDate);
        assert_eq!(s.next_version(), None);
    }

    #[test]
    fn with_a_terminal_a_new_release_waits_for_the_go_ahead() {
        let dir = test_dir("ask");
        let mut s = state_at(dir.join("claude-agents"));
        let next = apply(
            Msg::Release(Ok(release("1.1.0", true))),
            &mut s,
            &opts(false, false, true),
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(next, Next::Ask);
        assert_eq!(s.phase, Phase::Confirm);
        assert_eq!(s.next_version(), Some(&v("1.1.0")));
        assert_eq!(s.confirm(), Next::Install);
        assert_eq!(
            s.phase,
            Phase::Working(Step::Download {
                got: 0,
                total: Some(2_000_000)
            })
        );
    }

    #[test]
    fn without_a_terminal_it_needs_yes() {
        let dir = test_dir("piped");
        let mut s = state_at(dir.join("claude-agents"));
        let next = apply(
            Msg::Release(Ok(release("1.1.0", true))),
            &mut s,
            &opts(false, false, false),
        );
        assert_eq!(next, Next::Done(Outcome::Failed));
        assert!(matches!(&s.phase, Phase::Failed(e) if e.contains("--yes")));
        let mut s = state_at(dir.join("claude-agents"));
        let next = apply(
            Msg::Release(Ok(release("1.1.0", true))),
            &mut s,
            &opts(false, true, false),
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(next, Next::Install);
    }

    #[test]
    fn with_check_it_reports_and_touches_nothing() {
        // ni siquiera mira si puede escribir: --check sobre target/ vale
        let mut s = state_at(PathBuf::from("/x/target/debug/claude-agents"));
        let next = apply(
            Msg::Release(Ok(release("1.1.0", true))),
            &mut s,
            &opts(true, true, false),
        );
        assert_eq!(next, Next::Done(Outcome::Available(v("1.1.0"))));
        assert_eq!(s.phase, Phase::Available);
    }

    #[test]
    fn a_local_build_or_a_cargo_install_stops_before_downloading() {
        let mut dev = state_at(PathBuf::from(
            "/x/claude-agents/target/release/claude-agents",
        ));
        let next = apply(
            Msg::Release(Ok(release("1.1.0", true))),
            &mut dev,
            &opts(false, true, true),
        );
        assert_eq!(next, Next::Done(Outcome::Failed));
        assert!(matches!(&dev.phase, Phase::Refused(r) if r.fix == "make build"));

        let mut cargo = state_at(PathBuf::from("/Users/j/.cargo/bin/claude-agents"));
        apply(
            Msg::Release(Ok(release("1.1.0", true))),
            &mut cargo,
            &opts(false, true, true),
        );
        assert!(matches!(&cargo.phase, Phase::Refused(r) if r.fix.starts_with("cargo install")));
    }

    #[test]
    fn force_goes_over_a_build_but_not_over_a_missing_directory() {
        let mut o = opts(false, true, true);
        o.force = true;
        let dir = test_dir("force");
        let build = dir.join("target/debug");
        std::fs::create_dir_all(&build).unwrap();
        let mut s = state_at(build.join("claude-agents"));
        let next = apply(Msg::Release(Ok(release("1.1.0", true))), &mut s, &o);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(next, Next::Install);

        let mut gone = state_at(PathBuf::from("/no/such/dir/claude-agents"));
        let next = apply(Msg::Release(Ok(release("1.1.0", true))), &mut gone, &o);
        assert_eq!(next, Next::Done(Outcome::Failed));
        assert!(matches!(gone.phase, Phase::Failed(_)));
    }

    #[test]
    fn to_allows_a_downgrade_but_not_the_same_version_again_without_force() {
        let mut o = opts(false, false, true);
        o.to = Some(v("0.9.0"));
        assert!(o.worth_installing(&v("0.9.0")));
        o.to = Some(v("1.0.0"));
        assert!(!o.worth_installing(&v("1.0.0")));
        o.force = true;
        assert!(o.worth_installing(&v("1.0.0")));
        // sin --to ni --force, la misma versión no es noticia
        assert!(!opts(false, false, true).worth_installing(&v("1.0.0")));
    }

    #[test]
    fn a_github_failure_is_shown_and_ends_there() {
        let mut s = state_at(PathBuf::from("/usr/local/bin/claude-agents"));
        let next = apply(
            Msg::Release(Err("github: rate limit reached".into())),
            &mut s,
            &opts(false, false, true),
        );
        assert_eq!(next, Next::Done(Outcome::Failed));
        assert!(matches!(&s.phase, Phase::Failed(e) if e.contains("rate limit")));
        assert_eq!(Outcome::Failed.exit_code(), 1);
        assert_eq!(Outcome::Available(v("1.1.0")).exit_code(), 0);
        assert_eq!(Outcome::Cancelled.exit_code(), 0);
    }

    #[test]
    fn the_note_is_rewritten_on_check_and_forgotten_after_installing() {
        let dir = test_dir("note");
        let mut o = opts(true, false, false);
        o.state_dir = Some(dir.clone());
        let mut s = state_at(PathBuf::from("/usr/local/bin/claude-agents"));
        apply(Msg::Release(Ok(release("1.1.0", true))), &mut s, &o);
        assert_eq!(CheckCache::load(&dir).unwrap().latest, "1.1.0");
        // --to no es la última: no pisa la nota
        o.to = Some(v("0.9.0"));
        apply(Msg::Release(Ok(release("0.9.0", true))), &mut s, &o);
        assert_eq!(CheckCache::load(&dir).unwrap().latest, "1.1.0");
        s.release = Some(release("1.1.0", true));
        let next = apply(Msg::Finished(Ok(false)), &mut s, &o);
        assert_eq!(next, Next::Done(Outcome::Installed(v("1.1.0"))));
        assert!(!s.verified, "sin checksums.txt hay que decirlo");
        assert!(CheckCache::load(&dir).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drive_installs_after_the_yes_and_verifies_the_checksum() {
        let dir = test_dir("drive");
        let dest = dir.join("claude-agents");
        std::fs::write(&dest, b"old").unwrap();
        let fake = Fake::new(release("1.1.0", true), b"NEW!");
        let mut s = state_at(dest.clone());
        let mut view = Seen::answering(true);
        let outcome = drive(&opts(false, false, true), &fake, &mut s, &mut view);
        let installed = std::fs::read(&dest).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(outcome, Outcome::Installed(v("1.1.0")));
        assert_eq!(installed, b"NEW!");
        assert!(s.verified);
        assert_eq!(view.asked, 1);
        assert_eq!(view.phases.first(), Some(&Phase::Checking));
        assert!(view.phases.contains(&Phase::Confirm));
        assert!(view.phases.contains(&Phase::Working(Step::Verify)));
        assert!(view.phases.contains(&Phase::Working(Step::Install)));
        assert_eq!(view.phases.last(), Some(&Phase::Done));
    }

    #[test]
    fn drive_with_a_no_downloads_nothing() {
        let dir = test_dir("no");
        let dest = dir.join("claude-agents");
        std::fs::write(&dest, b"old").unwrap();
        let fake = Fake::new(release("1.1.0", true), b"NEW!");
        let mut s = state_at(dest.clone());
        let mut view = Seen::answering(false);
        let outcome = drive(&opts(false, false, true), &fake, &mut s, &mut view);
        let kept = std::fs::read(&dest).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(fake.downloads.get(), 0);
        assert_eq!(kept, b"old");
        assert_eq!(view.phases.last(), Some(&Phase::Cancelled));
    }

    #[test]
    fn drive_with_a_bad_checksum_leaves_the_old_binary() {
        let dir = test_dir("bad");
        let dest = dir.join("claude-agents");
        std::fs::write(&dest, b"old").unwrap();
        let mut fake = Fake::new(release("1.1.0", true), b"NEW!");
        fake.sums = Some(format!("{}  {ASSET}\n", "0".repeat(64)));
        let mut s = state_at(dest.clone());
        let mut view = Seen::answering(true);
        let outcome = drive(&opts(false, true, false), &fake, &mut s, &mut view);
        let kept = std::fs::read(&dest).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(outcome, Outcome::Failed);
        assert_eq!(kept, b"old");
        assert!(matches!(&s.phase, Phase::Failed(e) if e.contains("checksum mismatch")));
        assert_eq!(view.asked, 0, "con --yes no se pregunta");
    }

    #[test]
    fn drive_without_checksums_installs_but_says_unverified() {
        let dir = test_dir("nosums");
        let dest = dir.join("claude-agents");
        let fake = Fake::new(release("1.1.0", false), b"NEW!");
        let mut s = state_at(dest.clone());
        let outcome = drive(
            &opts(false, true, false),
            &fake,
            &mut s,
            &mut Seen::answering(true),
        );
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(outcome, Outcome::Installed(v("1.1.0")));
        assert!(!s.verified);
    }

    #[test]
    fn drive_with_to_asks_for_that_tag() {
        let fake = Fake::new(release("0.9.0", true), b"OLD!");
        let mut o = opts(true, false, false);
        o.to = Some(v("0.9.0"));
        let mut s = state_at(PathBuf::from("/usr/local/bin/claude-agents"));
        let outcome = drive(&o, &fake, &mut s, &mut Seen::answering(false));
        assert_eq!(outcome, Outcome::Available(v("0.9.0")));
        assert_eq!(fake.asked_tag.borrow().as_deref(), Some("v0.9.0"));
    }

    #[test]
    fn home_is_shortened_in_the_path() {
        let home = std::env::var("HOME").unwrap_or_default();
        if home.is_empty() {
            return;
        }
        let s = state_at(PathBuf::from(format!("{home}/.local/bin/claude-agents")));
        assert_eq!(s.dest_display(), "~/.local/bin/claude-agents");
    }
}
