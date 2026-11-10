# How it works

What claude-agents reads from disk and how it turns it into the screen's
figures, and how it updates itself. The screen itself is described in
[interface.md](interface.md).

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
   run](#what-is-read-from-each-run)). Each call also gets the [launch
   folder](#launch-folder) of its session.
3. A **global** agent matches any session. A **project** agent only matches
   sessions launched inside that project (the launch folder; the call's `cwd`
   if it is not known): two projects with a `frontend` agent do not contaminate
   each other.
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
   name: a global agent matches any run, a project one only runs whose session
   was launched inside the project (see [Launch folder](#launch-folder); the
   run's `cwd` if it is not known). If that `cwd` is Claude's temp folder, the
   real project's is used (see [Temporary `cwd` from
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

## What is read from each run

For the active monitor, the following is also taken from each run's transcript:

- **Task** (`task`): the first line with content of the first user message with
  text (if `content` is a list, the `text` blocks count; a `tool_result` is not
  a prompt). The leading `#`s and the boilerplate `ENCARGO (de …):` (or `TASK
  (from …):`) are removed and whitespace is collapsed: `# ENCARGO (de Ana,
  05-11-2026): copia de seguridad de TODO` becomes `copia de seguridad de
  TODO`. The boilerplate is input data (the prompt's literal text): both
  spellings are recognized as they are.
- **Last tool** (`last_tool`): the last `tool_use` block in the transcript, as
  `Name · short argument`. `Bash`, its `command` on one line; `Read`, `Edit` and
  `Write`, their `file_path` relative to the `cwd` if it hangs from it and,
  otherwise, only the last component (`NotebookEdit`, the same with
  `notebook_path`); `Grep` and `Glob`, their `pattern`; `Skill`, its `skill`
  (`Skill · superpowers:brainstorming`); the rest, only the name.
- **Skill** (`skill`): the last non-empty `input.skill` of a `Skill`
  `tool_use` block in the transcript (`superpowers:brainstorming`), as is. A
  loaded skill stays in force until another one is loaded, so it is kept even
  when that step falls out of the last 200 steps. Empty if the run never invoked one.

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

## Tokens and context

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

Token figures go in short form: `999`, `1.5k` (one decimal below 10k), `86k`,
`1M`, `1.1M`, `31.4M`, `142M`, without a trailing `.0`.

## Who launched whom

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

## Temporary `cwd` from Claude

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
  notice](interface.md#run-details). A call without a run does not have it.
- **Call** (`Invocation.cwd`): the first non-temporary `cwd` in the very file
  where the call is.

If there is no real `cwd`, the temporary one is kept: none is made up. With the
`cwd` resolved, the run is grouped under its project and matches its project
agents like any other (when the [launch folder](#launch-folder) is not known;
if it is, that one decides, and it is never a temp folder).

## Launch folder

A subagent's `cwd` is wherever the main agent was when it launched it, and it
follows every `cd`: the subagents of one session can carry `~/app`,
`~/app/src/css` or `/tmp`. Claude Code, though, loads `.claude/agents/` from
the folder where the session was launched. That folder is the launch folder:

- **Run** (`Run.launch_dir`): the first non-temporary `cwd` in the session's
  main transcript (`~/.claude/projects/<project>/<session>.jsonl`), read with
  the same first lines and the same per-session store as the [temporary
  `cwd`](#temporary-cwd-from-claude). Nested runs and workflow steps share the
  one of their session. If it cannot be read yet, there is none and it is
  retried on the next refresh.
- **Call** (`Invocation.launch_dir`): the same, from the main transcript of the
  session the call's file belongs to (a call made from inside a subagent takes
  the main session's). Until it is found, it is looked for again only when that
  transcript grows.

It decides three things, falling back to the `cwd` when it is not known:

- the active group (see [Active: the monitor](interface.md#active-the-monitor)),
  so all the subagents of a session fall in one group;
- which project agents match, and therefore `[own]` or `[claude]`, the time,
  runs and last use of each agent, in every view and in `--plain`: a project
  agent launched after a `cd /tmp` is still the project's;
- the project in the [run details](interface.md#run-details) header, which adds
  a `cwd` line when the subagent's differs.

The subagent's own `cwd` is kept apart: the active list does not show it, but
`/` searches it there, and it is what the agent details' runs list shows.

## Kept history

Claude Code deletes old transcripts (`cleanupPeriodDays`, 30 days by default),
and with them the runs would vanish from the totals. So the tool keeps finished
runs in its own cache:

```
$XDG_CACHE_HOME/claude-agents/runs.jsonl    # if XDG_CACHE_HOME is an absolute path
~/.cache/claude-agents/runs.jsonl           # otherwise
```

The screen and `--plain` both update it. Besides it, the tool only writes the
[update note](#the-daily-check) next to it and, on `claude-agents update`, its
own binary.

- One JSON object per line and per run, with `"v":1` (a line of another
  version, or a broken one, is skipped). Only runs that are not in progress go
  in; a running one goes in once it ends. It stores what the views need: agent,
  description, session and its first prompt, links, `cwd`, launch folder
  (`launch_dir`), times, worked time
  and its 15-minute slots, tool counts, task, tokens, model and context. Not the
  steps, the last tool, the skill or the temporary `cwd`. Around 1 MB for some
  400 runs.
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
- Lines written before `launch_dir` existed are read without a launch folder
  (they are grouped and matched by their `cwd`); there is no migration and the
  version stays `"v":1`.
- Tests never write there: they use their own temporary folders, or no cache.

To start over, delete the file.

## Updating

`claude-agents update` replaces the running binary with the one from a GitHub
release. The panel and the text it prints are in
[interface.md](interface.md#the-update-panel).

1. It asks the GitHub API (`api.github.com`, anonymous) for the latest release,
   or for the tag `vX.Y.Z` with `--to`. No release, a missing tag, no
   connection or the API's rate limit end it with an error.
2. It decides whether the release is worth installing: a newer one always; the
   one named with `--to` if it differs from the running version (downgrades
   included); and anything with `--force`, the same version too. Otherwise it
   says the running version is the latest and stops.
3. With `--check` it stops here and says what is available.
4. Before downloading anything it looks at where the running binary lives (with
   symbolic links resolved). Under `.cargo/bin` (a `cargo install`) or under
   `target/debug`, `target/release` or `target/<triple>/...` (a local build)
   it stops and names what updates that copy instead (`cargo install --git
   https://github.com/towerforge/claude-agents` or `make build`); `--force`
   goes on anyway. It then checks it can write in that folder: if not, it stops
   and suggests `sudo`, or `CLAUDE_AGENTS_INSTALL_DIR` with `install.sh`.
5. It asks before installing, unless `--yes`. Without a terminal there is no
   one to ask: without `--yes` it stops with an error.
6. It downloads the archive for the platform it was built for
   (`claude-agents-<os>-<arch>[-musl].tar.gz`, the same names as
   [releases.md](releases.md#archives); a musl build stays on musl), in memory.
7. It checks its SHA-256 against the archive's line in `checksums.txt`. A
   mismatch stops it with nothing written. If the release has no
   `checksums.txt`, or no line for that archive, it installs anyway and says
   `checksum not published: not verified`.
8. It takes the `claude-agents` binary out of the archive, writes it to a
   temporary file in the same folder with mode `755` and moves it over the old
   one with a `rename`, so a failure halfway leaves the old binary intact. The
   copy that is running keeps working until it exits.

The exit code is 1 only when something fails (including a refusal in step 4);
up to date, available with `--check`, installed and cancelled are 0.

### The daily check

When the interactive screen opens, a background thread asks whether there is
a newer release; `--plain` never asks. The answer is kept in a note next to the
[history cache](#kept-history):

```
$XDG_CACHE_HOME/claude-agents/update-check.json    # if XDG_CACHE_HOME is an absolute path
~/.cache/claude-agents/update-check.json           # otherwise
```

It holds the time of the check and the latest version GitHub had
(`{"checked_at": "...", "latest": "1.2.0"}`). While it is less than 24 h old,
the screen answers from it without going to the network; after that it asks
GitHub again and rewrites it (a temporary file and `rename`). A note dated in
the future, or one it cannot read, counts as no note.

- If the latest release is newer than the running version, the footer
  announces it ([interface.md](interface.md#the-screen)).
- Any failure (no connection, rate limit, an unreadable note) shows no notice
  and no error; the screen does not wait for the thread.
- `claude-agents update` rewrites the note whenever it learns the latest
  release (not with `--to`), and deletes it after installing, so the notice
  does not outlive the update.
- `CLAUDE_AGENTS_NO_UPDATE_CHECK` turns the screen's check off with any value
  except empty or `0`. It does not affect `claude-agents update`.
- Tests never touch the real note: they use their own temporary folders.

### Network

The tool goes to the network only for updates: `api.github.com` for the
release data, and GitHub's download URLs for the archive and `checksums.txt`.
That happens on the screen's daily check and on `claude-agents update`. To keep
it off the network, set `CLAUDE_AGENTS_NO_UPDATE_CHECK=1` and do not run
`claude-agents update`.

## Limitations

- «Running» relies on the meta's `finished` and the `.jsonl`'s date: a run that
  waits more than 10 min without writing anything (an unanswered permission, a
  long tool) shows as interrupted until it writes again. A workflow run, whose
  meta carries no `finished`, stops showing as running after 2 min without
  writing, and keeps showing for up to 2 min after finishing.
- Active merges the finished runs of the same agent in a project into a single
  row: they can only be seen one by one from the details, with `←` `→`, and the
  `recent` and `in window` counters count those rows, not runs (running runs
  get a row each). An own agent and a `[claude]` one with the same name, or a
  `visual [workflow]`, are different rows. The window (`-m`) decides which runs
  get in: one older than it does not count in the `×N`.
- Old calls, from before Claude Code stored one transcript per run, have no run
  of their own: they add no time or days, and in the details they only count in
  `+ N old calls with no recorded duration`. If they fall within the window, the
  active monitor does show them, as a finished run with no figures (merged with
  those of its agent).
- Claude Code agents (`general-purpose`, `Explore`, `fork`…) have no definition
  on disk and do not show in all or in the ranking, even if they have runs. They
  do show in active, running and as finished within the window, but they add no
  time or runs to any agent; there `Enter` opens the run details, but there are
  no agent details (`a` does nothing). The same goes for a project agent from a
  session launched outside its project; what counts is the [launch
  folder](#launch-folder), not the subagent's `cwd`.
- If the parent session's transcript cannot be read (or has no real `cwd` in its
  first lines), there is no launch folder and a run with a temporary `cwd`
  keeps the temporary one: it matches
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
- Runs from the history cache have no steps, last tool or skill, and nothing in
  them changes any more. Those kept before the launch folder existed, if their
  transcript is gone, go by their `cwd`: one launched after a `cd` can show in
  another group, or as `[claude]` if it left its project.
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
- `claude-agents update` uses GitHub's API without a token, which allows 60
  requests per hour per address; past that it fails with `rate limit reached`
  until the limit resets.
- The screen learns of a new release at most once a day: one published right
  after a check shows up to 24 h later (`claude-agents update --check` asks
  right away).
