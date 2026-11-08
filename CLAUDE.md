# claude-agents

CLI en Rust (un solo crate binario, edición 2021) que lista los subagentes de
Claude Code que hay en disco y marca cuáles se han invocado hace poco, con su
tiempo, tokens y quién los lanzó: pantalla
ratatui al estilo de moon en una terminal, tabla con `--plain`. Corre en el host,
sin Docker (solo `cross` usa Docker para empaquetar Linux). Se publica por
GitHub Actions y se instala con `install.sh`. Detalle en [README.md](README.md)
y en `docs/` (pantalla, funcionamiento, releases).

## Reparto entre agentes

Cada instrucción va al agente de su apartado (`.claude/agents/`):

| Agente | Apartado |
|---|---|
| `data` | Lo que se lee del disco y de `ps`, y su forma: `src/agents.rs`, `src/transcripts.rs`, `src/runs.rs` y `src/runs/` (tokens, enlaces, la caché `runs.jsonl`), `src/sessions.rs`. No hay base de datos |
| `backend` | El cruce y la orquestación: `src/status.rs` y `src/status/pricing.rs`, `main()` de `src/main.rs`, el bucle y el hilo de lectura de `src/tui/mod.rs` y `src/tui/scan.rs`, `Cargo.toml`, `Makefile`, `.github/workflows/`, `install.sh` |
| `frontend` | Lo que sale y cómo se maneja: flags y ayuda (`struct Cli` en `src/main.rs`), columnas y orden, teclas, filtro y ficha (`src/tui/app.rs`) |
| `visual` | Cómo se pinta: `src/report.rs`, `src/tui/view.rs`, `src/tui/theme.rs` y las cadenas de formato. Sin lógica |
| `docs` | `README.md` (en inglés de EE. UU.), `CLAUDE.md` (en castellano), `docs/` (en inglés, sin código), `assets/readme/` (imágenes del README, SVG a mano y con datos inventados) |

- Si toca varios apartados, en este orden: data → backend → frontend → visual →
  docs. Pasa a cada agente lo que dejó el anterior (tipos, funciones, flags).
- Tras cualquier cambio de comportamiento, flags, formato leído, configuración o
  comandos, termina siempre con `docs`, aunque no se pida.
- Ningún agente da algo por hecho sin verificarlo; reporta la salida.

## Reglas

- Comandos por el `Makefile` (`make help`): `make check` (fmt --check + clippy
  -D warnings + tests), `make fmt`, `make run ARGS="…"`, `make build`.
- Nunca `make release` ni `make set-version` sin que Jordi lo pida. Tampoco
  `make -n` sobre ellos ni sobre `package-all`, `package-all-linux` o
  `package-all-macos`: llevan `$(MAKE)` dentro y `-n` los ejecuta igualmente.
- `make release` solo etiqueta y empuja; el CI compila y publica. No adjuntes
  binarios a mano.
- Identificadores y todo lo que el usuario ve (ayuda, mensajes, pantalla, README) en
  inglés de EE. UU.; comentarios del código en castellano.
- Test primero: el test falla, luego el código. Tests junto al código con
  fixtures literales; nunca contra el `~/.claude` real.
- Solo se escribe un fichero: la caché de historial (`runs::history`,
  `~/.cache/claude-agents/runs.jsonl`). Los tests nunca escriben ahí: carpeta
  temporal propia o `Runs::default()`.
- `clippy -D warnings`: lo que nada llama es código muerto y rompe `make check`.
- Dependencia nueva solo si la instrucción lo pide; si crees que hace falta, dilo.
- No toques `.env` ni credenciales. No hagas commits ni `git add` sin petición.
