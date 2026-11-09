//! Poner el binario nuevo en el sitio del que está en marcha. Se escribe en un
//! temporal junto al destino y un `rename` lo cambia de golpe: una descarga
//! que muere a medias deja el binario viejo intacto.

use std::io;
use std::path::{Path, PathBuf};

use super::UpdateError;

/// De dónde salió el binario en marcha. Solo uno de release es nuestro para
/// sustituir: los otros dos son de cargo o de la carpeta de compilación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// Lo instaló `install.sh` o alguien a mano.
    Release,
    /// `cargo install`: es de cargo y lo actualiza `cargo install`.
    Cargo,
    /// `target/debug` o `target/release`: una compilación, no una instalación.
    Dev,
}

impl InstallKind {
    /// Por qué la actualización se para por el sitio donde vive el binario, y
    /// qué lo actualiza en su lugar.
    pub fn refusal(self) -> Option<Refusal> {
        match self {
            InstallKind::Release => None,
            InstallKind::Cargo => Some(Refusal {
                why: "this claude-agents came from cargo",
                fix: "cargo install --git https://github.com/towerforge/claude-agents",
            }),
            InstallKind::Dev => Some(Refusal {
                why: "this claude-agents is a local build under target/",
                fix: "make build",
            }),
        }
    }
}

/// Una actualización que se para antes de descargar nada: el motivo y el
/// comando que actualiza este claude-agents en su lugar. En dos piezas para
/// que el panel dé al comando una línea propia; como texto, una frase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    pub why: &'static str,
    pub fix: &'static str,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: update it with `{}` (or pass --force to overwrite it anyway)",
            self.why, self.fix
        )
    }
}

/// El binario en marcha, con los enlaces resueltos para que la actualización
/// caiga en el fichero de verdad.
pub fn current_exe() -> Result<PathBuf, UpdateError> {
    let exe = std::env::current_exe().map_err(|source| UpdateError::Io {
        path: PathBuf::from("<current exe>"),
        source,
    })?;
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

/// Por componentes enteros: `/opt/target/bin` no es una carpeta de compilación.
pub fn classify(exe: &Path) -> InstallKind {
    let parts: Vec<_> = exe.iter().map(|p| p.to_string_lossy()).collect();
    let after_target = parts
        .windows(2)
        .any(|w| w[0] == "target" && (w[1] == "debug" || w[1] == "release"));
    // también target/<triple>/release, lo que deja `make build CARGO_TARGET=…`
    let after_triple = parts
        .windows(3)
        .any(|w| w[0] == "target" && (w[2] == "debug" || w[2] == "release"));
    if after_target || after_triple {
        return InstallKind::Dev;
    }
    let cargo_bin = parts
        .windows(2)
        .any(|w| (w[0] == ".cargo" || w[0] == "cargo") && w[1] == "bin");
    if cargo_bin {
        return InstallKind::Cargo;
    }
    InstallKind::Release
}

/// Si el destino se puede sustituir, preguntado antes de descargar nada: el
/// cambio necesita escribir en la carpeta, no en el fichero.
pub fn check_writable(dest: &Path) -> Result<(), UpdateError> {
    let dir = dest.parent().unwrap_or(Path::new("."));
    let probe = dir.join(format!(
        ".claude-agents-update-probe-{}",
        std::process::id()
    ));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            Err(UpdateError::Denied(dest.to_path_buf()))
        }
        Err(source) => Err(UpdateError::Io {
            path: dir.to_path_buf(),
            source,
        }),
    }
}

/// Escribe `bytes` sobre `dest`. El viejo sigue ahí hasta que el nuevo está
/// en su sitio; en Unix un `rename` sobre el binario en marcha es seguro (el
/// proceso conserva el inodo viejo).
pub fn install_binary(bytes: &[u8], dest: &Path) -> Result<(), UpdateError> {
    let dir = dest.parent().unwrap_or(Path::new("."));
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| super::APP.to_string());
    let tmp = dir.join(format!(".{name}.new-{}", std::process::id()));
    let written = write_executable(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, dest).map_err(|source| io_err(dest, source)));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, bytes).map_err(|source| io_err(path, source))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|source| io_err(path, source))
}

fn io_err(path: &Path, source: io::Error) -> UpdateError {
    if source.kind() == io::ErrorKind::PermissionDenied {
        UpdateError::Denied(path.to_path_buf())
    } else {
        UpdateError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_dir;
    use super::*;

    #[test]
    fn it_tells_where_the_binary_came_from() {
        let kind = |p: &str| classify(Path::new(p));
        assert_eq!(kind("/usr/local/bin/claude-agents"), InstallKind::Release);
        assert_eq!(
            kind("/Users/j/.local/bin/claude-agents"),
            InstallKind::Release
        );
        assert_eq!(
            kind("/Users/j/.cargo/bin/claude-agents"),
            InstallKind::Cargo
        );
        assert_eq!(
            kind("/Users/j/claude-agents/target/release/claude-agents"),
            InstallKind::Dev
        );
        assert_eq!(
            kind("/Users/j/claude-agents/target/debug/claude-agents"),
            InstallKind::Dev
        );
        assert_eq!(
            kind("/Users/j/claude-agents/target/aarch64-apple-darwin/release/claude-agents"),
            InstallKind::Dev
        );
        // una carpeta que se llama target pero no es de compilación
        assert_eq!(kind("/opt/target/bin/claude-agents"), InstallKind::Release);
        assert_eq!(
            kind("/opt/my-target/release/claude-agents"),
            InstallKind::Release
        );
    }

    #[test]
    fn the_refusal_names_the_command_that_updates_it_instead() {
        assert_eq!(InstallKind::Release.refusal(), None);
        let cargo = InstallKind::Cargo.refusal().unwrap();
        assert_eq!(
            cargo.to_string(),
            "this claude-agents came from cargo: update it with `cargo install --git \
             https://github.com/towerforge/claude-agents` (or pass --force to overwrite it anyway)"
        );
        assert_eq!(InstallKind::Dev.refusal().unwrap().fix, "make build");
    }

    #[test]
    fn the_swap_leaves_the_new_binary_executable_and_nothing_else() {
        use std::os::unix::fs::PermissionsExt;
        let dir = test_dir("swap");
        let dest = dir.join("claude-agents");
        std::fs::write(&dest, b"old").unwrap();
        check_writable(&dest).unwrap();
        install_binary(b"new", &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "tiene que seguir siendo ejecutable");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "claude-agents")
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(leftovers.is_empty(), "restos: {leftovers:?}");
    }

    #[test]
    fn it_installs_where_there_is_nothing_yet() {
        let dir = test_dir("fresh");
        let dest = dir.join("claude-agents");
        install_binary(b"new", &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_read_only_directory_is_denied_with_the_way_out() {
        use std::os::unix::fs::PermissionsExt;
        let dir = test_dir("readonly");
        let dest = dir.join("claude-agents");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let probe = check_writable(&dest);
        let swap = install_binary(b"new", &dest);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        // como root no hay permiso que valga: entonces no hay nada que probar
        if probe.is_ok() {
            return;
        }
        let msg = probe.unwrap_err().to_string();
        assert!(msg.contains("sudo"), "{msg}");
        assert!(msg.contains("CLAUDE_AGENTS_INSTALL_DIR"), "{msg}");
        assert!(matches!(swap, Err(UpdateError::Denied(_))));
    }

    #[test]
    fn a_directory_that_does_not_exist_is_not_writable() {
        assert!(check_writable(Path::new("/no/such/dir/claude-agents")).is_err());
    }
}
