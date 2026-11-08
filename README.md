<p align="center">
  <img src="assets/readme/hero.svg" alt="claude-agents: which Claude Code subagents are working, and on what" width="100%">
</p>

<p align="center">
  <a href="#install"><img alt="Rust 1.88 or newer" src="https://img.shields.io/badge/rust-1.88%2B-d77757?style=flat-square&logo=rust&logoColor=141413"></a>
  <a href="LICENSE"><img alt="MIT license" src="https://img.shields.io/badge/license-MIT-d77757?style=flat-square"></a>
  <a href="https://github.com/towerforge/claude-agents/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/towerforge/claude-agents/total?style=flat-square&color=d77757"></a>
  <img alt="macOS and Linux" src="https://img.shields.io/badge/platforms-macOS%20%C2%B7%20Linux-d77757?style=flat-square">
</p>

<p align="center">
  Lists the Claude Code subagents on disk and shows which ones are running, what they were asked to do, how long they worked and the tokens they used.
</p>

<p align="center">
  <img src="assets/readme/screenshot.svg" alt="claude-agents showing two projects with running and finished subagents" width="900">
</p>

## Install

macOS and Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/towerforge/claude-agents/main/install.sh | sh
claude-agents
```

No configuration: it looks for agents under your home directory (`~/.claude/agents/*.md` and every `<project>/.claude/agents/*.md`) and reads the transcripts in `~/.claude/projects/`.

<details>
<summary><b>Installer options, manual download, from source, updating</b></summary>

[`install.sh`](install.sh) downloads the archive for your OS and architecture from the latest release, checks its SHA-256 against `checksums.txt` (a missing checksum warns, a mismatch stops it) and asks for `Enter` before installing. It needs `curl` and `tar`.

| Variable | Effect | Default |
|---|---|---|
| `CLAUDE_AGENTS_VERSION` | Installs that version (`0.2.0` or `v0.2.0`) | the latest release |
| `CLAUDE_AGENTS_INSTALL_DIR` | Directory to install into | `~/.local/bin`, or `/usr/local/bin` as root |
| `CLAUDE_AGENTS_VARIANT` | `musl` (static, no glibc) or `gnu`. Linux only | `musl` if `ldd` reports musl, `gnu` otherwise |
| `NO_COLOR` | No colors in the installer's output | |

**Manual download:** the [releases page](https://github.com/towerforge/claude-agents/releases/latest) has archives for Linux x86_64 and ARM64 (glibc and musl) and macOS Intel and Apple Silicon; see [docs/releases.md](docs/releases.md#archives). No Windows build: use WSL.

**From source** (Rust 1.88 or newer): `cargo install --git https://github.com/towerforge/claude-agents`.

**Updating:** run the installer again.

</details>

## Features

- **Global and project agents**, grouped by project; a project agent only matches sessions inside its project.
- **Live monitor** of what is running and what finished within the window: task, current tool, time, context and tokens. Refreshes every 2 s.
- **Claude Code's own agents and workflow steps** too, labeled `[claude]` and `[workflow]` next to your `[own]` ones.
- **Time worked per day**, without pauses over 5 minutes.
- **Tokens, context and a cost estimate** at API list prices (only for known models).
- **Who launched each run**: the session and its first prompt, or another run.
- **Ranking** by time, tokens or runs over 7 days, 30 days or all time.
- **Kept history**: finished runs survive Claude Code's transcript cleanup.
- **`--plain`** prints a table and exits; so does piping.

## Using claude-agents

Three views, as tabs in the header:

- **active** (default): one block per agent and project with `task` and `now`/`last`; running ones first, marked `✦`.
- **all**: every agent on disk by project, with a 14-day bar, time worked, runs and last use.
- **ranking**: agents with work in the period, from most to least.

`Enter` opens the agent's details (in all and ranking) or the run's (in active). Every view in detail: [docs/interface.md](docs/interface.md); all the keys: [keys and formats](docs/interface.md#keys-and-formats).

### Flags

```sh
claude-agents                       # interactive screen, 10 min window, whole home
claude-agents -m 60                 # last hour
claude-agents -r ~/code -r ~/work   # narrows the search
claude-agents -p -v                 # the table, plus the open sessions with pid and command
```

| Flag | Meaning | Default |
|---|---|---|
| `-r`, `--root <DIR>` | Directories to search for `.claude/agents/*.md` (repeatable). `~/.claude/agents` is always added | the home directory |
| `-m`, `--minutes <N>` | Window in minutes: an agent is active if it was used within it or has a run in progress | `10` |
| `-v`, `--verbose` | Also shows open Claude sessions, with pid and command (only with `--plain`) | |
| `-p`, `--plain` | Prints the table and exits, instead of opening the interactive screen. This also happens when the output is not a terminal | |
| `-V`, `--version` | Prints the version and exits | |

### `--plain`

```
STATE     NAME      LAST USED     WORKED  PATH
RUNNING   reviewer  2 min ago  1 h 4 min  ~/code/shop/api/.claude/agents/reviewer.md
ACTIVE    backend   6 min ago     35 min  ~/code/shop/api/.claude/agents/backend.md
inactive  docs      3 d ago        4 min  ~/code/shop/web/.claude/agents/docs.md

3 agents, 2 active (window: 10 min). Open Claude sessions: 2
```

Details: [docs/interface.md](docs/interface.md#the---plain-table).

## How it works

It only reads Claude Code's files, plus `ps` for open sessions:

- [Active](docs/how-it-works.md#how-it-decides-an-agent-is-active): an `Agent` call within the window, or a run in progress.
- [Time worked](docs/how-it-works.md#how-worked-time-is-calculated): gaps between transcript lines, minus pauses over 5 minutes.
- [Tokens, context and cost](docs/how-it-works.md#tokens-context-and-cost): from `message.usage`.
- [Who launched whom](docs/how-it-works.md#who-launched-whom): the meta's `toolUseId`.
- [Kept history](docs/how-it-works.md#kept-history): `~/.cache/claude-agents/runs.jsonl`, the only file it writes.

Limitations: [docs/how-it-works.md](docs/how-it-works.md#limitations).

## FAQ

**A run shows as finished but it is still working.** A run that writes nothing for 10 minutes (a pending permission, a long tool) shows as interrupted until it writes again; a workflow step, after 2 minutes.

**Why don't `general-purpose` or `Explore` show in all or in the ranking?** They have no definition on disk. They show in active, but add no time to any agent.

**Is the cost what I pay?** No. It is an estimate at API list prices; on a subscription you do not pay per token, and models missing from the table get no cost.

**An agent I used shows as `never`.** Old `Agent` calls are not kept once Claude Code deletes their transcripts; only runs are.

**I cannot select text with the mouse.** Hold `Shift` while dragging (`Option` in iTerm2).

## Contributing

Open pull requests against `beta`, with `make check` green.

## License

[MIT](LICENSE)

<p align="center">
  <br>
  <img src="assets/readme/mark.svg" width="48" alt="" style="image-rendering: pixelated">
</p>
