//! Definiciones de agentes: ficheros `.claude/agents/*.md` y su nombre.

use std::path::{Path, PathBuf};

/// Dónde vive el agente: global (`~/.claude/agents`) o dentro de un proyecto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Global,
    Project(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub name: String,
    pub path: PathBuf,
    pub scope: Scope,
    /// Campo `description:` del frontmatter, si lo hay.
    pub description: Option<String>,
}

use anyhow::{Context, Result};
use walkdir::{DirEntry, WalkDir};

/// Directorios que nunca contienen agentes y cuestan mucho de recorrer.
const SKIP: &[&str] = &[
    "node_modules",
    "target",
    ".venv",
    "venv",
    "Library",
    "vendor",
];

/// Decide si el recorrido entra en un directorio. Dentro de un `.claude` solo
/// interesa `agents`: así se evitan `projects`, `plugins` y `worktrees`.
pub fn should_descend(path: &Path, name: &str) -> bool {
    let parent_is_claude = path
        .parent()
        .and_then(Path::file_name)
        .map(|p| p == ".claude")
        .unwrap_or(false);
    if parent_is_claude {
        return name == "agents";
    }
    if SKIP.contains(&name) {
        return false;
    }
    name == ".claude" || !name.starts_with('.')
}

/// Valor de un campo (`name:`, `description:`) del frontmatter, si lo hay.
pub fn frontmatter_field(content: &str, key: &str) -> Option<String> {
    let mut lines = content.lines().map(str::trim_end);
    if lines.next()? != "---" {
        return None;
    }
    lines
        .take_while(|l| *l != "---")
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Ámbito a partir de la ruta `<X>/.claude/agents/<fichero>.md`.
pub fn scope_of(path: &Path, home: &Path) -> Scope {
    match path.ancestors().nth(3) {
        Some(root) if root != home => Scope::Project(root.to_path_buf()),
        _ => Scope::Global,
    }
}

fn is_agent_file(entry: &DirEntry) -> bool {
    let p = entry.path();
    entry.file_type().is_file()
        && p.extension().is_some_and(|e| e == "md")
        && p.parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "agents")
        && p.ancestors()
            .nth(2)
            .and_then(Path::file_name)
            .is_some_and(|n| n == ".claude")
}

/// Recorre `roots` y devuelve todas las definiciones encontradas, sin duplicados.
pub fn find_agents(roots: &[PathBuf], home: &Path) -> Result<Vec<Agent>> {
    let mut found = Vec::new();
    for root in roots {
        let walker = WalkDir::new(root).into_iter().filter_entry(|e| {
            e.depth() == 0
                || !e.file_type().is_dir()
                || should_descend(e.path(), &e.file_name().to_string_lossy())
        });
        for entry in walker.filter_map(|e| e.ok()).filter(is_agent_file) {
            let path = entry.into_path();
            if found.iter().any(|a: &Agent| a.path == path) {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            let name = frontmatter_field(&content, "name").unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
            let description = frontmatter_field(&content, "description");
            let scope = scope_of(&path, home);
            found.push(Agent {
                name,
                path,
                scope,
                description,
            });
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_comes_from_frontmatter() {
        let md = "---\nname: revisor-codigo\ndescription: Revisa cambios\n---\nInstrucciones...\n";
        assert_eq!(
            frontmatter_field(md, "name"),
            Some("revisor-codigo".to_string())
        );
    }

    #[test]
    fn name_is_trimmed_and_tolerates_crlf() {
        let md = "---\r\nname:   con-espacios  \r\n---\r\n";
        assert_eq!(
            frontmatter_field(md, "name"),
            Some("con-espacios".to_string())
        );
    }

    #[test]
    fn name_outside_frontmatter_is_ignored() {
        let md = "---\ndescription: sin nombre\n---\nname: en-el-cuerpo\n";
        assert_eq!(frontmatter_field(md, "name"), None);
    }

    #[test]
    fn description_is_read_too() {
        let md = "---\nname: a\ndescription: Revisa cambios y señala problemas\n---\n";
        assert_eq!(
            frontmatter_field(md, "description"),
            Some("Revisa cambios y señala problemas".to_string())
        );
    }

    #[test]
    fn key_must_match_the_whole_word() {
        let md = "---\nnamespace: x\nname: real\n---\n";
        assert_eq!(frontmatter_field(md, "name"), Some("real".to_string()));
    }

    #[test]
    fn file_without_frontmatter_has_no_name() {
        assert_eq!(frontmatter_field("name: suelto\n", "name"), None);
    }

    #[test]
    fn scope_is_global_under_home_claude_dir() {
        let home = Path::new("/Users/x");
        let path = Path::new("/Users/x/.claude/agents/a.md");
        assert_eq!(scope_of(path, home), Scope::Global);
    }

    #[test]
    fn scope_is_project_dir_for_project_agents() {
        let home = Path::new("/Users/x");
        let path = Path::new("/Users/x/Offing/app/.claude/agents/a.md");
        assert_eq!(
            scope_of(path, home),
            Scope::Project(PathBuf::from("/Users/x/Offing/app"))
        );
    }

    #[test]
    fn only_descends_into_agents_inside_claude_dir() {
        assert!(should_descend(Path::new("/p/.claude"), ".claude"));
        assert!(should_descend(Path::new("/p/.claude/agents"), "agents"));
        assert!(!should_descend(
            Path::new("/p/.claude/worktrees"),
            "worktrees"
        ));
        assert!(!should_descend(Path::new("/p/.claude/plugins"), "plugins"));
    }

    #[test]
    fn skips_hidden_and_dependency_dirs() {
        assert!(!should_descend(Path::new("/p/.git"), ".git"));
        assert!(!should_descend(
            Path::new("/p/node_modules"),
            "node_modules"
        ));
        assert!(!should_descend(Path::new("/p/target"), "target"));
        assert!(!should_descend(Path::new("/p/.venv"), ".venv"));
        assert!(!should_descend(Path::new("/Users/x/Library"), "Library"));
        assert!(should_descend(Path::new("/p/src"), "src"));
    }
}
