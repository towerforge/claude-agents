//! `claude-agents update`: qué publicó GitHub, si es más nuevo que lo que está
//! en marcha y la descarga que sustituye el binario. La misma lógica que
//! `moon-updater`, en síncrono (ureq, sin tokio). Aquí no hay terminal: el
//! flujo está en `flow` y lo que se pinta, aparte (`plain`).

pub mod cache;
pub mod flow;
pub mod install;
pub mod plain;
pub mod target;
pub mod version;

use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub use install::Refusal;
pub use target::{Target, APP};
pub use version::Version;

/// Dónde se publican las releases: el mismo repositorio que lee `install.sh`.
pub const REPO: &str = "towerforge/claude-agents";
const API: &str = "https://api.github.com/repos";
const CHECKSUMS: &str = "checksums.txt";
const UA: &str = concat!("claude-agents/", env!("CARGO_PKG_VERSION"));
/// Suficiente para las llamadas JSON y `checksums.txt`.
const API_TIMEOUT: Duration = Duration::from_secs(15);
/// La descarga del archivo (unos MB): sin prisa, pero sin colgarse para siempre.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub enum UpdateError {
    Unsupported(String),
    Http(String),
    NoRelease,
    NoTag(String),
    NoAsset {
        version: String,
        asset: String,
    },
    BadVersion(String),
    Checksum {
        expected: String,
        got: String,
    },
    Archive(String),
    Denied(PathBuf),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::Unsupported(p) => write!(f, "no prebuilt binary for {p}"),
            UpdateError::Http(m) => write!(f, "github: {m}"),
            UpdateError::NoRelease => write!(
                f,
                "no release published yet at https://github.com/{REPO}/releases"
            ),
            UpdateError::NoTag(t) => write!(f, "there is no release {t} to install"),
            UpdateError::NoAsset { version, asset } => {
                write!(f, "release v{version} has no {asset}")
            }
            UpdateError::BadVersion(v) => write!(f, "`{v}` is not a version like 1.2.3"),
            UpdateError::Checksum { expected, got } => {
                write!(f, "checksum mismatch: expected {expected}, got {got}")
            }
            UpdateError::Archive(a) => write!(f, "{a} does not contain the {APP} binary"),
            UpdateError::Denied(p) => write!(
                f,
                "cannot write to {}: run it again with sudo, or set \
                 CLAUDE_AGENTS_INSTALL_DIR and use install.sh",
                p.display()
            ),
            UpdateError::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for UpdateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            UpdateError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<ureq::Error> for UpdateError {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::StatusCode(404) => UpdateError::NoRelease,
            ureq::Error::StatusCode(403 | 429) => {
                UpdateError::Http("rate limit reached, try again in a while".into())
            }
            ureq::Error::StatusCode(s) => UpdateError::Http(format!("HTTP {s}")),
            e @ (ureq::Error::Timeout(_)
            | ureq::Error::HostNotFound
            | ureq::Error::ConnectionFailed
            | ureq::Error::Io(_)) => UpdateError::Http(format!("{e} (no connection?)")),
            e => UpdateError::Http(e.to_string()),
        }
    }
}

/// Una release publicada, con lo que la vista enseña de ella.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub notes: String,
    pub url: String,
    pub published_at: Option<DateTime<Utc>>,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

impl Release {
    /// La respuesta de `releases/latest` o `releases/tags/<tag>`.
    pub fn from_json(text: &str) -> Result<Self, UpdateError> {
        let bad = |what: &str| UpdateError::Http(format!("unexpected answer: {what}"));
        let v: Value = serde_json::from_str(text).map_err(|e| bad(&e.to_string()))?;
        let tag = v
            .get("tag_name")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("no tag_name"))?
            .to_string();
        let version = tag
            .parse()
            .map_err(|_| UpdateError::BadVersion(tag.clone()))?;
        let text_of = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default();
        let assets = v
            .get("assets")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|a| {
                        Some(Asset {
                            name: a.get("name")?.as_str()?.to_string(),
                            url: a.get("browser_download_url")?.as_str()?.to_string(),
                            size: a.get("size").and_then(Value::as_u64).unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Release {
            version,
            notes: text_of("body").to_string(),
            url: text_of("html_url").to_string(),
            published_at: text_of("published_at").parse().ok(),
            tag,
            assets,
        })
    }

    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }

    /// El asset que necesita esta plataforma.
    pub fn asset_for(&self, target: &Target) -> Result<&Asset, UpdateError> {
        let name = target.asset_name();
        self.asset(&name).ok_or_else(|| UpdateError::NoAsset {
            version: self.version.to_string(),
            asset: name,
        })
    }

    /// `checksums.txt`, publicado junto a los archivos.
    pub fn checksums_url(&self) -> Option<&str> {
        self.asset(CHECKSUMS).map(|a| a.url.as_str())
    }

    /// Las primeras líneas de las notas, como viñetas: lo que la vista enseña
    /// bajo «What's new». GitHub escribe `* título by @quién in <url>`.
    pub fn highlights(&self, max: usize) -> Vec<String> {
        self.notes
            .lines()
            .map(str::trim)
            .filter_map(|l| l.strip_prefix("* ").or_else(|| l.strip_prefix("- ")))
            .map(|l| match l.find(" by @") {
                Some(i) => l[..i].trim(),
                None => l,
            })
            .filter(|l| !l.is_empty() && !l.starts_with("**Full Changelog**"))
            .map(str::to_string)
            .take(max)
            .collect()
    }
}

/// De dónde salen las releases y los archivos. `Client` habla con GitHub; los
/// tests ponen uno de mentira para no tocar la red.
pub trait Source {
    /// La última release publicada.
    fn latest(&self) -> Result<Release, UpdateError>;
    /// Una versión concreta, por su etiqueta.
    fn release(&self, v: &Version) -> Result<Release, UpdateError>;
    /// El texto de un asset (`checksums.txt`).
    fn text(&self, url: &str) -> Result<String, UpdateError>;
    /// Un asset entero en memoria, avisando de los bytes que lleva y del total.
    fn download(
        &self,
        url: &str,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>, UpdateError>;
}

/// La API de releases de GitHub, anónima: 60 llamadas por hora y dirección,
/// muy lejos de lo que gasta la nota diaria.
pub struct Client {
    http: ureq::Agent,
    repo: String,
}

impl Client {
    pub fn new() -> Self {
        let http = ureq::Agent::config_builder()
            .user_agent(UA)
            .timeout_connect(Some(Duration::from_secs(5)))
            .build()
            .into();
        Self {
            http,
            repo: REPO.to_string(),
        }
    }

    fn release_at(&self, url: &str) -> Result<Release, UpdateError> {
        let text = self
            .http
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .config()
            .timeout_global(Some(API_TIMEOUT))
            .build()
            .call()?
            .body_mut()
            .read_to_string()?;
        Release::from_json(&text)
    }
}

impl Source for Client {
    fn latest(&self) -> Result<Release, UpdateError> {
        self.release_at(&format!("{API}/{}/releases/latest", self.repo))
    }

    fn release(&self, v: &Version) -> Result<Release, UpdateError> {
        self.release_at(&format!("{API}/{}/releases/tags/{}", self.repo, v.tag()))
            .map_err(|e| match e {
                UpdateError::NoRelease => UpdateError::NoTag(v.tag()),
                other => other,
            })
    }

    fn text(&self, url: &str) -> Result<String, UpdateError> {
        Ok(self
            .http
            .get(url)
            .config()
            .timeout_global(Some(API_TIMEOUT))
            .build()
            .call()?
            .body_mut()
            .read_to_string()?)
    }

    fn download(
        &self,
        url: &str,
        on_progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Vec<u8>, UpdateError> {
        let mut resp = self
            .http
            .get(url)
            .config()
            .timeout_global(Some(DOWNLOAD_TIMEOUT))
            .build()
            .call()?;
        let total = resp.body().content_length();
        let mut out = Vec::with_capacity(total.unwrap_or(1 << 20) as usize);
        let mut reader = resp.body_mut().as_reader();
        let mut buf = [0u8; 64 * 1024];
        on_progress(0, total);
        loop {
            let n = reader.read(&mut buf).map_err(|source| UpdateError::Io {
                path: PathBuf::from(url),
                source,
            })?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            on_progress(out.len() as u64, total);
        }
        Ok(out)
    }
}

/// SHA-256 del archivo contra su línea de `checksums.txt`. Que no haya línea
/// para este asset no es un error: `install.sh` avisa y sigue, y esto también
/// (devuelve `false`, «sin verificar»).
pub fn verify_sha256(bytes: &[u8], checksums: &str, asset: &str) -> Result<bool, UpdateError> {
    let Some(expected) = checksums
        .lines()
        .filter_map(|l| l.split_once("  ").or_else(|| l.split_once(' ')))
        .find(|(_, name)| name.trim().trim_start_matches('*') == asset)
        .map(|(hash, _)| hash.trim().to_lowercase())
    else {
        return Ok(false);
    };
    let got = format!("{:x}", Sha256::digest(bytes));
    if got == expected {
        Ok(true)
    } else {
        Err(UpdateError::Checksum { expected, got })
    }
}

/// Saca el binario de claude-agents del `.tar.gz` descargado.
pub fn unpack(bytes: &[u8], target: &Target) -> Result<Vec<u8>, UpdateError> {
    let name = target.binary_name();
    let io = |source| UpdateError::Io {
        path: PathBuf::from(target.asset_name()),
        source,
    };
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    for entry in archive.entries().map_err(io)? {
        let mut entry = entry.map_err(io)?;
        let path = entry.path().map_err(io)?;
        if path.file_name().is_some_and(|f| f == name) {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(io)?;
            if !buf.is_empty() {
                return Ok(buf);
            }
        }
    }
    Err(UpdateError::Archive(target.asset_name()))
}

/// Carpeta temporal propia de un test (nunca `~/.cache`). La borra el test.
#[cfg(test)]
pub(crate) fn test_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "claude-agents-update-{}-{name}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Un `.tar.gz` con los ficheros dados, como el que publica `make package`.
#[cfg(test)]
pub(crate) fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (name, data) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, name, *data).unwrap();
    }
    let raw = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut gz, &raw).unwrap();
    gz.finish().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(notes: &str) -> Release {
        Release {
            version: "1.1.0".parse().unwrap(),
            tag: "v1.1.0".into(),
            notes: notes.into(),
            url: String::new(),
            published_at: None,
            assets: vec![
                Asset {
                    name: "claude-agents-macos-aarch64.tar.gz".into(),
                    url: "https://example/claude-agents-macos-aarch64.tar.gz".into(),
                    size: 10,
                },
                Asset {
                    name: CHECKSUMS.into(),
                    url: "https://example/checksums.txt".into(),
                    size: 1,
                },
            ],
        }
    }

    #[test]
    fn the_github_answer_becomes_a_release() {
        // recorte literal de releases/latest de towerforge/claude-agents
        let json = r###"{
            "html_url": "https://github.com/towerforge/claude-agents/releases/tag/v1.1.0",
            "tag_name": "v1.1.0",
            "published_at": "2026-11-08T18:02:11Z",
            "body": "## What's Changed\n* feat(tui): click by @towerforge in https://x/pull/2\n",
            "assets": [
                {"name": "checksums.txt", "size": 600,
                 "browser_download_url": "https://github.com/towerforge/claude-agents/releases/download/v1.1.0/checksums.txt"},
                {"name": "claude-agents-linux-x86_64-musl.tar.gz", "size": 2100000,
                 "browser_download_url": "https://github.com/towerforge/claude-agents/releases/download/v1.1.0/claude-agents-linux-x86_64-musl.tar.gz"},
                {"name": "sin-url"}
            ]
        }"###;
        let r = Release::from_json(json).unwrap();
        assert_eq!(r.version, "1.1.0".parse().unwrap());
        assert_eq!(r.tag, "v1.1.0");
        assert_eq!(
            r.published_at,
            Some("2026-11-08T18:02:11Z".parse().unwrap())
        );
        assert_eq!(r.assets.len(), 2, "el asset sin url no cuenta");
        let musl = Target::new("linux", "x86_64", true);
        assert_eq!(r.asset_for(&musl).unwrap().size, 2_100_000);
        assert!(r
            .checksums_url()
            .unwrap()
            .ends_with("/v1.1.0/checksums.txt"));
        assert_eq!(r.highlights(5), vec!["feat(tui): click"]);
    }

    #[test]
    fn a_github_answer_without_a_version_is_an_error() {
        assert!(matches!(
            Release::from_json(r#"{"tag_name":"nightly"}"#),
            Err(UpdateError::BadVersion(t)) if t == "nightly"
        ));
        assert!(matches!(
            Release::from_json(r#"{"message":"Not Found"}"#),
            Err(UpdateError::Http(_))
        ));
        assert!(Release::from_json("<html>").is_err());
    }

    #[test]
    fn the_platform_asset_and_the_missing_one() {
        let r = release("");
        let mac = Target::new("macos", "aarch64", false);
        assert_eq!(r.asset_for(&mac).unwrap().size, 10);
        let linux = Target::new("linux", "x86_64", false);
        let err = r.asset_for(&linux).unwrap_err();
        assert_eq!(
            err.to_string(),
            "release v1.1.0 has no claude-agents-linux-x86_64.tar.gz"
        );
        assert_eq!(r.checksums_url(), Some("https://example/checksums.txt"));
    }

    #[test]
    fn the_notes_are_summarized_into_bullets() {
        let notes = "## What's Changed\n\
             * feat: update command by @towerforge in https://github.com/x/pull/3\n\
             - fix: narrow terminals\n\
             \n\
             **Full Changelog**: https://github.com/x/compare/v1.0.0...v1.1.0\n";
        assert_eq!(
            release(notes).highlights(5),
            vec!["feat: update command", "fix: narrow terminals"]
        );
        assert_eq!(release(notes).highlights(1).len(), 1);
        assert!(release("").highlights(5).is_empty());
    }

    #[test]
    fn the_checksum_matches_or_nothing_installs() {
        let data = b"claude-agents";
        let hash = format!("{:x}", Sha256::digest(data));
        let file = format!(
            "{hash}  claude-agents-macos-aarch64.tar.gz\nother  claude-agents-linux-x86_64.tar.gz\n"
        );
        assert!(verify_sha256(data, &file, "claude-agents-macos-aarch64.tar.gz").unwrap());
        // sin línea para este asset: nada con qué comparar, y no es un error
        assert!(!verify_sha256(data, &file, "claude-agents-linux-aarch64.tar.gz").unwrap());
        let wrong = format!("{}  claude-agents-macos-aarch64.tar.gz\n", "0".repeat(64));
        assert!(matches!(
            verify_sha256(data, &wrong, "claude-agents-macos-aarch64.tar.gz"),
            Err(UpdateError::Checksum { .. })
        ));
    }

    #[test]
    fn it_pulls_the_binary_out_of_the_tar_gz() {
        let archive = tar_gz(&[("README", b"hey"), ("claude-agents", b"MACH")]);
        let t = Target::new("macos", "aarch64", false);
        assert_eq!(unpack(&archive, &t).unwrap(), b"MACH");
    }

    #[test]
    fn an_archive_with_no_binary_does_not_pass() {
        let t = Target::new("linux", "x86_64", false);
        let archive = tar_gz(&[("README", b"hey")]);
        assert!(matches!(unpack(&archive, &t), Err(UpdateError::Archive(_))));
        let empty = tar_gz(&[("claude-agents", b"")]);
        assert!(matches!(unpack(&empty, &t), Err(UpdateError::Archive(_))));
        assert!(unpack(b"not gzip", &t).is_err());
    }

    #[test]
    fn http_failures_say_what_happened() {
        assert!(matches!(
            UpdateError::from(ureq::Error::StatusCode(404)),
            UpdateError::NoRelease
        ));
        let limit = UpdateError::from(ureq::Error::StatusCode(403)).to_string();
        assert!(limit.contains("rate limit"), "{limit}");
        let down = UpdateError::from(ureq::Error::HostNotFound).to_string();
        assert!(down.contains("no connection?"), "{down}");
        assert_eq!(
            UpdateError::from(ureq::Error::StatusCode(502)).to_string(),
            "github: HTTP 502"
        );
    }
}
