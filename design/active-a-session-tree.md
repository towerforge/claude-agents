# Propuesta A — Árbol por sesión

La más parecida a la captura de Claude Code: dentro de cada proyecto, cada
sesión es un nodo (`●` + su primer prompt) y cuelga de ella **una línea por
run**, sin fusionar. La tarea va en la propia línea, que es lo que hoy falta.

```
  active   all   ranking                         window 10 min · 4 running · 3 in window · 2 sessions · 1.1M tok

 claude-agents · 3 running · 1 recent ───────────────────────────────────────────────────────────────────────
 ● revisa la documentación y el makefile                                                        2 min · 408k
 ❯ ✦ docs      Revisando convenciones de docs/README                          ▰▰▱▱ 41%   1m 52s · ↓ 140.3k
   ✦ docs      Revisando targets del Makefile                                 ▰▱▱▱ 22%   1m 52s · ↓ 106.8k
   ✦ docs      Leyendo contact-form.astro y cookie-table                      ▰▰▱▱ 48%   1m 52s · ↓ 160.5k
   ○ visual    Ajusta el pie de la ficha                                               4 min ago · ↓  88.1k

 offing-base · 1 running · 2 recent ─────────────────────────────────────────────────────────────────────────
 ● añade el selector de idioma al header                                                       11 min · 712k
   ✦ frontend  Implementa LanguageSwitcher en header.tsx                      ▰▰▰▱ 71%      9 min · ↓ 412.0k
   ○ Explore   Busca dónde se lee el locale                                            6 min ago · ↓  51.2k
     └ ○ general-purpose  Comprueba los textos de i18n                                 7 min ago · ↓  32.4k
 ↑↓ move · enter details · tab mode · / filter · q quit                                                v1.2.0
```

## Qué cambia

- **Nivel 1:** proyecto (la regla de hoy, igual).
- **Nivel 2:** sesión, `●` en `ink` + el primer prompt de la sesión
  (`Run.session_prompt`, ya existe). A la derecha, el tiempo y los tokens
  sumados de sus runs. No es seleccionable.
- **Nivel 3:** un run por línea: `✦` (parpadea) si corre, `○` si terminó;
  nombre en columna fija; tarea; `ctx` solo si corre; `tiempo · ↓ tokens`
  como en la captura.
- **Anidados:** un run lanzado por otro run (`Run.parent_run`) cuelga con `└`.
- El origen (`[own]`, `[claude]`, `[workflow]`) sale del cuerpo: se ve en la
  ficha. Si se quiere, el nombre se tiñe por origen.
- `now`, `skill` y `cwd` desaparecen de la lista: van a la ficha (`enter`).

## Pros / contras

- ✅ Densidad ×3–4: una línea por run en vez de un bloque de 3–4.
- ✅ Se ven los runs paralelos del mismo agente, cada uno con su tarea.
- ✅ Se lee quién lanzó qué de un vistazo.
- ❌ Se pierde el «qué está haciendo ahora» (`now`) sin abrir la ficha.
- ❌ Con muchos runs terminados del mismo agente la lista crece (no hay `×N`).
- Cambio grande en `status.rs` (agrupar por sesión y no por agente).
