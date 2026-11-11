# The interface

The interactive screen, view by view, the `--plain` table and the panel of
`claude-agents update`. How the figures on it are computed, and how the update
works, is in [how-it-works.md](how-it-works.md).

## The screen

Three views, shown as tabs in the header (the open one painted over `claude`),
each one for a different purpose:

- **active** (default, the one that opens at startup): a live monitor of what is
  happening and what has happened in the window (`-m`). Everything is grouped by
  project (the folder where the Claude Code session was launched): in each
  group, one block per agent, with one row per running run and, below them,
  one row for everything it finished within the window.
- **all**: the catalog, every agent on disk grouped by project, with its
  history.
- **ranking**: the agents with work in the period, from the one that worked the
  most to the one that worked the least, without groups.

`Tab` cycles active → all → ranking; `a` toggles between active and all (from
the ranking, it goes to active).

On the right of the header: in active, `window N min · N running · N in window ·
N sessions · N tok` (`N running` are the runs in progress, one row each, and
`N in window` the finished rows, one per agent, both without filter; `N tok`
are the tokens of the rows that show, with the filter applied, and is left out
when 0); in all,
`window N min · N agents · N running · N sessions` (without `· N running` if
nothing is running); in the ranking, `<period> · <total> in N runs · N tok`
(the tokens of the period, also left out when 0). On narrow screens it drops
the sessions first (in active the tokens stay after them), then the tokens and
then the counts, never the tabs. The second line carries the
filter (or the read error) on the left and, on the right, `sort: worked` only
in «all» and when it is not the default order.

The version (`v0.1.0`, without the name) goes in gray on the last line, flush
right with one free column of margin; with the scroll counter it reads
`1-5/11  v0.1.0`. It shows in the three views, in the agent details and in the
[agent screen](#the-agent-screen) and the [run screen](#the-run-screen) (the
agent details and the run screen, without the counter). The keys take priority: the version is omitted if it does
not fit next to them (keys width + right-hand width + 2 <= screen width).

When the [daily check](how-it-works.md#the-daily-check) finds a newer release,
the footer announces it left of the version, on every screen that carries it:
`1-5/11  ↑ v1.2.0 available · claude-agents update  v1.1.0`, with `↑ v1.2.0` in
`claude` and the rest in gray. It arrives whenever the background check
answers, without blocking anything. When it does not fit, it shortens to
`↑ v1.2.0  v1.1.0`, then goes away, and only then does the version; the keys
are still cut first.

The history is refreshed every 2 s and the definitions every 30 s or on pressing
`r`. At startup, until the definitions and the first reading of the
transcripts arrive, the body shows, centered both ways, `✦ loading agents`:
the star, in `claude`, alternates between `✦` and `✧` every half second, and
the text goes in `ink`. Below it, in gray, come the transcripts folder, with
`~` for the home, and how long it has been waiting (`~/.claude/projects · 1.2 s`): tenths of a second under a minute and, from
then on, like the time worked (`2 min`). The footer only offers `q quit`, with
the version on the right. On a narrow screen the path is cut with `…` and the
time stays whole; with fewer than 4 columns for the path, only the time is
left. The text is cut with `…` too and, if not even one letter fits, only the
star is left. With a one-row body, only the first line shows.

## Active: the monitor

(Excerpt of a render with made-up data, at 110 columns: two of the groups.
`docs` has three runs in progress and one finished within the window, so its
block hangs four rows from its name, carries a single number and its `×3`
counts only the three in progress; the `frontend` one in progress in `web` is
alone and hangs from its name the same way; `frontend` in `station` only
finished, with no task or tokens recorded; `Explore` finished 4 min ago.)

```
  active   all   ranking                       window 5 min · 4 running · 5 in window · 4 sessions · 887k tok

         agent                                                                       ctx        time   tokens
 …
 station · 3 running · 2 recent ─────────────────────────────────────────────────────────────────────────────
    2. ✦ docs [own] ×3
         ├ Review the docs/README conventions                               ▰▰▰▰▱▱▱▱ 41%       1 min     140k
         │ skill  superpowers:brainstorming
         │ now    Read · docs/interface.md
         ├ Check the Makefile targets                                       ▰▰▱▱▱▱▱▱ 22%       1 min     141k
         │ now    Bash · grep -n release Makefile
         ├ Read contact-form.astro and cookie-table                         ▰▰▰▰▱▱▱▱ 48%       2 min     142k
         │ skill  frontend-design:frontend-design
         │ now    Read · src/components/contact-form.astro
         └ Update the install section                                                  —   3 min ago      96k

    3.   frontend [own]
         └ —                                                                           —   2 min ago        —

 web · 1 running · 1 recent ─────────────────────────────────────────────────────────────────────────────────
    4. ✦ frontend [claude]
         └ Add a LanguageSwitcher to header.tsx                             ▰▰▰▰▰▰▱▱ 71%       9 min     412k
           now    Edit · src/components/header.tsx

    5.   Explore [claude]
         └ Find where the locale is read                                               —   4 min ago      51k
 ↑↓ move · enter details · tab mode · / filter · q quit                                                v0.1.0
```

A single list, grouped by project, with the agent blocks numbered continuously
across all groups: one number per block, not per run. In each group, **one block per agent**: the agent's name on a line of its own
and, hanging from it, **one row per running run** and, last, **one row for
what it finished** within the window. A running row takes two lines, three if
its run has loaded a skill (the row, `skill` and `now`); a finished row is a
single line. The agent line carries the cursor and the block's number, and the
whole block (that line and all its rows, with their `skill` and `now`) is what
is selected. There are no blank
lines between the rows of a block: one blank goes between the blocks of a group
and another before each group rule except the first, and neither is part of
any block. The cursor and the scroll treat a block as a unit and keep it whole
on screen; one taller than the view shows its beginning. Above them, once and
in gray, the column header (`agent`, `ctx`, `time`, `tokens`), with the first
group rule right under it. There are no sections or a separate timeline.

- **What shows.** All **running** runs, with no cap (the list scrolls), and all
  **finished** runs within the window. A finished run is one that is not
  running (an interrupted one too) and whose last step falls in the window, of
  any kind: defined agents, Claude Code agents (`general-purpose`, `Explore`…)
  and workflow steps. `Agent` calls in the window that do not link to any run
  also show: they appear as a finished run with `—` in `ctx` and `tokens`, and
  their task is
  the first clean line of the call's prompt or, if there is none, its
  `description`. A call and its run are one and the same.
- **Who is the same agent.** The key is (project, agent). If the run resolves
  to a defined agent, the agent is that one: a direct launch and a workflow step
  with the same agent count as the same (the `via workflow` label does not
  count). With no defined agent, the key is (kind, name): a `visual [claude]`
  and a `visual [workflow]` are different agents, just like an own agent and a
  homonymous `[claude]` one.
- **One block per agent.** Everything an agent has in the group goes together,
  once: the agent's name on a line of its own (`docs [own] ×3`, with ` ×N`
  counting only the block's running runs, and only if there is more than one;
  the finished row and the runs it merges do not count), and below it its
  rows, each one hanging from it with `├` (`└` the last one) followed by its task, so all the tasks of the block start
  in the same column. An agent with a single row has its name line too, without
  ` ×N` (`frontend [claude]`), and the row below it with `└`.
- **Running: one row per run.** Running runs are never merged: each one is a
  row of its block.
- **Finished: one row per agent.** All the finished runs (and calls without a
  run) of the same agent in the group merge into one row, the last of its
  block, represented by the most recent one: its task and its `N min ago` come
  from it. An agent with only finished runs is a block of that one row, with
  its name line above it.
- **Order.** Within a group, first the blocks with something running, the one
  whose newest run started latest on top, and inside each block the run that
  started latest on top, with the finished row last; then the blocks with only
  a finished row, the one that ended latest on top. Groups are
  ordered by their most recent activity: the start of their newest running run
  or the end of their newest finished one; on a tie, by folder.
- **Group folder**: the folder where the Claude Code session was launched (its
  first real `cwd`, see [Launch folder](how-it-works.md#launch-folder)), not the
  subagent's `cwd`, which is wherever the main agent was when it launched it and
  follows every `cd`. So all the subagents of a session fall in the same group,
  even if they work in a subfolder or outside the project. If the launch folder
  is not known, the run's `cwd` is used.
- **Group rule**: ` folder · N running · N recent ───`, each part only if
  greater than 0. They count the rows that follow it: `running` the running
  runs, `recent` the finished rows (one per agent). The folder goes in `claude-soft` and the
  rest in gray; if two groups have the same folder they are prefixed with the
  parent folder, as in «all»; `global` if there is no folder at all.
- **The agent line**: cursor `❯` and the block's number (the number in gray,
  in `claude-soft` under the cursor, as in all and the ranking); then, in the
  star slot, `✦` in `claude` (alternating
  with `✧` every half second) if the block has something running, blank if it
  only has a finished row; then, in the name's column, the name and a gray
  (`ink-muted`) label saying where the agent comes from (`[own]` for one of the user's own
  agents; `[own] via workflow` for a workflow step whose `description` is one
  of them: `docs [own] via workflow`; `[workflow]` for a workflow step that is
  not one of the user's agents; `[claude]` for a Claude Code one) and, if the
  block has more than one run in progress, ` ×N` in `subtle` at the end of the
  label (`docs [own] ×5`, `docs [own] via workflow ×3`): N is the block's
  running runs only. With a single one in progress, or only a finished row, it
  does not show. The finished row never carries a count. No figures.
- **The row**: no cursor and no number: blank where the agent line has them
  and its star; then, in the name's column, `├ ` or `└ ` in `subtle` and the task,
  or `—` if there is none. The row never repeats the name: it is on the agent line. On the right,
  flush to the edge, `ctx`, `time` and, last, `tokens`:
  - `ctx` is how full the run's context is, only while it runs: an 8-step gauge
    where each started eighth fills a step (`▰` in `claude`, `▱` in `subtle`),
    and the percentage in gray (`▰▰▰▰▱▱▱▱ 41%`); gauge and percentage turn
    amber from 85%. `—` if the row is finished or there is no data yet.
  - `time` is `N min` (what the run has worked) for running rows and `N min
    ago` (since the last step) for finished ones.
  - `tokens` is the run's tokens, in short form; on a finished row, the sum of
    **all** the runs merged into it; `—` if 0 (see [Tokens and
    context](how-it-works.md#tokens-and-context)).

  The star slot is always reserved, on the agent line and on every row, so
  the name and the tree stay in the same column, and it is never removed for
  narrowness.
- **`skill`**, right under a running row and only if its run has invoked a
  skill: the last one it loaded (`superpowers:brainstorming`), which stays until
  it loads another (see [What is read from each
  run](how-it-works.md#what-is-read-from-each-run)).
- **`now`**, under every running row: the run's last tool (`Bash · git diff
  --stat`, `Edit · README.md`, `Skill · superpowers:brainstorming`), or `—` if
  there is none yet.

Finished rows carry neither line, and the list never shows the subagent's
`cwd`: it is in the [run screen](#the-run-screen), and the `/` filter still
searches it.

Each value goes on a single line, cut with `…` if it does not fit.

Colors. The name on the agent line depends only on the state of its block,
whatever the kind: with something running, `claude-soft` in bold (a `[claude]`
one too); with only a finished row, gray (`ink-muted`). A running row has its
task in `ink`, a finished one in gray. What is running is told apart by the
blinking star on its agent line, by the task color and by `N min` versus `N
min ago`. The figures (`time`, `tokens`, the `ctx` percentage) go in gray on
every row; a `—` in `ctx` or `tokens` goes in `subtle`. The lines under a row
start in the name's column, which comes right after the star slot (9 columns
with fewer than 100 blocks in the list; it grows with the digits of the number),
as does the `agent` column header: first a 2-column guide, `│ ` in `subtle` while the block goes on below
(under `├`, also when what follows is the block's finished row), two spaces
under the block's last row; then the label (`skill`, `now`) in `subtle`,
7 columns wide, and the value in gray.

Who each run is:

- a **defined agent** launched directly: its name, with `[own]`;
- a **workflow step** (`agentType` `workflow-subagent`): if its `description`
  is the name of a defined agent, that agent (label `[own] via workflow`);
  otherwise it is not one of the user's agents: the `description` as the name
  (or `workflow` if it has none), with the `[workflow]` label. In a workflow the
  `description` is the name, not the task;
- any other kind (`general-purpose`, `Explore`…, or a project agent from a
  session launched outside its project): its `agentType`, with the `[claude]`
  label.

Whether a project agent is `[own]` is decided with the launch folder too,
because Claude Code loads `.claude/agents/` from there: a project agent launched
after a `cd /tmp` is still `[own]`.

The task is the first content line of the run's first message (see [What is
read from each run](how-it-works.md#what-is-read-from-each-run)); if it could not be
extracted, it falls back to the meta's `description`, except in a workflow,
where it stays `—`. For a call without a run, the call's own (above).

Rows narrow down: the task is cut first, with `…`; below 70 columns `ctx` and
`tokens` and their headers are removed together and only `time` stays, at the
right edge; below 50, the labels too (with their ` ×N`). If the whole label
does not fit it is lost in parts, in this priority order: name > origin
(`[own]`, `[workflow]`, `[claude]`) > ` ×N` > `via workflow` (`via workflow`
drops first, then ` ×N`, then the origin and, if that does not fit either,
nothing). The name, on the agent line, which has no figures, is only cut with
`…` if it does not fit the width.

If there is nothing running or in the window, the monitor says so and points to
the last use of any defined agent:

```
 nothing in the last N min
 last use: <agent> from <short project> X ago · tab to see all
```

(or `no use recorded · tab to see all`). If it is the filter that leaves it
empty, it says `adjust the filter or press tab`.

`Enter` on a block opens the [agent screen](#the-agent-screen) of that agent:
all its runs in the window, one per row; `Enter` there on a run opens the [run
screen](#the-run-screen). Both take the whole screen, without the tab row or
the second line, and it works on a block of any kind (defined agent, Claude
Code agent or workflow step). The footer paints `enter details` dimmed only if
the cursor is not on a block. The digits,
`↑` `↓` and `Home`/`End` select blocks, in the grouped order (continuous across
groups); a row cannot be selected on its own. A click on any line of a block
(the agent line, a row, `skill` or `now`) selects that block. A blank line does
nothing on a click.
`/` filters by name, group folder, `cwd` or task over all rows (running and
finished); a group left without rows does not show, its rule counters are
recomputed and so are the blocks, over the rows left (their ` ×N` counts only
the running ones among those; a row left alone keeps its name line, with `└`). `o` does nothing here: the order is the one above.

## All: the catalog

```
  active   all   ranking                                                          window 60 min · 37 agents · 4 sessions

                                                               last 14 days    worked   tokens        last used █
 api · 6 agents ─────────────────────────────────────────────   ───────────━╸━  6 h 18 min     214M      5 min ago █
 ❯  1. docs                                                     ───────────━╸━      40 min    38.2M      5 min ago █
    2. reviewer                                                 ───────────━╸━  1 h 45 min    61.7M     19 min ago █
    3. visual                                                   ───────────━╸━  1 h 17 min    44.9M  1 h 3 min ago █
 …
 web · 5 agents ─────────────────────────────────────────────   ───────────━─━   1 h 5 min    58.3M  2 h 51 min ago █
    7. docs                                                     ───────────━─╸       4 min     2.1M  2 h 51 min ago █
 …
 global · 1 agent ───────────────────────────────────────────   ─━──╸━──╸╸─━─╸       8 min     9.6M  3 h 33 min ago █
   12. lint-checker                                             ─━──╸━──╸╸─━─╸       8 min     9.6M  3 h 33 min ago █
 …
   17. ▸ legacy · 4 agents unused ──────────────────────────────────────────────────────────────────────────────────── │
   18. ▸ sandbox · 3 agents unused ─────────────────────────────────────────────────────────────────────────────────── │
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
  ranking: `api`, `web`), or `global`. If two groups in the list have
  the same folder, both are prefixed with the parent folder (`shop/api`,
  `blog/api`); it only counts the groups the filter leaves. This is
  only what is painted: the full path is still the group's key, decides the
  order and is what `/` looks at (filtering by `shop` finds `api`).
- **Column header**, once, above the first group: `last 14 days`, `worked`,
  `tokens` and `last used`, aligned with the rows.
- **Group rule**: ` folder · N agents ───`, or ` folder · N agents · K running
  ───` if any of its agents is running; the folder and the `K running` in
  `claude`, the rest in gray. The `───` rule ends two columns before the data
  block and, on the same line and in the same columns as the rows, go the
  group's figures: its 14-day bar (the sum of its rows day by day), the total
  worked in `ink` bold (the only figure of that column in `ink`), the sum of
  its rows' tokens and its last use (the most recent of its
  rows, empty if any is running). So the rule's figures fall under the headers
  just like those of the rows. The counts come from the rows that are visible:
  with a filter, only from those it leaves.
- **Row**: cursor and number, name and, only if running, `✦` in `claude`
  (alternating with `✧` every half second). The mark slot is always reserved,
  so the columns do not move when it appears. Then:
  - **last 14 days**: a one-character bar per local day, the oldest on the left
    and today on the right, drawn with lines that do not fill the height of
    the cell so it does not merge with the row below: a gray `─` for days without work and, for the others,
    measured against that same row's maximum (each row has its own scale), `╸`
    up to half of it and `━` above half, both in `claude-soft`. A day with any
    work never shows as `─`. An agent that was never used carries
    `──────────────`. See [How worked time is
    calculated](how-it-works.md#how-worked-time-is-calculated).
  - **worked**: the total time of all its runs, in gray without bold, also on
    the row under the cursor: `ink` is kept for the project total on the rule.
    Empty if it has no runs.
  - **tokens**: the tokens of all its runs, ever (running ones, what they
    carry so far), in short form (`45.9M`, `412k`) and in gray; `—` in `subtle`
    if it has runs but no tokens counted; empty if it has no runs. See [Tokens
    and context](how-it-works.md#tokens-and-context). The number of runs is
    not a column here: it is in the [agent details](#agent-details) and
    the ranking.
  - **last used**: when it was last used in the **whole** history, not only in
    the window, or `never`. A running agent leaves it empty: its `✦` already
    says it.

  Like the ranking, «all» takes the full width: the bar and the data columns
  are flush to the right edge (before the scroll bar if it shows), in rows and
  group rules. Each header is right-aligned to its column, like its data, and
  each column is as wide as the widest of its header, its rows and its group
  rules. The bar is right-aligned, two spaces before `worked`, and ends with its
  header. The leftover space stays between the name and the bar. Below 100
  columns the bar is removed; below 80, `tokens` too; in both cases what remains
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

## Ranking

```
  active   all   ranking                                          all time · 1 h 17 min in 3 runs · 10.6M tok

 By tokens · 2 agents with work ────────────────────────────────────────────────── tokens       worked   runs
 ❯  1. frontend    api      ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰    6.2M        5 min      1
    2. docs        api      ▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▰▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱    4.4M   1 h 12 min      2

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
columns: `worked`, `tokens` and `runs` in the period. The bar is the same meter
as the `ctx` column in active: one `▰` in `claude` per step of its share of the
top value, rounded up, then `▱` in `subtle` up to the bar's width. Neither fills
the height of the cell, so the bars of consecutive rows do not touch, and a
nonzero value always shows at least one `▰`. The active key's column goes first, right after the bar it measures,
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

If some runs of the data come only from the [history cache](how-it-works.md#kept-history), a
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

## Agent details

`Enter` on an agent, in all and in the ranking, opens its details, below the
tab row; from active they open with `a` in the [agent
screen](#the-agent-screen), only for a defined agent:

```
 reviewer ─────────────────────────────────────────────────────────────────────────────────── ~/code/shop/api
 ~/code/shop/api/.claude/agents/reviewer.md

 1 h 9 min worked   ·   10 runs   ·   6 min avg   ·   last used 2 h 18 min ago

 Reviews every change before it is committed: reads the diff, runs the tests and flags anything that
 … (4 lines)

 Work per day ──────────────────────────────────────────────────────────────────────────── last 14 days
 55 min ┤                                  ██
        │                                  ██
        │                                  ██
        │                                  ██ ▆▆ ▃▃
       0└───────────────────────────────────────────
          23 24 25 26 27 28 29 30 31  1  2  3  4  5   Oct → Nov

 Runs ──────────────────────────────────────────────────────────────────────────────── 10 · 7 most recent
   Nov 5 12:00   4 min   45 tools  Review the cursor pagination change in the orders end…  ./src/routes
   Nov 4 09:31   6 min   65 tools  Check the migration for the new orders index
   Nov 4 08:58   3 min   40 tools  Review the retry logic in the payment client
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
  launched with and the `cwd`, relative to the project root (`./src/routes`;
  with `~` if it is outside) and only if it is not the root. It includes
  workflow runs launched with the agent's `agentType`. Those that do not fit,
  `… N more`. If the agent has old calls without a run of their own, at the end
  and if there is room, `+ N old calls with no recorded duration`. With no runs or calls, `none`.

Inside the details, `↑` `↓` move to the previous or next agent of the view it
was opened from (in the ranking, in ranking order; in active, through the
monitor blocks that belong to a defined agent), skipping folded groups; `Esc` or
`Enter` go back: to the list or, if the details were opened from the agent
screen, to that screen. Any other key there (`↑` `↓`, `Tab`…) leaves the agent
screen behind, and `Esc` then goes back to the list. A click only responds on
the tabs: it closes the details (and the agent screen under them) and switches
to that view.

## The agent screen

`Enter` on **any** block of active (with runs in progress or only finished, of
any kind: defined agent, Claude Code agent or workflow step) opens the agent
screen: every run of that agent in the group, one per row. It takes the whole
screen, without the tab row or the second line. Example at 100 columns:

```
 ✦ visual [own] · 4 runs                                      2 working now · 2 finished · 4.6M tok
   defined in   ~/code/shop/api/.claude/agents/visual.md
   running in   ~/code/shop/api

 working now · 2 ──────────────────────────────────────────────────────── ctx         time   tokens
 ❯ 1. ✦ Paint the active screen with design B                    ▰▰▰▰▰▰▱▱ 71%        4 min     412k
        now     Bash · git diff --stat
        skills  frontend-design:frontend-design

   2. ✦ Check ratatui gauge widths                               ▰▰▱▱▱▱▱▱ 22%       <1 min     140k
        now     Read · src/tui/view.rs
 finished · 2 ─────────────────────────────────────────────────────────────────────────────────────
   3.   Fix the footer keys                                                 —   12 min ago     835k
   4.   Review the docs conventions                                         —   38 min ago     3.2M
 esc back · ↑↓ run · enter open run · ←→ other agent · a agent · q quit                      v0.1.0
```

(Render with made-up data, without the empty lines between the runs and the
footer.)

- **Header**: the star (`✦` in `claude` alternating with `✧` every half second
  if anything in the block is running, blank if not), the name (`claude-soft`
  in bold with something running, plain `ink` if everything finished) and, in
  gray, the origin label (the same as in the list) and `· N runs`: all the runs
  on the screen, running and finished. On the right, `N working now` in
  `claude` bold and, in gray, `· N finished · N tok` (the tokens of the whole
  block); a part that is 0 does not show. On narrow screens the right side
  loses its parts from the last one (the tokens first); when not even one fits
  next to the whole left side, the label and `· N runs` stay only if there is
  room for them, and at the end the name is cut with `…`.
- **defined in**: a gray line, indented 3 spaces, with the path of the `.md`
  that defines the agent, with `~` for the home. Only for a defined agent: a
  Claude Code agent or a workflow step without a defined agent has no such
  line.
- **running in**: the group's folder (the [launch
  folder](how-it-works.md#launch-folder), or the `cwd` if it is not known),
  with `~`, in the same column. No line in the `global` group. Both paths are
  cut with `…`.
- **Sections**: the rule `working now · N` (in `claude`) with the running runs
  and the rule `finished · N` (in `claude-soft`) with the finished ones, each
  only if it has rows. The first rule on screen carries the column headers on
  its right: `ctx`, `time` and `tokens`.
- **Running rows**, in the order of the block in active (the one that started
  latest on top): cursor, number, star, the task on one line (`ink`; in bold
  under the cursor; `—` if there is none) and, flush right, `ctx`, `time` and
  `tokens` as in the [list](#active-the-monitor). Under each one, starting in
  the task's column, `now` (its last tool, or `—`) and, only if it has invoked
  a skill, `skills` (the last one it loaded); the labels in `subtle`, the
  values in gray. A blank line separates two running rows.
- **Finished rows**: **one per run**; here they are not merged. From the most
  recent to the oldest, each on a single line: number, the task in gray, `—`
  in `ctx`, `N min ago` and its own tokens. An `Agent` call in the window with
  no linked run is one more finished row, in its place by the time of the
  call, with `—` in `tokens`.
- **Numbers**: 1 to N straight through the screen; they are the `i` of `run i
  of n` in the [run screen](#the-run-screen). The cursor `❯` is always on a
  run; on opening, on the first one.
- **Footer**: ` esc back · ↑↓ run · enter open run · ←→ other agent · a agent ·
  q quit`; `←→ other agent` only if active shows more than one block and `a
  agent` only if the agent is a defined one and its block shows in the list. If
  the runs do not all fit, the ones in view go on the right, left of the
  version (`2-4/7`). When it does not fit, the version goes first and then
  `a`, `←→`, `↑↓` and `enter`, in that order; `esc` and `q` are never removed.

On narrow screens the task is cut first, with `…`; below 70 columns `ctx` and
`tokens` go with their headers and only `time` stays; if not even 8 columns
are left for the task, `time` goes too. With little height the blank line
under the paths goes first and then the two paths, as long as the runs keep 4
lines; the runs scroll.

Keys:

- `↑` `↓` (or `k` `j`), `Home`/`g`, `End`/`G` and the digits (two in a row make
  a two-digit number): choose a run; at the ends it does not move.
- `Enter`: opens the [run screen](#the-run-screen) of the run under the
  cursor; for a call without a run, a minimal one.
- `←` `→` (or `h` `l`): the previous or next block of active, in screen order,
  without leaving the screen; it opens on its first run and the list cursor
  follows. With a filter, only the blocks it leaves. At the ends it does not
  move.
- `a`: opens the [agent details](#agent-details), only for a defined agent
  whose block shows in the list; `Esc` or `Enter` there come back here.
- `PageDown` or `space`, `PageUp` and the mouse wheel: scroll the runs; if the
  cursor's run ends up out of view, the cursor moves to one that shows whole.
- A click on any line of a run (its row, `now` or `skills`) chooses it; a click
  on the one already chosen opens it, like `Enter`.
- `r`: rereads the definitions.
- `Esc`: back to active, with the cursor on this block.
- `q` (and `Ctrl+C`) quits. `/`, `Tab` and `o` do nothing here.

The scroll follows the cursor: its run shows whole, with its `now` and
`skills`, and with its rule if it is the first of a section.

The screen goes **without the filter**: it shows all the runs of the block
even if what is typed in `/` leaves only some of them in the list. The ` ×N`
of the agent line counts only the running runs the filter leaves, so `N runs`
and the `n` of `run i of n` (all the runs of the block, running and finished)
can be greater than it, even without a filter.

It refreshes live, like active, and the cursor stays on the **same** run: if
it ends, it moves to `finished` and the cursor goes with it. If the run under
the cursor leaves the window, the cursor passes to the one that takes its
place (or the last one). If the whole block leaves the window, the screen
closes and goes back to active.

## The run screen

`Enter` on a run of the [agent screen](#the-agent-screen) opens it, full
screen too. Also on an `Agent` call with no linked run, which opens a minimal
screen: no task (`—`), no tools, no steps (`—`) and with `ended … ago` in the
header. Example of a running one that launched two other runs, at 100 columns:

```
 visual [own] › ✦ run 1 of 4                                     running 4 min · 5 tools · 412k tok
   launched by  session 23dabecb · “paint the active screen with tokens”
   running in   ~/code/shop/api

 usage ───────────────────────────────────────────────────────────────────────────────── sonnet-5-5
   context  71%   ████████████████████████████▍░░░░░░░░░░░  142k of 200k
   tokens   412k  input 18k · output 24k · cache read 351k · cache write 19k
 task ─────────────────────────────────────────────────────────────────────────────────────────────
   paint the active screen
   with design B
 skills · 2 ───────────────────────────────────────────────────────────────────────────────────────
   3 min ago  frontend-design:frontend-design
   4 min ago  superpowers:verification-before-completion
 tools ────────────────────────────────────────────────────────────────────────────────────────────
   Read 3 · Bash 1 · Edit 1
 launched · 2 ─────────────────────────────────────────────────────────────────────────────────────
   1. ✦ Explore [claude]           2 min · 9 tools · 64k tok         Find where ctx is computed
   2.   general-purpose [claude]   4 min ago · 21 tools · 210k tok   Check ratatui gauge widths
 steps · 5 ────────────────────────────────────────────────────────────────────────────────────────
   5 s ago    Bash · git diff --stat
   1 min ago  Read · src/d.rs
   2 min ago  Edit · src/c.rs
   3 min ago  Read · src/b.rs
   4 min ago  Read · src/a.rs
 esc back to visual · ←→ other run · tab launched · pgup/pgdn steps · q quit                 v0.1.0
```

(Render with made-up data, without the empty lines between the steps and the
footer.)

- **Header**: in gray, where it comes from: the agent's name, its origin label
  (the same as in the list: `[own]`, `[own] via workflow`, `[workflow]` or
  `[claude]`) and `›`. Then the run: `✦` in `claude` alternating with `✧` every
  half second if it is running, and `run i of n`, in `claude-soft` bold if
  running and plain `ink`, without the star, if finished. `i` and `n` are the
  run's number and the total in the agent screen, in its order; it always
  shows, `run 1 of 1` too. On the right, in gray, `running N min · N tools · N
  tok`, or `ended N min ago · N tok` if the run already ended; `· N tok` is
  left out when the run has no tokens. Below 70 columns `running N min ·` and
  `· N tok` are removed; below 50, the label; then the agent's name is cut
  with `…` and, with fewer than 4 columns for it, goes away with its `›`. If
  the run is no longer in the window of active there is no `run i of n`: the
  header is the star, the name and its label.
- **Launched by**: a gray line right under the header, indented 3 spaces,
  `launched by  …` with who launched the run (see [Who launched
  whom](how-it-works.md#who-launched-whom)): `session 23dabecb · “first prompt”` (the first 8
  characters of the session id and its first prompt, in quotes, cut with `…`;
  without the prompt if there is none or no room); another run, as its name and
  label (`launched by  backend [own]`; `p` opens it); or `a run no longer on
  disk`. Without that datum, there is no line.
- **running in**: the group's folder, as in the agent screen (the launch
  folder, or the `cwd` if it is not known), with `~` for the home and cut with
  `…`, in the same column as the value of `launched by`.
- **Temp folder notice**: only if the run was launched from Claude's temp folder
  (`Run.temp_cwd`, see [Temporary `cwd` from Claude](how-it-works.md#temporary-cwd-from-claude)).
  A gray line under the previous one, indented 3 spaces: `launched from Claude's
  temp folder · project taken from the session` if the `cwd` was resolved to the
  session's, or `… · project unresolved` if not. If 8 or more columns are left
  for the path, ` · <temp path with ~>` follows, cut with `…`.
- **cwd**: only if the subagent's `cwd` differs from the launch folder (and both
  are known). A gray line under the previous ones, `   cwd          <path>`, with
  `~` for the home and the path in that same column, cut with `…`.
- A blank line separates those lines from the sections only if the body is 14
  lines or more.
- **usage**, the first section: the rule carries the model on its right,
  without the `claude-` prefix nor the date suffix (`sonnet-5-5`; `—` if
  unknown). Below, two rows, with the label in gray and the row's figure in
  `ink` bold:
  - `context`: the percentage (amber from 85%), a bar of 8 to 40 columns
    (amber from 85% too) and `142k of 200k` in gray. On narrow screens the bar
    shrinks and then goes.
  - `tokens`: the run's total and its breakdown, `input · output · cache read ·
    cache write`, in short form, with the labels in gray and the figures in
    `ink`. If it does not fit, the breakdown goes on in more lines under its
    column, without losing any figure.

  A figure with no data is `—`. See [Tokens and
  context](how-it-works.md#tokens-and-context). With no tokens, context or model
  (a call without a run, or a run that has no reply yet) the section does not
  show.
- **task**: the run's whole first message, with its line breaks, wrapped to the
  width and with a maximum of 8 lines (the last ends in `…` if cut); `—` if it
  could not be extracted. It does not fall back to the `description`. Blank
  lines in the task are not painted.
- **skills · N**: every skill the run invoked (see [What is read from each
  run](how-it-works.md#what-is-read-from-each-run)), the most recent first:
  when, in gray, and its name in `ink`, cut with `…`. At most 6 show, the most
  recent ones, with `1-6/N` on the right of the rule. With no skills, the
  section does not show.
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
  cache](how-it-works.md#kept-history) has no steps: under the `steps` rule it says `from the
  history cache · steps are not kept`.
- **Footer**: ` esc back to <agent> · ←→ other run · tab launched · enter open
  · pgup/pgdn steps · p parent · q quit`, each one only when it applies: `esc`
  names where it goes back to, the agent of the agent screen or of the run
  this one was opened from (plain `esc back` if it goes back to active); `←→
  other run` if the agent has more than one run; `tab launched` if the run
  launched any; `enter open` with a child selected; `pgup/pgdn steps` unless it
  comes from the history cache; `p parent` if it was launched by a run still
  in the data. If it does not fit, the version goes first; then `pgup/pgdn`,
  `p`, `tab`, `←→` and `enter` are removed, in that order, and at the end
  `back to <agent>` is left as `back`; `esc` and `q` are never removed.

With little height the steps rule and up to 3 steps (or the history cache note)
are reserved first, under the header; the rest goes in whole or not at all, in
this order: the launched-by line, `running in`, the temp folder notice, the
`cwd` line, the blank line, the tools (rule and lines), the task's rule and
first line, the usage section, the second line of the task, the skills, the
launched runs and the rest of the task (up to 8 lines); whatever is left goes
to steps. More than the available height is never painted.

Keys:

- `Esc`: go back to the run it came from (see `Enter` and `p` below), as it was
  (scroll and selection); if there is none, go back to the agent screen, with
  its cursor on the run that was open. If the agent's block left the window
  meanwhile, it goes back to active.
- `→` `←` (or `l` `h`): the next or previous run of the agent, in the order and
  numbering of the agent screen: `→` toward the higher numbers (the oldest
  finished one last), `←` back toward the first. At the ends, or without
  siblings, they do nothing. The steps go back to the first line. Siblings are
  computed over the unfiltered groups: what is typed in `/` does not trim them.
- `↑` `↓` (or `k` `j`): move the steps one line. They no longer change agent.
- `PageUp`, `PageDown` or `space` (one page), `Home`/`g`, `End`/`G`: scroll the
  steps; `End` leaves the last page full.
- `Tab` / `Shift+Tab`: select the next or previous run in «launched» (from none,
  the first or the last); past the end, none is selected. The digits select
  that number directly (two in a row make a two-digit number).
- `Enter`: with a launched run selected, opens it; without, like `Esc`.
- `p`: opens the run that launched this one, only if it was launched by a run
  still in the data.
- `q` (and `Ctrl+C`) quits the program.
- `/`, `o`, `r`, `a`, the mouse wheel and clicks are ignored. The agent details
  open from the agent screen, not from here.

Opening a launched run (`Enter`) or the parent (`p`) stacks the open one, so
`Esc` walks back through them, and the footer names the agent of the run it
goes back to. `←` `→` replace the open run and keep the stack; on a launched
run or a parent they walk the block of **its** agent, and `run i of n` counts
that block. If the selected launched run disappears, the selection moves to
the last one left.

The screen refreshes with the usual 2 s cycle and stays on the **same** run. If
the run ends while it is open, the screen **does not close**: it says `ended N
min ago`; if the run disappears from disk, it keeps the last snapshot. If the
agent's block leaves the window meanwhile, the header loses `run i of n`, the
footer says `esc back` and `Esc` goes back to active, with the cursor on the
block that contains that run if it is still in the list.

## Keys and formats

| Key | Does |
|---|---|
| `↑` `↓` (or `k` `j`), `Home`/`End` (or `g`/`G`) | Moves the cursor. In the agent screen, chooses a run. In the run screen, they scroll the steps (`↑` `↓`, one line) |
| `PageDown` or `space`, `PageUp` | Pages down or up (view and cursor). In the agent screen, the runs; in the run screen, the steps |
| mouse wheel | Moves the view 3 lines; if the cursor ends up outside, it drags it. Also in the agent screen; nothing in the agent details or the run screen |
| left click | On a tab of the top row, switches to that view (and closes the agent details if open). On a list row (in active, on any line of a block; in the agent screen, on any line of a run), selects it; on the one already selected, does the same as `Enter`. See [mouse](#mouse) |
| digits | Jumps to that numbered line (agents, folded groups and active blocks; in the agent screen, runs); two in a row make a two-digit number. In the run screen, selects that launched run |
| `Enter` | In all and ranking, opens the agent details, or unfolds/folds an unused group. In active, on any block, opens the [agent screen](#the-agent-screen); there, opens the [run screen](#the-run-screen) of the run under the cursor (a call without a run too). In the agent details and the run screen, goes back (in the run screen, with a launched run selected, it opens that one) |
| `Esc` | Goes back one level: from the run screen to the runs opened from it and then to the agent screen; from the agent screen to active, with the cursor on its block; from the agent details to where they were opened |
| `/` | Filters by name or project (in active, by name, group folder, `cwd` or task, over all rows) as you type; `Enter` fixes the filter, `Esc` removes it |
| `Tab` | Cycles active → all → ranking. In the run screen, selects the next launched run (`Shift+Tab`, the previous one) |
| `a` | Toggles active and all. In the agent screen, opens the agent details (only for a defined agent) |
| `←` `→` (or `h` `l`) | In the agent screen, the previous or next block of active. In the run screen, the previous or next run of the agent, in the order of the agent screen |
| `o` | In all, order within groups: recent or worked |
| `p` | In the ranking, period: 7 days → 30 days → all time. In the run screen, opens the run that launched it (if it is still in the data) |
| `s` | In the ranking, sort key: time worked → tokens → runs |
| `r` | Rereads the definitions now (not in the run screen) |
| `q` or `Ctrl+C` | Quits |

The footer only shows the main ones; `PageUp`/`PageDown`, the digits and `r`
work even if they do not show. The [agent screen](#the-agent-screen) and the
[run screen](#the-run-screen) have their own keys, listed in their sections.

Times: `20 s ago`, `3 min ago` and `2 h 30 min ago` below one day; `N d ago`
from 1 to 29 days; from 30 days on, the local date (`10/05/2026`, as
`MM/DD/YYYY`). Dates with time in the runs list: `Nov 4 18:12`. Worked
durations: `<1 min`, `N min` below one hour and `N h M min` from there on (`1 h
9 min`); truncated to the minute.

Colors: Claude Code's colors (`claude`, `claude-soft`, `ink`…) in truecolor if the
terminal announces `COLORTERM=truecolor`, and their 256-color approximation if
not. It paints no background: it uses the terminal's.

### Mouse

The screen captures the mouse so the wheel and the left click work:

- **Wheel**: moves the list, as in the table above.
- **Click on a tab** (`active`, `all`, `ranking` in the top row): switches to
  that view; if the agent details are open, it closes them.
- **Click on a row**: moves the cursor to it. A click on the row the cursor is
  already on opens it, exactly like `Enter`: the agent details, the agent
  screen in active, or unfolds/folds a group. There is no double click: it is two
  separate clicks. In active, any of the lines of a block (the agent line, its
  rows and their `skill` and `now`) counts as that block; group rules, the column header, blank lines and
  notes do not respond.
- In the agent screen, which has no tabs, a click on any line of a run (its
  row, `now` or `skills`) chooses it and a second click opens it; the wheel
  scrolls the runs.
- With the agent details open only the tabs respond; with the run screen open
  or while typing the filter (`/`), clicks do nothing.
- Other buttons, dragging and moving the mouse do nothing.

To select text with the terminal you have to hold `Shift` while dragging (in
iTerm2, `Option`).

## The `--plain` table

It does not change with the screen views:

```
STATE     NAME                     LAST USED       WORKED  PATH
ACTIVE    backend                  6 min ago       35 min  ~/code/shop/api/.claude/agents/backend.md
ACTIVE    database                 20 min ago      12 min  ~/code/shop/api/.claude/agents/database.md
ACTIVE    docs                     38 min ago       4 min  ~/code/shop/web/.claude/agents/docs.md
ACTIVE    database                 38 min ago       6 min  ~/code/shop/web/.claude/agents/database.md
ACTIVE    backend                  39 min ago      19 min  ~/code/shop/web/.claude/agents/backend.md
inactive  ai-engineer              never                   ~/code/legacy/.claude/agents/ai-engineer.md

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
remain in the [history cache](how-it-works.md#kept-history) count in `WORKED` and `LAST USED`
too, and `--plain` also updates that cache. A running agent looks like this:

```
RUNNING   demo    7 min ago           7 min  ~/.claude/agents/demo.md
```

With `-v`, the open Claude sessions follow the summary line, one per line, with
their pid and command. `-v` only has an effect with `--plain`: on the screen the
sessions are a counter in the header.

## The update panel

`claude-agents update` in a terminal opens a panel of 11 rows below the
prompt, without taking over the screen (excerpt with made-up data):

```
 claude-agents update ──────────────────────────────── released 2 d ago

   1.1.0  →  1.2.0                         what's new in 1.2.0
   package     macos · aarch64 · 2.1 MB    · feat: update command
   install to  ~/.local/bin/claude-agents  · fix: narrow terminals

 ████████████░░░░░░░░░░░░   50%  1.1 MB / 2.1 MB

 esc cancel
```

- **Rule**: `claude-agents update` and, if it fits, how long ago the release
  was published.
- **Data**: the running version `→` the new one; the package (platform,
  architecture, `musl` if it applies, and size); where it installs, with `~`
  for the home and cut from the front (`…/bin/claude-agents`) if it does not
  fit. Up to date, the first line reads `claude-agents 1.1.0`.
- **What's new**: on the right, the first three bullets of the release notes
  (without GitHub's `by @who in <url>`), or `no release notes`. It goes away
  when there is no room for it.
- **Status**, one line per phase: `✦ checking GitHub for a new release…`; `✓
  claude-agents 1.1.0 is the latest version`; `↑ claude-agents 1.2.0 is
  available` (with `--check`); `↑ install claude-agents 1.2.0?`; the download
  bar in `claude` with percent and MB (only MB if the size is unknown; on a
  narrow terminal the bar shortens and then goes, the figures stay); `✦
  verifying sha-256`; `✦ installing to <path>`; `✓ claude-agents 1.2.0
  installed`, plus `· checksum not published: not verified` in amber when it
  could not be checked; `✗ cancelled · nothing was touched`. A refusal reads
  `✗ this claude-agents came from cargo · --force overwrites it` (or `is a local
  build under target/`), and any other error goes in red over up to three
  lines.
- **Keys**, last row: `enter update · esc cancel` at the question; `esc
  cancel` while checking and downloading; afterwards, what to type next
  (`claude-agents update` after `--check`, `claude-agents` after installing, or
  the command that updates a refused copy).

`Enter` or `y` install; `Esc`, `q`, `n` or `Ctrl+C` cancel while checking, at
the question and during the download: nothing has been written yet. Verifying
and installing cannot be interrupted. When it ends, the last frame stays in the
terminal above the prompt.

Without a terminal (stdin or stdout redirected), or if the terminal does not
answer the query for the cursor position, the same steps come out as plain
lines (errors to stderr):

```
claude-agents 1.1.0 → 1.2.0
  package     claude-agents-macos-aarch64.tar.gz (2.1 MB)
  install to  ~/.local/bin/claude-agents
  · feat: update command
install it? [y/N]
```

In a terminal that answer is read from the keyboard (`y`, `Y` or `yes`) and the
download shows MB and percent on one line. Without a terminal there is no
question: it needs `--yes`, or it stops with `no interactive terminal: pass
--yes to update without asking`. With `--check` and no terminal, the only line
is the result: ``claude-agents 1.2.0 is available · run `claude-agents
update` ``.
