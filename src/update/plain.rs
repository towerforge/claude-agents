//! La vista de texto de `claude-agents update`: unas líneas por stdout (los
//! errores por stderr) y la pregunta por stdin. Es lo que sale sin terminal, o
//! si el panel (`tui::update`) no se deja abrir; con terminal manda el panel.

use std::io::{BufRead, Write};

use super::flow::{Phase, State, Step, View};
use super::APP;

pub struct Plain {
    /// Hay terminal: se pinta el progreso en la misma línea y se pregunta.
    pub interactive: bool,
    /// La última fase dicha, para no repetir línea con cada trozo descargado.
    last: Option<std::mem::Discriminant<Phase>>,
    last_step: Option<std::mem::Discriminant<Step>>,
    /// La línea del progreso está a medias (sin salto de línea).
    progress_open: bool,
}

impl Plain {
    pub fn new(interactive: bool) -> Self {
        Self {
            interactive,
            last: None,
            last_step: None,
            progress_open: false,
        }
    }
}

/// Lo que dice cada fase, una vez.
pub fn line(state: &State) -> String {
    let next = state
        .next_version()
        .map(ToString::to_string)
        .unwrap_or_default();
    match &state.phase {
        Phase::Checking => format!("{APP} {} · checking for a new release…", state.current),
        Phase::UpToDate => format!("{APP} {} is the latest version", state.current),
        Phase::Available => format!("{APP} {next} is available · run `{APP} update`"),
        Phase::Confirm => {
            let mut s = format!("{APP} {} → {next}", state.current);
            let size = state
                .asset_size()
                .map(|b| format!(" ({:.1} MB)", b as f64 / 1_000_000.0))
                .unwrap_or_default();
            s.push_str(&format!("\n  package     {}{size}", state.asset_name()));
            s.push_str(&format!("\n  install to  {}", state.dest_display()));
            if let Some(r) = &state.release {
                for h in r.highlights(5) {
                    s.push_str(&format!("\n  · {h}"));
                }
            }
            s
        }
        Phase::Working(Step::Download { .. }) => format!(
            "{APP} {} → {next} · downloading {}",
            state.current,
            state.asset_name()
        ),
        Phase::Working(Step::Verify) => "verifying sha-256".into(),
        Phase::Working(Step::Install) => format!("installing to {}", state.dest_display()),
        Phase::Done => format!(
            "installed {APP} {next} to {}{}",
            state.dest_display(),
            if state.verified {
                ""
            } else {
                " (checksum not published: not verified)"
            }
        ),
        Phase::Cancelled => "cancelled · nothing was touched".into(),
        Phase::Refused(r) => format!("error: {r}"),
        Phase::Failed(e) => format!("error: {e}"),
    }
}

impl View for Plain {
    fn show(&mut self, state: &State) {
        let phase = std::mem::discriminant(&state.phase);
        let step = match &state.phase {
            Phase::Working(s) => Some(std::mem::discriminant(s)),
            _ => None,
        };
        let new_line = self.last != Some(phase) || self.last_step != step;
        // sin terminal no se dice «comprobando»: solo el resultado
        if new_line && !(state.phase == Phase::Checking && !self.interactive) {
            if std::mem::take(&mut self.progress_open) {
                println!();
            }
            let text = line(state);
            match state.phase {
                Phase::Refused(_) | Phase::Failed(_) => eprintln!("{text}"),
                _ => println!("{text}"),
            }
        }
        if let (true, Phase::Working(Step::Download { got, total })) =
            (self.interactive, &state.phase)
        {
            let pct = total
                .filter(|t| *t > 0)
                .map(|t| format!(" {:>3}%", got * 100 / t))
                .unwrap_or_default();
            print!("\r  {:.1} MB{pct}", *got as f64 / 1_000_000.0);
            let _ = std::io::stdout().flush();
            self.progress_open = true;
        }
        self.last = Some(phase);
        self.last_step = step;
    }

    fn confirm(&mut self, _state: &State) -> bool {
        print!("install it? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().lock().read_line(&mut answer).is_err() {
            return false;
        }
        matches!(answer.trim(), "y" | "Y" | "yes")
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Release, Target};
    use super::*;
    use std::path::PathBuf;

    fn state(phase: Phase) -> State {
        let mut s = State::new(
            "1.0.0".parse().unwrap(),
            Target::new("macos", "aarch64", false),
            PathBuf::from("/usr/local/bin/claude-agents"),
        );
        s.release = Some(Release {
            version: "1.1.0".parse().unwrap(),
            tag: "v1.1.0".into(),
            notes: "* feat: update command by @x in y\n".into(),
            url: String::new(),
            published_at: None,
            assets: vec![super::super::Asset {
                name: "claude-agents-macos-aarch64.tar.gz".into(),
                url: String::new(),
                size: 2_100_000,
            }],
        });
        s.phase = phase;
        s
    }

    #[test]
    fn each_phase_says_what_happened() {
        assert_eq!(
            line(&state(Phase::UpToDate)),
            "claude-agents 1.0.0 is the latest version"
        );
        assert_eq!(
            line(&state(Phase::Available)),
            "claude-agents 1.1.0 is available · run `claude-agents update`"
        );
        let confirm = line(&state(Phase::Confirm));
        assert!(
            confirm.starts_with("claude-agents 1.0.0 → 1.1.0"),
            "{confirm}"
        );
        assert!(confirm.contains("claude-agents-macos-aarch64.tar.gz (2.1 MB)"));
        assert!(confirm.contains("· feat: update command"));
        let mut done = state(Phase::Done);
        assert_eq!(
            line(&done),
            "installed claude-agents 1.1.0 to /usr/local/bin/claude-agents"
        );
        done.verified = false;
        assert!(line(&done).ends_with("(checksum not published: not verified)"));
        assert_eq!(
            line(&state(Phase::Failed("github: HTTP 502".into()))),
            "error: github: HTTP 502"
        );
    }
}
