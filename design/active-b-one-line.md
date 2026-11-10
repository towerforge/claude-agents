# Propuesta B — Una línea por run, columnas de siempre

Mantiene los grupos por proyecto y las columnas actuales (`ctx`, `tokens`,
`tools`, `time`), pero cada run ocupa **una línea** con su tarea, como la
captura. Solo la fila del cursor se despliega con `now` / `skill` / `cwd`.

```
  active   all   ranking                         window 10 min · 4 running · 3 in window · 2 sessions · 1.1M tok

         agent       task                                                    ctx   tokens  tools        time
 claude-agents · 3 running · 1 recent ───────────────────────────────────────────────────────────────────────
 ❯  1. ✦ docs        Revisando convenciones de docs/README              ▰▰▱▱ 41%   140.3k     12       1 min
         │ now    Read · docs/interface.md
         │ skill  superpowers:brainstorming
         │ cwd    ~/Towerforge/claude-agents
    2. ✦ docs        Revisando targets del Makefile                     ▰▱▱▱ 22%   106.8k      8       1 min
    3. ✦ docs        Leyendo contact-form.astro y cookie-table          ▰▰▱▱ 48%   160.5k     15       1 min
    4.   visual ×3   Ajusta el pie de la ficha                                 —    88.1k     21   4 min ago

 offing-base · 1 running · 2 recent ─────────────────────────────────────────────────────────────────────────
    5. ✦ frontend    Implementa LanguageSwitcher en header.tsx          ▰▰▰▱ 71%   412.0k     37       9 min
    6.   Explore     Busca dónde se lee el locale                              —    51.2k      9   6 min ago
    7.   general-p…  Comprueba los textos de i18n                              —    32.4k      4   7 min ago
 ↑↓ move · enter details · tab mode · / filter · q quit                                                v1.2.0
```

## Qué cambia

- **Corriendo:** un run por línea, nunca fusionados (tres `docs` en paralelo
  son tres filas, cada una con su tarea).
- **Terminados:** se siguen fusionando por agente con `×N`; la tarea es la del
  último run.
- **Columna `task`** nueva entre el nombre y `ctx`; es la que cede primero en
  pantallas estrechas (cortada con `…`).
- **Cursor:** la fila seleccionada abre debajo 1–3 líneas (`now`, `skill` si
  hay, `cwd`) con una guía `│` en gris. Las demás, una línea.
- Sin línea en blanco entre filas; solo antes de cada regla.

## Pros / contras

- ✅ El cambio más pequeño: misma estructura, mismas columnas, mismos grupos.
- ✅ Se conserva `now` para lo que tienes seleccionado.
- ✅ Los terminados no inflan la lista (`×N`).
- ❌ No enseña quién lanzó qué (sesión / run padre).
- ❌ El desplegable del cursor hace que la lista «salte» al moverte.
