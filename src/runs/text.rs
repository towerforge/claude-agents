//! Textos que se sacan de las líneas de una ejecución: el encargo y la
//! última herramienta (nombre y argumento corto).

use serde_json::Value;
use std::path::Path;

/// Quita las `#` y el boilerplate `ENCARGO (de …):` (o `TASK (from …):`) del principio de una
/// línea, sin tocar el resto.
fn strip_boilerplate(line: &str) -> &str {
    let text = line.trim().trim_start_matches('#').trim_start();
    match text
        .strip_prefix("ENCARGO (")
        .or_else(|| text.strip_prefix("TASK ("))
        .and_then(|r| r.split_once(')'))
    {
        Some((_, rest)) => rest.trim_start_matches(':'),
        None => text,
    }
}

/// Como `strip_boilerplate`, juntando los espacios. Vacío si no queda nada.
fn clean_task_line(line: &str) -> String {
    strip_boilerplate(line)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Máximo de caracteres del encargo completo; lo que sobra se corta con `…`.
pub(super) const TASK_FULL_MAX: usize = 2000;

/// Los textos de un mensaje de usuario: `content` es un texto o una lista de
/// bloques, de los que cuentan los `text` (un `tool_result` no es un prompt).
fn prompt_blocks(doc: &Value) -> Option<Vec<&str>> {
    if doc.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    match doc.pointer("/message/content")? {
        Value::String(s) => Some(vec![s.as_str()]),
        Value::Array(blocks) => Some(
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect(),
        ),
        _ => None,
    }
}

/// El texto de un mensaje de usuario, con sus bloques unidos por saltos de línea.
fn prompt_text(doc: &Value) -> Option<String> {
    prompt_blocks(doc).map(|b| b.join("\n"))
}

/// El prompt de una línea de la transcripción de una sesión, en una línea: un
/// mensaje de usuario que no sea `isMeta` (el cuerpo de una skill) ni el
/// resumen de una compactación, sin los bloques que empiezan por `<` (avisos
/// que Claude Code mete: `<command-name>`, `<ide_opened_file>`,
/// `<system-reminder>`…). `None` si no queda texto.
pub(super) fn session_prompt_of(doc: &Value) -> Option<String> {
    let flag = |k: &str| doc.get(k).and_then(Value::as_bool) == Some(true);
    if flag("isMeta") || flag("isCompactSummary") {
        return None;
    }
    let text = prompt_blocks(doc)?
        .into_iter()
        .filter(|t| !t.trim_start().starts_with('<'))
        .collect::<Vec<_>>()
        .join("\n");
    first_task_line(&text)
}

/// Un comando de barra como prompt (`/review 123`), sacado de las etiquetas
/// `<command-name>` y `<command-args>`: lo que se usa cuando la sesión no tiene
/// otro prompt.
pub(super) fn command_of(doc: &Value) -> Option<String> {
    let text = prompt_text(doc)?;
    let tag = |name: &str| {
        let open = format!("<{name}>");
        let start = text.find(&open)? + open.len();
        let end = text[start..].find(&format!("</{name}>"))? + start;
        Some(text[start..end].trim().to_string())
    };
    let name = tag("command-name").filter(|n| !n.is_empty())?;
    let line = match tag("command-args").filter(|a| !a.is_empty()) {
        Some(args) => format!("{name} {args}"),
        None => name,
    };
    first_task_line(&line)
}

/// El encargo de un mensaje de usuario en una línea.
pub(super) fn task_of(doc: &Value) -> Option<String> {
    first_task_line(&prompt_text(doc)?)
}

/// La primera línea con contenido de un prompt, limpia y en una línea.
pub fn first_task_line(text: &str) -> Option<String> {
    text.lines().map(clean_task_line).find(|l| !l.is_empty())
}

/// El encargo completo: el texto del mensaje con sus saltos de línea, sin la
/// cabecera `# ENCARGO (de …):` y recortado a `TASK_FULL_MAX` caracteres.
pub(super) fn task_full_of(doc: &Value) -> Option<String> {
    let text = prompt_text(doc)?;
    let mut lines = text.lines().skip_while(|l| l.trim().is_empty());
    let first = strip_boilerplate(lines.next()?);
    let mut full = first.trim().to_string();
    for l in lines {
        full.push('\n');
        full.push_str(l.trim_end());
    }
    let full = full.trim();
    if full.is_empty() {
        return None;
    }
    Some(match full.char_indices().nth(TASK_FULL_MAX) {
        Some((cut, _)) => format!("{}…", &full[..cut]),
        None => full.to_string(),
    })
}

/// Nombre y argumento corto de un bloque `tool_use`. Los ficheros van
/// relativos a `cwd` si cuelgan de él y, si no, solo su último componente.
pub(super) fn tool_parts(block: &Value, cwd: Option<&str>) -> Option<(String, Option<String>)> {
    let name = block.get("name").and_then(Value::as_str)?;
    let input = block.get("input");
    let field = |k: &str| input.and_then(|i| i.get(k)).and_then(Value::as_str);
    let arg = match name {
        "Bash" => field("command").map(|c| c.split_whitespace().collect::<Vec<_>>().join(" ")),
        "Read" | "Edit" | "Write" => field("file_path").map(|p| short_path(p, cwd)),
        "NotebookEdit" => field("notebook_path").map(|p| short_path(p, cwd)),
        "Grep" | "Glob" => field("pattern").map(str::to_string),
        _ => None,
    };
    Some((name.to_string(), arg.filter(|a| !a.is_empty())))
}

fn short_path(path: &str, cwd: Option<&str>) -> String {
    let p = Path::new(path);
    cwd.and_then(|c| p.strip_prefix(c).ok())
        .or_else(|| p.file_name().map(Path::new))
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(content: &str) -> Value {
        serde_json::json!({"type":"user","message":{"role":"user","content":content}})
    }

    #[test]
    fn task_full_keeps_line_breaks_and_drops_the_boilerplate() {
        let doc = user("\n# ENCARGO (de Jordi, 05-11-2026): copia de seguridad\n\nDetalle  largo\n  de varias líneas\n");
        assert_eq!(
            task_full_of(&doc).as_deref(),
            Some("copia de seguridad\n\nDetalle  largo\n  de varias líneas")
        );
        // si la cabecera va sola, desaparece la línea entera
        let doc = user("# ENCARGO (de Jordi):\n\nHaz esto\nY esto");
        assert_eq!(task_full_of(&doc).as_deref(), Some("Haz esto\nY esto"));
        assert_eq!(task_full_of(&user("  \n ")), None);
        let result = serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}});
        assert_eq!(task_full_of(&result), None);
    }

    #[test]
    fn task_full_also_drops_the_english_boilerplate() {
        let doc = user("# TASK (from Jordi, 11/05/2026): back up everything\n\nMore detail");
        assert_eq!(
            task_full_of(&doc).as_deref(),
            Some("back up everything\n\nMore detail")
        );
    }

    #[test]
    fn task_full_is_cut_at_a_char_boundary_with_an_ellipsis() {
        let long = "ñ".repeat(TASK_FULL_MAX + 50);
        let full = task_full_of(&user(&long)).unwrap();
        assert_eq!(full.chars().count(), TASK_FULL_MAX + 1);
        assert!(full.ends_with('…'));
        let exact = "a".repeat(TASK_FULL_MAX);
        assert_eq!(task_full_of(&user(&exact)).unwrap(), exact);
    }

    #[test]
    fn tool_parts_split_name_and_short_argument() {
        let block = serde_json::json!({"type":"tool_use","name":"Read","input":{"file_path":"/p/src/a.rs"}});
        assert_eq!(
            tool_parts(&block, Some("/p")),
            Some(("Read".into(), Some("src/a.rs".into())))
        );
        let bare = serde_json::json!({"type":"tool_use","name":"Foo","input":{}});
        assert_eq!(tool_parts(&bare, None), Some(("Foo".into(), None)));
    }

    #[test]
    fn the_session_prompt_skips_meta_lines_and_tagged_blocks() {
        let doc = serde_json::json!({"type":"user","message":{"content":[
            {"type":"text","text":"<browser_instruction># Chrome</browser_instruction>"},
            {"type":"text","text":"quita la funcion de export"}]}});
        assert_eq!(
            session_prompt_of(&doc).as_deref(),
            Some("quita la funcion de export")
        );
        let meta =
            serde_json::json!({"type":"user","isMeta":true,"message":{"content":"Base directory"}});
        assert_eq!(session_prompt_of(&meta), None);
        let cmd = user("<command-message>x</command-message>\n<command-name>/review</command-name>\n<command-args>123</command-args>");
        assert_eq!(session_prompt_of(&cmd), None);
        assert_eq!(command_of(&cmd).as_deref(), Some("/review 123"));
        assert_eq!(command_of(&user("hola")), None);
    }
}
