# claude-agents

Mini CLI that lists every Claude Code subagent on disk
(`~/.claude/agents/*.md` and `<project>/.claude/agents/*.md`), marks which ones
were invoked recently or are working right now, and tells you when each one was
last used and how long it has worked, in total and day by day, plus the tokens
each run used, how full its context is and who launched it. In a terminal it
opens a moon-style screen that refreshes itself (a live monitor of what is
running and what happened in the window, a per-project catalog, a ranking by
time worked, tokens or runs, a detail screen for each agent with its runs, and
a detail screen for each run, running or finished); when piped or with
`--plain` it prints a table and exits.

It only reads Claude Code's files, with one exception: it keeps finished runs
in its own [history cache](#kept-history), so they survive Claude Code's
cleanup of old transcripts. That file is the only thing it writes.

The Makefile is the entry point (`make help` lists everything):

```
make check                        # fmt --check + clippy -D warnings + tests
make run ARGS="-m 60 -v"          # debug: last hour, and lists the open sessions
make install                      # cargo install from this checkout → ~/.cargo/bin/claude-agents
make package                      # dist/claude-agents-<platform>-<arch>.tar.gz
```

Packaging and releasing are covered in [Packaging and releases](#packaging-and-releases).

Binary flags (`claude-agents --help`):

```
claude-agents                                  # interactive screen (opens on active), 10 min window, whole home
claude-agents -m 60                            # last hour
claude-agents -r ~/Offing -r ~/Towerforge      # narrows the search
claude-agents --plain                          # the table, no screen (same as when piped)
claude-agents -p -v                            # the table and, below it, the sessions with pid and command
```

| Flag | Meaning | Default |
|---|---|---|
| `-r`, `--root <DIR>` | Directories to search for `.claude/agents/*.md` (repeatable) | the home directory |
| `-m`, `--minutes <N>` | Window in minutes: an agent is active if it was used within it or has a run in progress | `10` |
| `-v`, `--verbose` | Also shows open Claude sessions, with pid and command (only with `--plain`) | |
| `-p`, `--plain` | Prints the table and exits, instead of opening the interactive screen. This also happens when the output is not a terminal | |
| `-V`, `--version` | Prints the version and exits | |

## Install

```
curl -fsSL https://raw.githubusercontent.com/towerforge/claude-agents/main/install.sh | sh
```

[`install.sh`](install.sh) detects the OS and architecture, downloads the
matching archive from the latest GitHub release, checks its SHA-256 against the
release's `checksums.txt` (if that file or the archive's line is missing, it
warns and installs unverified; a mismatch stops it) and installs the binary.
Before touching anything it shows what it will install and where, and waits
for `Enter` (or asks to confirm a reinstall of the same version). It needs
`curl` and `tar`.

| Variable | Effect | Default |
|---|---|---|
| `CLAUDE_AGENTS_VERSION` | Installs that version (`0.2.0` or `v0.2.0`) | the latest release |
| `CLAUDE_AGENTS_INSTALL_DIR` | Directory to install into | `~/.local/bin`, or `/usr/local/bin` as root |
| `CLAUDE_AGENTS_VARIANT` | `musl` (static, no glibc) or `gnu`. Linux only | `musl` if `ldd` reports musl, `gnu` otherwise |
| `NO_COLOR` | No colors in the installer's output | |

```
curl -fsSL https://raw.githubusercontent.com/towerforge/claude-agents/main/install.sh | CLAUDE_AGENTS_VERSION=0.1.0 sh
```

There is no `update` command: running the script again is the update (it shows
`vOLD → vNEW`). If the install directory is not in your `PATH`, it says so and
prints the `export` line to add.

Prebuilt archives, one per platform, attached to every release:

| Archive | Platform |
|---|---|
| `claude-agents-linux-x86_64.tar.gz` | Linux x86_64, glibc |
| `claude-agents-linux-x86_64-musl.tar.gz` | Linux x86_64, static (musl) |
| `claude-agents-linux-aarch64.tar.gz` | Linux ARM64, glibc |
| `claude-agents-linux-aarch64-musl.tar.gz` | Linux ARM64, static (musl) |
| `claude-agents-macos-x86_64.tar.gz` | macOS Intel |
| `claude-agents-macos-aarch64.tar.gz` | macOS Apple Silicon |

No Windows build: open sessions are read with `ps`. Use it from WSL (the
installer stops on Windows and says so). On any other architecture, build from
source with `cargo install --git https://github.com/towerforge/claude-agents`.

## The screen

Three views, shown as tabs in the header (the open one painted over `claude`),
each one for a different purpose:

- **active** (default, the one that opens at startup): a live monitor of what is
  happening and what has happened in the window (`-m`). Everything is grouped by
  project, with one row per agent: in each group, what is running first and,
  below it, what finished within the window.
- **all**: the catalog, every agent on disk grouped by project, with its
  history.
- **ranking**: the agents with work in the period, from the one that worked the
  most to the one that worked the least, without groups.

`Tab` cycles active → all → ranking; `a` toggles between active and all (from
the ranking, it goes to active).

On the right of the header: in active, `window N min · N running · N in window ·
N sessions · N tok` (`N running` are the rows, that is, the agents, with at
least one run in progress, and `N in window` those that finished within it,
both without filter and counting rows, not runs; `N tok` are the tokens of the
rows that show, with the filter applied, and is left out when 0); in all,
`window N min · N agents · N running · N sessions` (without `· N running` if
nothing is running); in the ranking, `<period> · <total> in N runs · N tok`
(the tokens of the period, also left out when 0). On narrow screens it drops
the sessions first (in active the tokens stay after them), then the tokens and
then the counts, never the tabs. The second line carries the
filter (or the read error) on the left and, on the right, `sort: worked` only
in «all» and when it is not the default order.

The version (`v0.1.0`, without the name) goes in gray on the last line, flush
right with one free column of margin; with the scroll counter it reads
`1-5/11  v0.1.0`. It shows in the three views and in both detail screens (these,
without the counter). The keys take priority: the version is omitted if it does
not fit next to them (keys width + right-hand width + 2 <= screen width).

The history is refreshed every 2 s and the definitions every 30 s or on pressing
`r`. At startup it shows `loading agents…` until the definitions arrive.

### Active: the monitor

(Excerpt of a render with made-up data, at 110 columns: two of the four groups.
`visual` is a workflow step that is not one of your agents and has just started,
with no token data yet; `frontend` is a finished
call with no run, so every figure is `—`; `general-purpose` is over 85% of its
context, painted amber.)

```
  active   all   ranking                      window 10 min · 3 running · 4 in window · 4 sessions · 1.5M tok

         agent                                                               time        ctx   tokens   tools
 …
 station · 2 running · 1 recent ─────────────────────────────────────────────────────────────────────────────
    3. ✦ visual [workflow]                                                 <1 min          —        —       0
         task     paint the active screen
         now      —

    4. ✦ backend [own]                                                     10 min   ▰▰▰▱ 71%     412k       7
         task     —
         now      —

    5.   frontend [own]                                                 2 min ago          —        —       —
         task     —
         last     —

 other · 1 running ──────────────────────────────────────────────────────────────────────────────────────────
    6. ✦ general-purpose [claude]                                           1 min   ▰▰▰▰ 92%     1.1M      37
         task     Independent review of the uncommitted change in /Users/x/other
         now      Bash · git diff --stat
 …
 ↑↓ move · enter details · tab mode · / filter · q quit                                                v0.1.0
```

A single list, grouped by project and numbered continuously across all groups.
Each **agent** is a three-line block that counts as a single row, without a
path: all its runs (and calls without a run) from that project within the
window, plus the running ones, are merged into that row. A blank line separates
one block from the next, and another one goes before each group rule except the
first (never after the last block, never two in a row); the blank is not part
of any block, so the cursor and the scroll still keep the whole three-line
block on screen. Above them, once and in gray, the column header (`agent`,
`time`, `ctx`, `tokens`, `tools`), with the first group rule right under it.
There are no sections or a separate timeline.

- **What shows.** All **running** runs, with no cap (the list scrolls), and all
  **finished** runs within the window. A finished run is one that is not
  running (an interrupted one too) and whose last step falls in the window, of
  any kind: defined agents, Claude Code agents (`general-purpose`, `Explore`…)
  and workflow steps. `Agent` calls in the window that do not link to any run
  also show: they appear as a finished run with no steps or tools (`tools` =
  `—`, `last` = `—`) and their task is the first clean line of the call's
  prompt or, if there is none, its `description`. A call and its run are one
  and the same. All are grouped by agent (see «One row per agent»).
- **One row per agent.** The key is (project, agent). If the run resolves to a
  defined agent, the agent is that one: a direct launch and a workflow step with
  the same agent share a row (the `via workflow` label does not count). With no
  defined agent, the key is (kind, name): a `visual [claude]` and a
  `visual [workflow]` are different rows, just like an own agent and a
  homonymous `[claude]` one. The row is represented by one run: the most
  recent running one (by start) or, if there is none, the most recent finished
  one; the state, time, `tools`, task and `now`/`last` come from it. The row is
  running if any of its runs is. If it groups more than one, the label carries
  ` ×N` (see below).
- **Order.** Within a group, running rows first (the one that started latest on
  top, by its representative run) and then finished ones (the one that ended
  latest on top). Groups are ordered by their most recent activity: the start of
  their newest running row or the end of their newest finished one; on a tie,
  by folder.
- **Group rule**: ` folder · N running · N recent ───`, each part only if
  greater than 0. It counts rows (agents), not runs. The folder goes in
  `claude-soft` and the rest in gray; if two groups have the same folder they
  are prefixed with the parent folder, as in «all»; `global` if there is no
  `cwd`.
- **Line 1** of a block: cursor `❯` and number; a 2-column slot for the star
  (`✦` in `claude` if running, alternating with `✧` every half second; blank if
  finished); the name; a gray (`ink-muted`) label saying where the agent comes
  from (`[own]` for one of the user's own agents; `[own] via workflow` for a
  workflow step whose `description` is one of them: `docs [own] via workflow`;
  `[workflow]` for a workflow step that is not one of the user's agents;
  `[claude]` for a Claude Code one) and, if the row merges more than one run,
  ` ×N` in gray at the end of the label (`docs [own] ×5`, `docs [own] via
  workflow ×3`, `visual [workflow] ×3`; with a single run it does not show);
  and on the right, flush
  to the edge, `time`, `ctx`, `tokens` and `tools` (the number only). `time` is
  `N min` (what has worked) for running ones and `N min ago` (since the last
  step) for finished ones. `ctx` is how full the context of the row's
  representative run is, only while it runs: a 4-step gauge where each started
  quarter fills a step, and the percentage (`▰▰▰▱ 71%`), amber from 85%; `—` if
  the row is finished or there is no data yet. `tokens` is the sum of the
  tokens of **all** the runs merged into the row, in short form, or `—` if 0
  (see [Tokens, context and cost](#tokens-context-and-cost)). The star slot is always reserved, so names stay aligned between
  running and finished ones, and it is never removed for narrowness (the label
  yields first, then the name, cut with `…`).
- **Line 2**, indented under the name: `task` and what it was asked to do.
- **Line 3**: `now` and the last tool if running (`Bash · git diff --stat`,
  `Edit · README.md`); `last` and the last tool if finished. `—` in gray if the
  datum is missing, also in `task`.

Each value goes on a single line, cut with `…` if it does not fit.

Colors. The name depends only on the state, whatever the kind: running,
`claude-soft` in bold (a `[claude]` one too); finished, plain `ink` without bold
(an `[own]` one too), and its number also in `ink`, without tint even when
selected. What is running is told apart by the blinking star, by the name color
and by `N min` versus `N min ago`. The `agent` column header and lines 2 and 3
are indented under the name, which starts right after the star (9 columns with
fewer than 100 runs in the list; it grows with the digits of the number).

Who each run is:

- a **defined agent** launched directly: its name, with `[own]`;
- a **workflow step** (`agentType` `workflow-subagent`): if its `description`
  is the name of a defined agent, that agent (label `[own] via workflow`);
  otherwise it is not one of the user's agents: the `description` as the name
  (or `workflow` if it has none), with the `[workflow]` label. In a workflow the
  `description` is the name, not the task;
- any other kind (`general-purpose`, `Explore`…, or a project agent launched
  from outside its project): its `agentType`, with the `[claude]` label.

The task is the first content line of the run's first message (see [What is
read from each run](#what-is-read-from-each-run)); if it could not be
extracted, it falls back to the meta's `description`, except in a workflow,
where it stays `—`. For a call without a run, the call's own (above).

Blocks narrow down: below 70 columns `ctx`, `tokens` and `tools` and their
headers are removed; below 50, the labels too (with their ` ×N`). If the whole label does not fit it is
lost in parts, in this priority order: name > origin (`[own]`, `[workflow]`,
`[claude]`) > ` ×N` > `via workflow` (`via workflow` drops first, then ` ×N`,
then the origin and, if that does not fit either, nothing). The name is cut with `…` before the
figures.

If there is nothing running or in the window, the monitor says so and points to
the last use of any defined agent:

```
 nothing in the last N min
 last use: <agent> from <short project> X ago · tab to see all
```

(or `no use recorded · tab to see all`). If it is the filter that leaves it
empty, it says `adjust the filter or press tab`.

`Enter` opens the [run details](#run-details) on **any** row: running or
finished, of any kind, and also a call without a run (a minimal screen, without
steps and with `ended … ago`); it opens the run that represents the row. The
other runs merged into it are browsed from the details screen with `←` `→`.
From active, `Enter` no longer opens the agent details: only `a` inside the run
details does, and only if it has a defined agent. The footer paints `enter
details` dimmed only if the cursor is not on a row. The digits, the cursor and
`↑` `↓` follow the grouped order (continuous across groups). `/` filters by
name, project or task over all rows (running and finished); a group left
without rows does not show and its rule counters are recomputed. `o` does
nothing here: the order is the one above.

### All: the catalog

```
  active   all   ranking                                                          window 60 min · 37 agents · 4 sessions

                                                                 last 14 days    worked   runs        last used █
 station · 6 agents ───────────────────────────────────────────   ···········⣶⣤⣿  6 h 18 min     63      5 min ago █
 ❯  1. docs                                                       ···········⣿⣤⣿      40 min     17      5 min ago █
    2. kiosk                                                      ···········⣿⣀⣶  1 h 45 min     11     19 min ago █
    3. visual                                                     ···········⣿⣤⣿  1 h 17 min     11  1 h 3 min ago █
 …
 taskflow · 5 agents ──────────────────────────────────────────   ···········⣿·⣿   1 h 5 min     24  2 h 51 min ago █
    7. docs                                                       ···········⣿·⣤       4 min      5  2 h 51 min ago █
 …
 global · 1 agent ─────────────────────────────────────────────   ·⣶··⣀⣶··⣤⣀·⣿·⣀       8 min     13  3 h 33 min ago █
   12. react-structure-checker                                    ·⣶··⣀⣶··⣤⣀·⣿·⣀       8 min     13  3 h 33 min ago █
 …
   17. ▸ _argibox · 4 agents unused ────────────────────────────────────────────────────────────────────────────────── │
   18. ▸ argibox · 3 agents unused ─────────────────────────────────────────────────────────────────────────────────── │
 ↑↓ move · enter details · tab mode · / filter · o sort · q quit                                          1-27/30  v0.1.0
```

(Capture at 120×30 with `-m 60`, nothing running: no row carries a mark and
neither the header nor the rules say `running`. The column alignment is
approximate.)

What is running shows only through its `✦` on the row: all has no separate
strip. The `N running` in the header counts those rows, the ones in the list
with `✦` (Claude Code agents and workflows have no row here and do not count;
they show in active), and it only appears if there is any. Whether it was used
within the window is told by the `last used` column; there is no other mark.

- **Group title**: only the project folder (the last path component, as in the
  ranking: `station`, `taskflow`), or `global`. If two groups in the list have
  the same folder, both are prefixed with the parent folder (`argi/station`,
  `Towerforge/station`); it only counts the groups the filter leaves. This is
  only what is painted: the full path is still the group's key, decides the
  order and is what `/` looks at (filtering by `argi` finds `station`).
- **Column header**, once, above the first group: `last 14 days`, `worked`,
  `runs` and `last used`, aligned with the rows.
- **Group rule**: ` folder · N agents ───`, or ` folder · N agents · K running
  ───` if any of its agents is running; the folder and the `K running` in
  `claude`, the rest in gray. The `───` rule ends two columns before the data
  block and, on the same line and in the same columns as the rows, go the
  group's figures: its 14-day bar (the sum of its rows day by day), the total
  worked in bold, the sum of runs and its last use (the most recent of its
  rows, empty if any is running). So the rule's figures fall under the headers
  just like those of the rows. The counts come from the rows that are visible:
  with a filter, only from those it leaves.
- **Row**: cursor and number, name and, only if running, `✦` in `claude`
  (alternating with `✧` every half second). The mark slot is always reserved,
  so the columns do not move when it appears. Then:
  - **last 14 days**: a one-character bar per local day, the oldest on the left
    and today on the right, in braille so it does not merge with the row below:
    gray `·` for days without work and, for the others, the quartile of that
    same row's maximum they fall in (each row has its own scale): `⣀` up to a
    quarter, `⣤`, `⣶` and `⣿` the last. An agent that was never used carries
    `··············`. See [How worked time is
    calculated](#how-worked-time-is-calculated).
  - **worked** and **runs**: the total of all its runs and how many there are.
    Empty if it has none.
  - **last used**: when it was last used in the **whole** history, not only in
    the window, or `never`. A running agent leaves it empty: its `✦` already
    says it.

  Like the ranking, «all» takes the full width: the bar and the data columns
  are flush to the right edge (before the scroll bar if it shows), in rows and
  group rules. Each header is right-aligned to its column, like its data, and
  each column is as wide as the widest of its header, its rows and its group
  rules. The bar is right-aligned, two spaces before `worked`, and ends with its
  header. The leftover space stays between the name and the bar. Below 100
  columns the bar is removed; below 80, `runs` too; in both cases what remains
  stays flush to the edge.
- **Unused groups**: projects in which no agent has any call or run go at the
  end, folded into a gray line `N. ▸ folder · N agents unused ───` (by name,
  `global` last), with the rule up to the right edge, where everything else
  ends, and nothing in the columns. The line carries a number and the cursor
  can be put on it; `Enter` unfolds it (`▾`) or folds it again. It is judged
  against all the group's agents, not only those the filter leaves.

Group order: on top those with any running agent; then those with any active
one, the one with the most recent activity first; then the rest by their last
call ever; at the end, the folded ones. On a tie, `global` goes last. Within
each group, `o` chooses between:

- **recent** (default): running ones first, then active ones by recency,
  inactive ones by last use and never-used ones by name.
- **worked**: by total time worked, from most to least. The second line says
  `sort: worked`.

If the list does not fit, a scroll bar shows on the right (`│` the track, `█`
what is visible), as in moon, in active too. On narrow screens the footer drops
`o`, `/` and `enter`, in that order, before the counter, and the version drops
before anything else.

### Ranking

```
  active   all   ranking                                          all time · 1 h 17 min in 3 runs · 10.6M tok

 By tokens · 2 agents with work ────────────────────────────────────────────────── tokens       worked   runs
 ❯  1. frontend    station  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━    6.2M        5 min      1
    2. docs        station  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━╸                   4.4M   1 h 12 min      2

 history kept: 1 run older than Claude's cleanup

 ↑↓ move · enter details · tab mode · p period · s by: time · tokens · runs · q quit                   v0.1.0
```

(Render with made-up data at 110 columns, sorted by tokens.)

The defined agents with work in the period: at least one run in it or some time
worked, so an agent whose runs add up to 0 min still shows. They go from most to
least by the active key (`s` cycles **time worked** → **tokens** → **runs**,
default time worked); on a tie, by the other two keys, then by name and
project. Each row: number, name, short project (the last path component, or
`global`), a bar proportional to the top value of the active key and three
columns: `worked`, `tokens` and `runs` in the period. The bar is drawn with `━`
per full cell and `╸` per half cell, so it does not fill the height of the cell
and the bars of consecutive rows do not touch; a nonzero value always shows at
least `╸`. The active key's column goes first, right after the bar it measures,
and the other two follow in their fixed order (sorted by tokens:
`tokens   worked   runs`); it goes in `ink`, the other two in gray. If the
agent has a running run, after the name it carries the same mark as in all:
`✦` in `claude`, alternating with `✧` every half second. Last use does not show
here: it is seen in all and in the details screen.

The ranking takes the full width. The rule reads ` By time worked · N agents
with work ───` (or `By tokens`, `By runs`) and carries the column headers
at its right end in that same order, aligned with the data (before the scroll
bar if it shows). The name is as wide as the longest one and always reserves two
columns for the mark; the project is as wide as the longest one up to 18
columns, and a longer one is cut with `…`. So project and bar start in the same
column on every row. The bar is elastic: it fills the gap between the
project and the columns, with two spaces on each side. On narrow screens the
columns that are not the active key go before the bar drops below 8 columns
(sorted by time, `runs` first and then `tokens`; the active key's column never
goes); then the bar goes away; if it still does not fit, the name is cut with
`…` (down to 6 columns), and then the project shrinks, until it disappears,
before any number is cut: the numbers always end at the right edge.

If some runs of the data come only from the [history cache](#kept-history), a
gray line below the rows says so: `history kept: N runs older than Claude's
cleanup`.

`p` cycles the period: `7 days` → `30 days` → `all time` (default, `all
time`); the footer shows `p period: 7 days · 30 days · all time` with the open
one highlighted if it fits, or just `p period`; likewise `s by: time · tokens ·
runs` or `s by`. `7 days` and `30 days` are local days counting today: the
period's work is the sum of the work of those days, and the period's runs are
those that started (local start day) within it. A run that crosses the period's
start contributes only its days inside and does not count as a run of the
period. Tokens are not split by day: the period's tokens are those of whole
runs that started in it. The filter also applies to the ranking and to its
total.

### Agent details

`Enter` on an agent opens its details, full screen:

```
 kiosk ──────────────────────────────────────────────────────────────────────────────── ~/Offing/argi/station
 ~/Offing/argi/station/.claude/agents/kiosk.md

 1 h 9 min worked   ·   10 runs   ·   6 min avg   ·   last used 2 h 18 min ago

 Station kiosk mode on the DGX: starts on its own at power-on, no desktop or login (systemd, cage, logind,
 … (4 lines)

 Work per day ──────────────────────────────────────────────────────────────────────────── last 14 days
 55 min ┤                                  ██
        │                                  ██
        │                                  ██
        │                                  ██ ▆▆ ▃▃
       0└───────────────────────────────────────────
          23 24 25 26 27 28 29 30 31  1  2  3  4  5   Oct → Nov

 Runs ──────────────────────────────────────────────────────────────────────────────── 10 · 7 most recent
   Nov 5 12:00   4 min   45 tools  GOAL (requested by Jordi): the Station screen can…  ./frontend/src
   Nov 4 09:31   6 min   65 tools  Make boot spinner solid
   Nov 4 08:58   3 min   40 tools  kiosk: boot spinner
 …
   … 3 more
 esc back · ↑↓ other agent · q quit                                                                    v0.1.0
```

- Rule with the name and the project, and the path of the `.md`.
- Figures: the time worked, the runs, the average per run and the last use; if
  running, in front `✦ running, N min`. If the line does not fit, the
  separators narrow. With no runs, `no runs recorded` replaces the first three.
- The full frontmatter description (`no description` if it has none).
- **Work per day** of the last 14 local days: a 4-row chart, with the maximum
  on the axis, a 2-character column per day and the days of the month below
  (`Oct → Nov` if it crosses a month). With no work in those days, `no work in
  the last 14 days`.
- **Runs**, from most recent to oldest: local date and time (`Nov 4 18:12`),
  duration (`running N min` in `claude` if it is), tools, the description it was
  launched with and the `cwd`, relative to the project root (`./frontend/src`)
  and only if it is not the root. It includes workflow runs launched with the
  agent's `agentType`. Those that do not fit, `… N more`. If the agent has old
  calls without a run of their own, at the end and if there is room, `+ N old
  calls with no recorded duration`. With no runs or calls, `none`.

Inside the details, `↑` `↓` move to the previous or next agent of the view it
was opened from (in the ranking, in ranking order; in active, through the
monitor rows that belong to a defined agent), skipping folded groups; `Esc` or
`Enter` go back.

### Run details

`Enter` on **any** row of active (running or finished, of any kind: defined
agent, Claude Code agent or workflow step) opens the details of the run that
**represents** the row (the most recent running one or, if there is none, the
most recent finished one), not those of the agent. Also on an `Agent` call with
no linked run, which opens a minimal screen: no task (`—`), no tools, no steps
(`—`) and with `ended … ago` in the header. The agent details open from here
with `a`, and only if the run has a defined agent. Example of a running one that
launched two other runs, at 100 columns:

```
 ✦ visual [own] via workflow · station                          running 4 min · 5 tools · 412k tok
   launched by  session 23dabecb · “paint the active screen with tokens”

 task ────────────────────────────────────────────────────────────────────────────────────────────
   paint the active screen
   with design B
 usage ───────────────────────────────────────────────────────────────────────────────── sonnet-5-5
   context       71%  ████████████████████████████▍░░░░░░░░░░░  142k of 200k
   input         18k
   output        24k
   cache read   351k
   cache write   19k   ≈ $0.39 at API prices
 tools ───────────────────────────────────────────────────────────────────────────────────────────
   Read 3 · Bash 1 · Edit 1
 launched · 2 ────────────────────────────────────────────────────────────────────────────────────
   1. ✦ Explore [claude]           2 min · 9 tools · 64k tok         Find where ctx is computed
   2.   general-purpose [claude]   4 min ago · 21 tools · 210k tok   Check ratatui gauge widths
 steps · 5 ───────────────────────────────────────────────────────────────────────────────────────
   5 s ago    Bash · git diff --stat
   1 min ago  Read · src/d.rs
   2 min ago  Edit · src/c.rs
   3 min ago  Read · src/b.rs
   4 min ago  Read · src/a.rs
 esc back · ↑↓ other agent · tab launched · pgup/pgdn steps · a agent · q quit               v0.1.0
```

(Render with made-up data, without the empty lines between the steps and the
footer. Below the tab header and the screen's second line, like the agent
details.)

- **Header**: the star, the name, the origin label (the same as in the list:
  `[own]`, `[own] via workflow`, `[workflow]` or `[claude]`) and the project in
  gray; on the right,
  `running N min · N tools · N tok`, preceded by `run i/n` in gray if the agent
  has more than one run (`1/n` is the most recent); `· N tok` is left out when
  the run has no tokens. In front of the name, the same star
  slot as the list: `✦` in `claude` alternating with `✧` every half second if
  running, blank if finished. The name follows the same state rule as the list:
  running, `claude-soft` in bold; finished, plain `ink`, even for a defined
  agent. The project is the last component of the `cwd`, without the list's
  disambiguation of twin folders. Below 70 columns `running N min ·` and `· N
  tok` are removed and so is `run i/n` (which also yields before the name and
  the project: it only shows if there is room); below 50, the labels too. If
  the run already ended, there is no `running` and the right side says `ended N
  min ago · N tok` in gray.
- **Launched by**: a gray line right under the header, indented 3 spaces,
  `launched by  …` with who launched the run (see [Who launched
  whom](#who-launched-whom)): `session 23dabecb · “first prompt”` (the first 8
  characters of the session id and its first prompt, in quotes, cut with `…`;
  without the prompt if there is none or no room); another run, as its name and
  label (`launched by  backend [own]`; `p` opens it); or `a run no longer on
  disk`. Without that datum, there is no line.
- **Temp folder notice**: only if the run was launched from Claude's temp folder
  (`Run.temp_cwd`, see [Temporary `cwd` from Claude](#temporary-cwd-from-claude)).
  A gray line under the previous one, indented 3 spaces: `launched from Claude's
  temp folder · project taken from the session` if the `cwd` was resolved to the
  session's, or `… · project unresolved` if not. If 8 or more columns are left
  for the path, ` · <temp path with ~>` follows, cut with `…`.
- A blank line separates those lines from the sections only if the body is 14
  lines or more.
- **task**: the run's whole first message, with its line breaks, wrapped to the
  width and with a maximum of 8 lines (the last ends in `…` if cut); `—` if it
  could not be extracted. It does not fall back to the `description`. Blank
  lines in the task are not painted.
- **usage**: the rule carries the model on its right, without the `claude-`
  prefix nor the date suffix (`sonnet-5-5`; `—` if unknown). Below, with
  13-column labels: `context`, the percentage, a bar of 8 to 40 columns (amber
  from 85%, like the figure) and `142k of 200k`; then `input`, `output`, `cache
  read` and `cache write`, in short form. After `cache write`, or on its own row
  if it does not fit, the cost estimate in gray: `≈ $0.39 at API prices` (`<
  $0.01` below a cent); with an unknown model there is no cost. See [Tokens,
  context and cost](#tokens-context-and-cost). With no tokens, context or model
  (a call without a run, or a run that has no reply yet) the section does not
  show.
- **tools**: uses per tool across the **whole** run, from most to least used
  (`Read 17 · Bash 12 · Edit 8`), spread over lines of the width, with a maximum
  of 3 (ends in ` …` if there are more). With no data, the section does not
  show.
- **launched · N**: the runs this one launched that are still in the data, in
  launch order: number, star if running, name and label, then `2 min · 9 tools
  · 64k tok` (running) or `4 min ago · 21 tools · 210k tok` (finished; without
  `· N tok` if 0) and the task. At most 6 show: the window follows the selected
  one, with `a-b/N` on the right of the rule. The selected one carries `❯`.
  Without children, the section does not show.
- **steps · N**: N is the total tools; if only the last 200 were kept, `steps ·
  last 200 of 345`. The most recent first (and only that one in `ink`, while
  looking from the top), with its `N s ago` and `Name · argument`. If there are
  more steps than lines, `a-b/N` shows to the right of the rule and they can be
  scrolled; with no steps, `—`. A run that only remains in the [history
  cache](#kept-history) has no steps: under the `steps` rule it says `from the
  history cache · steps are not kept`.
- **Footer**: ` esc back · ↑↓ other agent · ←→ other run · tab launched · enter
  open · pgup/pgdn steps · p parent · a agent · q quit`, each one only when it
  applies: `←→ other run` if the row has more than one run; `tab launched` if
  the run launched any; `enter open` with a child selected; `pgup/pgdn steps`
  unless it comes from the history cache; `p parent` if it was launched by a
  run still in the data; `a agent` if the run has a defined agent and its row
  is still in the list. If it does not fit, `←→`, `a`, `pgup/pgdn`, `↑↓`, `p`,
  `tab` and then `enter` are removed, in that order; `esc` and `q` are never
  removed.

With little height the steps rule and up to 3 steps (or the history cache note)
are reserved first, under the header; the rest goes in whole or not at all, in
this order: the launched-by line, the temp folder notice, the blank line, the
tools (rule and lines), the first 2 lines of the task, the launched runs, the
usage section and the rest of the task (up to 8 lines); whatever is left goes
to steps. More than the available height is never painted.

Keys inside the details:

- `Esc`: go back to the run it came from (see `Enter` and `p` below), as it was
  (scroll and selection); if there is none, go back to the list, with the
  cursor on the **row** that contains that run (whether or not it is the
  representative one) if it is still in the list.
- `Tab` / `Shift+Tab`: select the next or previous run in «launched» (from none,
  the first or the last); past the end, none is selected. The digits select
  that number directly (two in a row make a two-digit number).
- `Enter`: with a launched run selected, opens its details; without, like
  `Esc`.
- `p`: opens the details of the run that launched this one, only if it was
  launched by a run still in the data.
- `q` (and `Ctrl+C`) quits the program.
- `←` `→` (or `h` `l`): move to the previous (older) or next (more recent) run
  of the same agent. At the ends, or if the row has a single run, they do
  nothing. The list cursor does not move and the steps go back to the first
  line. Sibling runs are computed over the unfiltered groups: what is typed in
  `/` does not trim them.
- `↑` `↓` (or `k` `j`): move to the previous or next row of the list (another
  agent; running and finished), in grouped order, and open its representative
  run; at the ends it does not move.
- `PageUp`, `PageDown` or `space` (one page), `Home`/`g`, `End`/`G`: scroll the
  steps; `End` leaves the last page full.
- `a`: opens the agent details, only if the run has a defined agent and its row
  is still in the list; it also works from a run that is not the representative
  one.
- `/`, `o`, `r` and the mouse wheel are ignored.

Opening a launched run (`Enter`) or the parent (`p`) stacks the open one, so
`Esc` walks back through them. `←` `→` replace the open run and keep the stack;
`↑` `↓` empty it, and `a` closes the details and empties it too. If the selected
launched run disappears, the selection moves to the last one left.

The details refresh with the usual 2 s cycle and stay on the **same** run even
if, when the data changes, the row's representative becomes another. If the run
ends with the details open, they **do not close**: they keep the last snapshot
and `ended N min ago`; if it disappears from disk, they keep the last snapshot.
Even so, it becomes a finished run: on going back, the cursor follows it as long
as its last step falls in the window; if it leaves it, there is no row and it no
longer follows it.

### Keys and formats

| Key | Does |
|---|---|
| `↑` `↓` (or `k` `j`), `Home`/`End` (or `g`/`G`) | Moves the cursor |
| `PageDown` or `space`, `PageUp` | Pages down or up (view and cursor) |
| mouse wheel | Moves the view 3 lines; if the cursor ends up outside, it drags it |
| digits | Jumps to that numbered line (agents, folded groups and active rows); two in a row make a two-digit number. In a run's details, selects that launched run |
| `Enter` | In all and ranking, opens the agent details, or unfolds/folds an unused group. In active, on any row (running, finished or call without a run), opens the details of the run that represents it. In any details screen, goes back (in a run's, with the cursor on its row; with a launched run selected, it opens that one) |
| `Esc` | In any details screen, goes back (in a run's, first to the runs opened from it) |
| `/` | Filters by name or project (in active, also by task, over all rows) as you type; `Enter` fixes the filter, `Esc` removes it |
| `Tab` | Cycles active → all → ranking. In a run's details, selects the next launched run (`Shift+Tab`, the previous one) |
| `a` | Toggles active and all. In a run's details, opens the agent's (only if it has one) |
| `←` `→` (or `h` `l`) | Only in a run's details: the previous (older) or next (more recent) run of the same agent |
| `o` | In all, order within groups: recent or worked |
| `p` | In the ranking, period: 7 days → 30 days → all time. In a run's details, opens the run that launched it (if it is still in the data) |
| `s` | In the ranking, sort key: time worked → tokens → runs |
| `r` | Rereads the definitions now |
| `q` or `Ctrl+C` | Quits |

The footer only shows the main ones; `PageUp`/`PageDown`, the digits and `r`
work even if they do not show. Inside a run's details the keys are
[its own](#run-details).

Times: `20 s ago`, `3 min ago` and `2 h 30 min ago` below one day; `N d ago`
from 1 to 29 days; from 30 days on, the local date (`10/05/2026`, as
`MM/DD/YYYY`). Dates with time in the runs list: `Nov 4 18:12`. Worked
durations: `<1 min`, `N min` below one hour and `N h M min` from there on (`1 h
9 min`); truncated to the minute.

The screen captures the mouse so the wheel moves the list. To select text with
the terminal you have to hold `Shift` while dragging (in iTerm2, `Option`).

Colors: Claude Code's colors (`claude`, `ink`, `ok`…) in truecolor if the
terminal announces `COLORTERM=truecolor`, and their 256-color approximation if
not. It paints no background: it uses the terminal's.

## Output with `--plain`

It does not change with the screen views:

```
STATE     NAME                     LAST USED       WORKED  PATH
ACTIVE    backend                  6 min ago       35 min  ~/Offing/argi/station/.claude/agents/backend.md
ACTIVE    database                 20 min ago      12 min  ~/Offing/argi/station/.claude/agents/database.md
ACTIVE    docs                     38 min ago       4 min  ~/Offing/taskflow/.claude/agents/docs.md
ACTIVE    database                 38 min ago       6 min  ~/Offing/taskflow/.claude/agents/database.md
ACTIVE    backend                  39 min ago      19 min  ~/Offing/taskflow/.claude/agents/backend.md
inactive  ai-engineer              never                   ~/Offing/argi/_argibox/.claude/agents/ai-engineer.md

37 agents, 5 active (window: 60 min). Open Claude sessions: 4
```

`LAST USED` is the last time ever, with the same time format, or `never`: the
last call or the last step of a run, whichever is later (runs launched by a
workflow have no call). `WORKED` is the total of its runs, right-aligned, and is
empty if it has none; the width of the two columns comes from the data. `STATE`
is `RUNNING` if it has a running run, `ACTIVE` if its last use falls within the
window and `inactive` if not; the footer count includes the running ones. First
the running ones, then the active ones by recency, then the inactive ones by
their last time and at the end the never-used ones by name. Runs that only
remain in the [history cache](#kept-history) count in `WORKED` and `LAST USED`
too, and `--plain` also updates that cache. A running agent, in the same test:

```
RUNNING   demo    7 min ago           7 min  ~/.claude/agents/demo.md
```

## How it decides an agent is active

A subagent is not a process: a Claude Code session launches it with the `Agent`
tool. That is why «active» is a heuristic:

1. It reads all the transcripts (`~/.claude/projects/**/*.jsonl`). The first
   pass reads them whole; after that, the screen only reads what was appended to
   each file since the previous round (they are append-only: if one shrinks it
   is reread from the start). Before parsing the JSON, it discards the lines
   that do not contain `subagent_type"`.
2. From each line with an `Agent` `tool_use` it takes `subagent_type`, the
   session's `cwd` and the `timestamp`, and also the block's `id`, the
   `input.description` (`description` in `Invocation`) and the first clean line
   of the `input.prompt` (`prompt_line`; see [What is read from each
   run](#what-is-read-from-each-run)).
3. A **global** agent matches any session. A **project** agent only matches
   sessions whose `cwd` is inside that project: two projects with a `frontend`
   agent do not contaminate each other.
4. The last call that matches is the agent's «last time». If it falls within the
   window, the agent shows as `ACTIVE`.
5. If it has a **running** run (see below), it shows as `RUNNING` and counts as
   active even if it has no call in the window.

The name comes from the frontmatter's `name:` field; if missing, from the file
name.

«Open Claude sessions» counts the processes whose executable is named `claude`
(terminal, VS Code...), without the desktop app or its helpers.

## How worked time is calculated

Each subagent run has its own transcript and, next to it, a meta:

```
~/.claude/projects/<project>/<session>/subagents/**/agent-<id>.jsonl
~/.claude/projects/<project>/<session>/subagents/**/agent-<id>.meta.json
```

(`**` because the ones launched by a workflow hang from
`subagents/workflows/<run>/…`). From the meta it reads `agentType` (required:
without it the run does not show), `description`, `toolUseId` and `finished`;
from each line of the `.jsonl`, `timestamp`, `cwd`, `sessionId`, the
`tool_use` blocks of `message.content` and, on assistant lines, `message.id`,
`message.model` and `message.usage`.

1. **Time worked** of a run: the sum of the gaps between consecutive
   `timestamp`s, not counting those over **5 minutes**. This way a run resumed
   hours later does not count the wait.
2. **Tools**: the number of `tool_use` blocks in its transcript.
3. **Running**: the meta says `finished: false` and the `.jsonl` was modified
   less than **10 minutes** ago. What it shows is the time worked plus, if the
   gap from the last line to now is under 5 min, that gap. With `finished:
   false` but more than 10 min untouched, it is **interrupted**: it adds up the
   time worked and does not show as running. Without `finished` in the meta
   (the ones launched by a workflow), it is running as long as the `.jsonl` was
   written less than **2 minutes** ago.
4. **Which agent it goes to**: the same rule as calls, with `agentType` as the
   name: a global agent matches any `cwd`, a project one only runs whose `cwd`
   is inside the project. If the run's `cwd` is Claude's temp folder, the real
   project's is used (see [Temporary `cwd` from
   Claude](#temporary-cwd-from-claude)).
5. **Total** of an agent: the sum of all its runs (running ones, up to now),
   including those that only remain in the [history cache](#kept-history).
6. **Split by day** (for the 14-day bar, the details chart and the ranking
   periods): each gap counts whole in the **local** day where it starts, even if
   it ends past midnight. When reading, each gap is recorded in the 15-minute
   UTC slot where it starts, and the slots are grouped afterwards by local date;
   since all current time zones have an offset that is a multiple of 15 min, a
   slot never crosses a local midnight and the split is exact without rereading
   anything. The sum of the days is the total. What a running run adds since its
   last line goes to that line's local day.
7. **Link with its call**: the `id` of the `Agent` `tool_use` block in the
   parent transcript is the meta's `toolUseId`. The calls that do not link to
   any run are the ones in the details' `+ N old calls with no recorded
   duration`.

### What is read from each run

For the active monitor, the following is also taken from each run's transcript:

- **Task** (`task`): the first line with content of the first user message with
  text (if `content` is a list, the `text` blocks count; a `tool_result` is not
  a prompt). The leading `#`s and the boilerplate `ENCARGO (de …):` (or `TASK
  (from …):`) are removed and whitespace is collapsed: `# ENCARGO (de Jordi,
  05-11-2026): copia de seguridad de TODO` becomes `copia de seguridad de
  TODO`. The boilerplate is input data (the prompt's literal text): both
  spellings are recognized as they are.
- **Last tool** (`last_tool`): the last `tool_use` block in the transcript, as
  `Name · short argument`. `Bash`, its `command` on one line; `Read`, `Edit` and
  `Write`, their `file_path` relative to the `cwd` if it hangs from it and,
  otherwise, only the last component (`NotebookEdit`, the same with
  `notebook_path`); `Grep` and `Glob`, their `pattern`; the rest, only the name.

For the run details, also:

- **Full task** (`task_full`): the same first user message, but whole and with
  its line breaks, without the leading `ENCARGO (de …):` (or `TASK (from …):`)
  and with a maximum of 2000 characters (if exceeded, it ends in `…`).
- **Steps** (`steps`): the last 200 `tool_use` blocks, each with the time of its
  line and its `Name · argument`, with the argument as above and uncut (the view
  cuts it to the width).
- **Tools** (`tool_counts`): uses per tool across the whole run, not only the
  200 steps, from most to least used and, on a tie, by name. The sum is
  `tools`.

Steps and counts are stored shared (`Arc`), so refreshing the screen does not
copy them.

It is read like the history: the first pass whole and then only what was
appended to each `.jsonl`; the meta is reread if its date or size changes. The
two reads run in parallel.

From the `Agent` call in the parent transcript (`Invocation`), in addition to
`agent`, `cwd`, `at` and `id`, two fields are read that the monitor uses for
calls that do not link to any run:

- **`description`**: the call's `input.description`; discarded if empty.
- **`prompt_line`**: the first line with content of the `input.prompt`, cleaned
  like a run's task (without leading `#`s or `ENCARGO (de …):` / `TASK (from
  …):`, with whitespace collapsed) and uncut. The task of that monitor line is
  `prompt_line` or, if there is none, `description`.

### Tokens, context and cost

From the assistant lines of each run's transcript (`Run.usage`):

- **Tokens**: the four counters of `message.usage`, `input_tokens`,
  `output_tokens`, `cache_read_input_tokens` and `cache_creation_input_tokens`;
  their sum is the `tokens`/`tok` figure on screen. Claude Code writes one line
  per content block (`thinking`, `text`, `tool_use`…) and repeats the same
  `usage` on each one, so every message counts once, with the last `usage` seen
  for its `message.id` (the line's `uuid` if it has no id). `<synthetic>`
  messages (made up by Claude Code, with zero usage) are ignored. A fork starts
  with a copy of the parent's message that launched it, with the fork's own
  call (its `toolUseId`) inside: that line's usage is not the fork's and is
  skipped.
- **Model**: the latest `message.model` seen.
- **Context**: `input + cache_read + cache_creation` of the latest assistant
  message. The window is 1M if the model id contains `[1m]` or if the run ever
  went over 200k of context; otherwise 200k. The percentage is context over
  window, rounded.
- **Cost** (`status::pricing`): an estimate at the API list prices of a table
  of known models, with cache writes at the 5-minute rate. A model that is not
  in the table (or no model) has no cost: it is never guessed.

Token figures go in short form: `999`, `1.5k` (one decimal below 10k), `86k`,
`1M`, `1.1M`, `31.4M`, `142M`, without a trailing `.0`. The cost reads `≈
$0.62`, or `< $0.01` below a cent.

### Who launched whom

- **The session**: the run's `sessionId` (or, for reading, the folder its file
  hangs from: `<project>/<session>/subagents/…`), whose transcript is
  `<project>/<session>.jsonl`. From it the tool takes
  its first real prompt line (`Run.session_prompt`): it skips meta lines (the
  body of a skill), compact summaries and the blocks that start with `<`
  (`<system-reminder>`, `<command-name>`…). With no prompt, the first slash
  command (`/review 123`) is used. Like the temporary `cwd` below, only the
  first lines of the session are looked at.
- **Another run** (`Run.parent_run`, `Run.children`): a run whose meta
  `toolUseId` is the id of an `Agent` or `Task` `tool_use` block inside another
  run's transcript was launched by that run. This nested case has only been
  seen in test fixtures, not yet in real transcripts.

The details say `launched by` the parent run if it is still in the data (on
disk or in the history cache), `a run no longer on disk` if not, or the session
otherwise.

### Temporary `cwd` from Claude

Some runs and calls carry a Claude temp folder as `cwd`
(`/tmp/claude-<uid>/…` or `/private/tmp/claude-<uid>/…`, with a numeric
`<uid>`). That folder is not a project, so it is replaced by the real
directory:

- **Run** (`Run.cwd`): the first non-temporary `cwd` in the parent session's
  transcript (`~/.claude/projects/<project>/<session>.jsonl`). Only its first
  lines are looked at (200 at most or 4 MB) and only those containing `"cwd"`
  (or `"user"`, for the [session's first prompt](#who-launched-whom)). The
  result is stored per session. If the parent cannot be read, the temporary
  one is kept and retried on the next refresh. In addition, `Run.temp_cwd` keeps
  the temporary `cwd` the run was launched with, whether or not it was resolved
  (`None` if it was never temporary); `RunDetail.temp_cwd` carries it to the
  details, along with `cwd_resolved` (`true` if `cwd` is already another: the
  session's was taken). With that the details paint the [temp folder
  notice](#run-details). A call without a run does not have it.
- **Call** (`Invocation.cwd`): the first non-temporary `cwd` in the very file
  where the call is.

If there is no real `cwd`, the temporary one is kept: none is made up. With the
`cwd` resolved, the run is grouped under its project and matches its project
agents like any other.

## Kept history

Claude Code deletes old transcripts (`cleanupPeriodDays`, 30 days by default),
and with them the runs would vanish from the totals. So the tool keeps finished
runs in its own cache:

```
$XDG_CACHE_HOME/claude-agents/runs.jsonl    # if XDG_CACHE_HOME is an absolute path
~/.cache/claude-agents/runs.jsonl           # otherwise
```

It is the only file the tool writes; the screen and `--plain` both update it.

- One JSON object per line and per run, with `"v":1` (a line of another
  version, or a broken one, is skipped). Only runs that are not in progress go
  in; a running one goes in once it ends. It stores what the views need: agent,
  description, session and its first prompt, links, `cwd`, times, worked time
  and its 15-minute slots, tool counts, task, tokens, model and context. Not the
  steps, the last tool or the temporary `cwd`. Around 1 MB for some 400 runs.
- It is rewritten whole and atomically (a temporary file next to it, then
  `rename`), only when some run is new or changed, and merged first with what
  is on disk, so another instance's runs are not lost.
- Read and write errors are ignored: the tool goes on without it. If the file
  exists but cannot be read, it is never overwritten.
- When reading, the transcript wins: the cache only adds the runs whose
  transcript is gone. Those count wherever the history counts (worked time,
  runs, the per-day chart, last use and the ranking), but they have no steps:
  their details say `from the history cache · steps are not kept`. The ranking
  says how many there are (`history kept: N runs older than Claude's cleanup`).
- Tests never write there: they use their own temporary folders, or no cache.

To start over, delete the file.

## Packaging and releases

Each archive holds just the `claude-agents` binary, and its name carries no
version, so `releases/latest/download/<archive>` always points to the latest
one. Locally:

```
make package                          # host: dist/claude-agents-<platform>-<arch>.tar.gz
make package CARGO_TARGET=aarch64-unknown-linux-musl CARGO_CMD=cross
make package-all-linux                # the 4 Linux targets, gnu and musl (cross + Docker)
make package-all-macos                # both macOS architectures (needs a Mac)
make package-all                      # all of the above
make checksums                        # dist/checksums.txt (SHA-256 of every dist/*.tar.gz)
make rust-targets                     # lists the triples package-all builds
```

Linux targets build with [`cross`](https://github.com/cross-rs/cross), which
needs Docker running. Install it once with
`cargo install cross --git https://github.com/cross-rs/cross`. The variables
`CARGO_TARGET`, `CARGO_CMD` (`cargo` or `cross`), `DIST_DIR` (default `dist`),
`LINUX_TARGETS` and `MACOS_TARGETS` change what gets built and where.

Releases follow `beta → main`. `make release` from `beta` asks for the new
version (patch, minor, major or typed), writes it to `VERSION`, `Cargo.toml`
and `Cargo.lock`, commits it on `beta`, merges `beta` into `main`, tags
`vX.Y.Z` and pushes; from `main` it tags the current version and pushes; from
any other branch it refuses. Nothing is built or attached by
hand: the pushed tag triggers
[`.github/workflows/release.yml`](.github/workflows/release.yml), which builds
the six targets (Linux with `cross`, macOS on a macOS runner), checks that the
tag is `v$(cat VERSION)`, runs `make checksums` and publishes the GitHub
release with the archives and `checksums.txt`.
[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs `cargo fmt
--check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` on
Ubuntu and macOS for every push to `main`, every `v*` tag and every pull
request.

## Limitations

- «Running» relies on the meta's `finished` and the `.jsonl`'s date: a run that
  waits more than 10 min without writing anything (an unanswered permission, a
  long tool) shows as interrupted until it writes again. A workflow run, whose
  meta carries no `finished`, stops showing as running after 2 min without
  writing, and keeps showing for up to 2 min after finishing.
- Active merges the runs of the same agent in a project into a single row: they
  can only be seen one by one from the details, with `←` `→`, and the counters
  in the header and in the rules count rows, not runs. An own agent and a
  `[claude]` one with the same name, or a `visual [workflow]`, are different
  rows. The window (`-m`) decides which runs get in: one
  older than it does not count in the `×N`.
- Old calls, from before Claude Code stored one transcript per run, have no run
  of their own: they add no time or days, and in the details they only count in
  `+ N old calls with no recorded duration`. If they fall within the window, the
  active monitor does show them, as a finished run without tools (merged with
  those of its agent).
- Claude Code agents (`general-purpose`, `Explore`, `fork`…) have no definition
  on disk and do not show in all or in the ranking, even if they have runs. They
  do show in active, running and as finished within the window, but they add no
  time or runs to any agent; there `Enter` opens the run details, but there are
  no agent details (`a` does nothing). The same goes for a project agent
  launched from outside its project, unless the `cwd` was Claude's temp folder:
  that one is resolved to the session's project (see [Temporary `cwd` from
  Claude](#temporary-cwd-from-claude)).
- If the parent session's transcript cannot be read (or has no real `cwd` in its
  first lines), a run with a temporary `cwd` keeps the temporary one: it matches
  no project agent and shows as `[claude]` under a group named after that
  temporary folder.
- Workflow runs with `agentType` `workflow-subagent` are not assigned to any
  agent for time, runs or ranking. They show in active, running and as finished
  within the window, and there the agent is resolved by the `description` (and
  `a` in the run details opens that agent's details), but that run does not add
  to the agent's details or mark it with `✦` in all.
- A workflow run launched with an explicit `agentType` adds time to that agent
  and shows in the «Runs» of its details, but it has no `Agent` call in the
  parent transcript: its last use comes from the run's last step.
- The run details keep only the last 200 steps (the per-tool count does count
  all) and the full task is cut at 2000 characters; in the details, the task
  shows 8 lines at most and the tools 3. Its header uses the project's short
  name, without the list's disambiguation of twin folders.
- Time worked is an approximation: it counts the time between transcript lines,
  not CPU or model time, and discards any pause over 5 min even if it was work
  (a long tool).
- In the split by day, a gap of up to 5 min that crosses midnight counts whole
  in the day it starts. If some time zone had an offset that is not a multiple
  of 15 min (none today), a gap next to midnight could fall in the neighboring
  day.
- Days, ranking periods and the details' times use the machine's local time
  zone.
- Run transcripts that are symbolic links are skipped (they point to the same run
  from another session).
- It depends on the current transcript format (`message.content[].name ==
  "Agent"` and `input.subagent_type`). If a future version changes it,
  everything will show as inactive.
- When walking the home, hidden directories (except `.claude`), `node_modules`,
  `target`, `.venv`, `venv`, `vendor` and `Library` are skipped. Inside a
  `.claude` only `agents` is looked at, so agents copied into `worktrees` are
  not duplicated.
- macOS/Linux only: sessions are counted with `ps`. There is no Windows build;
  use WSL. Prebuilt binaries cover x86_64 and aarch64 only.
- It reads the whole history of transcripts and runs. With ~1 GB in ~480 files
  (about 290 of them subagent runs), `--plain` takes 1.3–1.8 s in both release
  and with the debug binary (which uses `opt-level = 1` for this), and pays it
  on every execution. On the screen, later rounds only read what was appended
  (a few ms).
- The «last time» only goes as far as the transcripts and the [history
  cache](#kept-history) do: runs seen once are kept, but `Agent` calls are not,
  so an agent whose only uses were old calls without a run of their own may
  show as `never` once those transcripts are deleted. Runs whose transcripts
  were deleted before the tool ever ran are not in the cache either.
- Runs from the history cache have no steps or last tool, and nothing in them
  changes any more.
- The cost is an estimate at API list prices, from a fixed table in the code:
  on a subscription you do not pay per token, cache writes are counted at the
  5-minute rate (Claude Code's 1-hour cache writes cost twice the input, so it
  can fall short), and a model missing from the table has no cost at all.
- The context window is not written anywhere: it is 1M if the model id carries
  `[1m]` or the run went over 200k, and 200k otherwise. A 1M run whose model id
  lacks `[1m]` and that never went over 200k shows its percentage over 200k.
- A run launched by another run is only detected through the parent's `Agent`
  or `Task` call in its transcript; that case has only been tested with
  fixtures, never seen in real transcripts.
- The ranking's tokens are not split by day: a run that crosses the start of
  the period counts its tokens whole if it started inside, and none otherwise.
- The screen captures the mouse: without `Shift` (or `Option` in iTerm2) text
  cannot be selected by dragging.
- `-v` only has an effect with `--plain`; on the screen the sessions show as a
  counter in the header.
