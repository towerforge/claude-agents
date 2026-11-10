# Propuesta C — Híbrido: en marcha en árbol, terminados compactos

Separa lo vivo de lo hecho. Arriba, **lo que corre**, como en la captura: por
sesión, un run por línea con su tarea y, debajo de cada uno, una sublínea gris
con lo que está haciendo ahora. Abajo, **lo terminado**, una línea por agente
con `×N`.

```
  active   all   ranking                         window 10 min · 4 running · 3 in window · 2 sessions · 1.1M tok

 running ────────────────────────────────────────────────────────────────────────────────────────────────────
 ● claude-agents  revisa la documentación y el makefile
 ❯ ✦ docs      Revisando convenciones de docs/README                          ▰▰▱▱ 41%   1m 52s · ↓ 140.3k
     ⎿ Read · docs/interface.md
   ✦ docs      Revisando targets del Makefile                                 ▰▱▱▱ 22%   1m 52s · ↓ 106.8k
     ⎿ Bash · grep -n release Makefile
   ✦ docs      Leyendo contact-form.astro y cookie-table                      ▰▰▱▱ 48%   1m 52s · ↓ 160.5k
     ⎿ Skill · superpowers:brainstorming
 ● offing-base  añade el selector de idioma al header
   ✦ frontend  Implementa LanguageSwitcher en header.tsx                      ▰▰▰▱ 71%      9 min · ↓ 412.0k
     ⎿ Edit · src/components/header.tsx

 done in window ─────────────────────────────────────────────────────────────────────────────────────────────
   ○ visual ×3        claude-agents   Ajusta el pie de la ficha                    4 min ago · ↓  88.1k
   ○ Explore          offing-base     Busca dónde se lee el locale                 6 min ago · ↓  51.2k
   ○ general-purpose  offing-base     Comprueba los textos de i18n                 7 min ago · ↓  32.4k
 ↑↓ move · enter details · tab mode · / filter · q quit                                                v1.2.0
```

## Qué cambia

- **Dos secciones** con regla: `running` y `done in window`. Desaparece la
  agrupación por proyecto como regla; el proyecto pasa a la línea de sesión
  (arriba) o a una columna (abajo).
- **Running:** `● proyecto  prompt de la sesión`, y debajo cada run con `✦`,
  nombre, tarea, `ctx`, `tiempo · ↓ tokens`. Sublínea `⎿` en gris con la
  última herramienta (lo de hoy en `now`), como hace Claude Code.
- **Done:** una línea por agente y proyecto, `×N` si fusiona, tarea del último
  run, `N min ago · ↓ tokens`.
- `cwd` y `skill` van a la ficha (la skill sale en la sublínea cuando es la
  última herramienta).

## Pros / contras

- ✅ Lo que corre se lee como en Claude Code, con su tarea y su `now`.
- ✅ Lo terminado no estorba: compacto y al final.
- ✅ Cada run en marcha ocupa 2 líneas (hoy 3–4).
- ❌ Dos órdenes distintos en la misma lista (sesión arriba, recencia abajo).
- ❌ Un proyecto aparece repartido entre las dos secciones.
