//! Sesiones de Claude Code abiertas, a partir de la tabla de procesos.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub pid: u32,
    pub command: String,
}

use anyhow::{Context, Result};
use std::process::Command;

/// Filtra la salida de `ps -axo pid=,command=` y se queda con los procesos cuyo
/// ejecutable se llama `claude`, descartando el pid propio.
pub fn parse_ps(output: &str, own_pid: u32) -> Vec<Session> {
    output
        .lines()
        .filter_map(|line| {
            let (pid, command) = line.trim_start().split_once(' ')?;
            let pid: u32 = pid.parse().ok()?;
            let command = command.trim();
            let binary = command.split_whitespace().next()?;
            let name = binary.rsplit('/').next()?;
            (name == "claude" && pid != own_pid).then(|| Session {
                pid,
                command: command.to_string(),
            })
        })
        .collect()
}

/// Sesiones de Claude Code abiertas ahora mismo en esta máquina.
pub fn open_sessions() -> Result<Vec<Session>> {
    let out = Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .context("cannot run ps")?;
    Ok(parse_ps(
        &String::from_utf8_lossy(&out.stdout),
        std::process::id(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS: &str = "\
  712 /Applications/Claude.app/Contents/MacOS/Claude
 1229 /Applications/Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper --type=gpu-process
19737 /Users/x/.vscode/extensions/anthropic.claude-code-2.1.318-darwin-arm64/resources/native-binary/claude --output-format stream-json
31898 /bin/zsh -c source /Users/x/.claude/shell-snapshots/snapshot.sh
30679 claude
19754 claude --resume abc
 4242 /usr/local/bin/claude-agents --minutes 5
";

    #[test]
    fn keeps_only_processes_whose_binary_is_claude() {
        let pids: Vec<u32> = parse_ps(PS, 0).into_iter().map(|s| s.pid).collect();
        assert_eq!(pids, vec![19737, 30679, 19754]);
    }

    #[test]
    fn keeps_the_full_command_line() {
        let s = parse_ps(PS, 0);
        assert_eq!(s[2].command, "claude --resume abc");
    }

    #[test]
    fn excludes_own_pid() {
        let pids: Vec<u32> = parse_ps(PS, 30679).into_iter().map(|s| s.pid).collect();
        assert_eq!(pids, vec![19737, 19754]);
    }
}
