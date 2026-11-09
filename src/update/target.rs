//! La plataforma para la que se compiló el binario en marcha y el asset de la
//! release que le toca. Los nombres son los que escribe `make package` y baja
//! `install.sh`: `claude-agents-<os>-<arch>[-musl].tar.gz`. Sin Windows: las
//! sesiones se leen con `ps`.

use std::fmt;

use super::UpdateError;

pub const APP: &str = "claude-agents";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub os: &'static str,
    pub arch: &'static str,
    /// Enlazado estático contra musl. Solo puede ser cierto en Linux.
    pub musl: bool,
}

impl Target {
    pub const fn new(os: &'static str, arch: &'static str, musl: bool) -> Self {
        Self { os, arch, musl }
    }

    /// Para qué se compiló este binario. La libc sale de la compilación, no
    /// de la máquina: un binario musl sigue en musl.
    pub fn current() -> Result<Self, UpdateError> {
        Self::from_consts(
            std::env::consts::OS,
            std::env::consts::ARCH,
            cfg!(target_env = "musl"),
        )
    }

    fn from_consts(os: &str, arch: &str, musl: bool) -> Result<Self, UpdateError> {
        let os = match os {
            "macos" => "macos",
            "linux" => "linux",
            other => return Err(UpdateError::Unsupported(other.to_string())),
        };
        let arch = match arch {
            "x86_64" => "x86_64",
            "aarch64" => "aarch64",
            other => return Err(UpdateError::Unsupported(format!("{os} {other}"))),
        };
        Ok(Self::new(os, arch, musl && os == "linux"))
    }

    /// Nombre del asset de la release para esta plataforma.
    pub fn asset_name(&self) -> String {
        let musl = if self.musl && self.os == "linux" {
            "-musl"
        } else {
            ""
        };
        format!("{APP}-{}-{}{musl}.tar.gz", self.os, self.arch)
    }

    /// Nombre del binario dentro de ese asset.
    pub fn binary_name(&self) -> &'static str {
        APP
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} · {}", self.os, self.arch)?;
        if self.musl && self.os == "linux" {
            write!(f, " · musl")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_are_the_ones_the_releases_use() {
        assert_eq!(
            Target::new("macos", "aarch64", false).asset_name(),
            "claude-agents-macos-aarch64.tar.gz"
        );
        assert_eq!(
            Target::new("macos", "x86_64", false).asset_name(),
            "claude-agents-macos-x86_64.tar.gz"
        );
        assert_eq!(
            Target::new("linux", "x86_64", false).asset_name(),
            "claude-agents-linux-x86_64.tar.gz"
        );
        assert_eq!(
            Target::new("linux", "aarch64", true).asset_name(),
            "claude-agents-linux-aarch64-musl.tar.gz"
        );
        // musl es cosa de Linux: no toca los demás nombres
        assert_eq!(
            Target::new("macos", "aarch64", true).asset_name(),
            "claude-agents-macos-aarch64.tar.gz"
        );
        assert_eq!(
            Target::new("linux", "x86_64", false).binary_name(),
            "claude-agents"
        );
    }

    #[test]
    fn only_the_platforms_with_a_release_are_supported() {
        assert_eq!(
            Target::from_consts("linux", "aarch64", true).unwrap(),
            Target::new("linux", "aarch64", true)
        );
        assert!(!Target::from_consts("macos", "aarch64", true).unwrap().musl);
        assert!(matches!(
            Target::from_consts("windows", "x86_64", false),
            Err(UpdateError::Unsupported(_))
        ));
        assert!(matches!(
            Target::from_consts("linux", "riscv64", false),
            Err(UpdateError::Unsupported(_))
        ));
    }

    #[test]
    fn this_platform_is_supported() {
        let t = Target::current().expect("claude-agents se compila para esta plataforma");
        assert!(t.asset_name().starts_with("claude-agents-"));
    }

    #[test]
    fn it_prints_os_arch_and_libc() {
        assert_eq!(
            Target::new("linux", "x86_64", true).to_string(),
            "linux · x86_64 · musl"
        );
        assert_eq!(
            Target::new("macos", "aarch64", false).to_string(),
            "macos · aarch64"
        );
    }
}
