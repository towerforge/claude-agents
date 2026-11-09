//! Una nota en disco con lo que encontró la última comprobación, para que la
//! pantalla pregunte a GitHub una vez al día y no en cada arranque. Vive en
//! `update-check.json`, junto a `runs.jsonl` (`~/.cache/claude-agents/`).
//! Igual que `moon_updater::cache`, sin serde: dos campos no lo merecen.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::{Client, Source, Version};

pub const FILE: &str = "update-check.json";
/// Lo que vale una comprobación. Una release no es noticia al minuto.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckCache {
    pub checked_at: DateTime<Utc>,
    /// La versión más nueva que tenía GitHub en ese momento.
    pub latest: String,
}

/// La carpeta de la caché de claude-agents: la misma de `runs.jsonl`.
pub fn default_dir() -> Option<PathBuf> {
    crate::runs::history::default_path().and_then(|p| p.parent().map(Path::to_path_buf))
}

impl CheckCache {
    pub fn new(latest: &Version, now: DateTime<Utc>) -> Self {
        Self {
            checked_at: now,
            latest: latest.to_string(),
        }
    }

    pub fn path(dir: &Path) -> PathBuf {
        dir.join(FILE)
    }

    /// Lee la nota. Cualquier cosa rara (no existe, no se lee, otro formato)
    /// es lo mismo que no tener nota.
    pub fn load(dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(Self::path(dir)).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        let checked_at = v.get("checked_at")?.as_str()?.parse().ok()?;
        let latest = v.get("latest")?.as_str()?.to_string();
        Some(Self { checked_at, latest })
    }

    /// Temporal + `rename`: dos pantallas abiertas a la vez no dejan una nota
    /// a medias.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let text = json!({
            "checked_at": self.checked_at.to_rfc3339(),
            "latest": self.latest,
        })
        .to_string();
        let tmp = dir.join(format!(".{FILE}.{}", std::process::id()));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, Self::path(dir)).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// Olvida la comprobación. Se hace justo después de instalar, para que el
    /// aviso no sobreviva a la actualización que lo respondió.
    pub fn forget(dir: &Path) {
        let _ = std::fs::remove_file(Self::path(dir));
    }

    pub fn fresh_at(&self, now: DateTime<Utc>, ttl: Duration) -> bool {
        match (now - self.checked_at).to_std() {
            Ok(age) => age < ttl,
            // comprobada en el futuro: el reloj se movió, no se le hace caso
            Err(_) => false,
        }
    }

    pub fn version(&self) -> Option<Version> {
        self.latest.parse().ok()
    }
}

/// La versión publicada que supera a `current`, si la hay: de la nota
/// mientras esté fresca, de GitHub si no. Cualquier fallo es `None`: nadie se
/// entera de que no se pudo comprobar.
pub fn newer_than(current: &Version, dir: Option<&Path>, ttl: Duration) -> Option<Version> {
    newer_than_with(current, dir, ttl, Utc::now(), || {
        Client::new().latest().ok().map(|r| r.version)
    })
}

/// `newer_than` con el reloj y la pregunta a GitHub inyectados (los tests).
pub fn newer_than_with(
    current: &Version,
    dir: Option<&Path>,
    ttl: Duration,
    now: DateTime<Utc>,
    fetch_latest: impl FnOnce() -> Option<Version>,
) -> Option<Version> {
    if let Some(c) = dir.and_then(CheckCache::load) {
        if c.fresh_at(now, ttl) {
            if let Some(v) = c.version() {
                return (v > *current).then_some(v);
            }
        }
    }
    let latest = fetch_latest()?;
    if let Some(dir) = dir {
        let _ = CheckCache::new(&latest, now).save(dir);
    }
    (latest > *current).then_some(latest)
}

#[cfg(test)]
mod tests {
    use super::super::test_dir;
    use super::*;
    use std::cell::Cell;

    fn v(s: &str) -> Version {
        s.parse().unwrap()
    }

    fn now() -> DateTime<Utc> {
        "2026-11-09T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn the_note_round_trips_and_is_forgotten() {
        let dir = test_dir("note");
        assert!(CheckCache::load(&dir).is_none());
        CheckCache::new(&v("1.1.0"), now()).save(&dir).unwrap();
        let back = CheckCache::load(&dir).unwrap();
        assert_eq!(back.version(), Some(v("1.1.0")));
        assert_eq!(back.checked_at, now());
        assert!(back.fresh_at(now(), TTL));
        CheckCache::forget(&dir);
        assert!(CheckCache::load(&dir).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_note_from_yesterday_is_stale_and_one_from_the_future_is_not_believed() {
        let old = CheckCache {
            checked_at: now() - chrono::Duration::hours(25),
            latest: "1.1.0".into(),
        };
        assert!(!old.fresh_at(now(), TTL));
        assert!(old.fresh_at(now(), Duration::from_secs(48 * 3600)));
        let future = CheckCache {
            checked_at: now() + chrono::Duration::hours(2),
            latest: "1.1.0".into(),
        };
        assert!(!future.fresh_at(now(), TTL));
    }

    #[test]
    fn a_fresh_note_answers_without_asking_github() {
        let dir = test_dir("fresh");
        CheckCache::new(&v("1.1.0"), now() - chrono::Duration::hours(1))
            .save(&dir)
            .unwrap();
        let asked = Cell::new(false);
        let fetch = || {
            asked.set(true);
            Some(v("9.9.9"))
        };
        let got = newer_than_with(&v("1.0.0"), Some(&dir), TTL, now(), fetch);
        assert_eq!(got, Some(v("1.1.0")));
        assert!(!asked.get());
        // la misma nota, ya con la versión al día: sin aviso
        let same = newer_than_with(&v("1.1.0"), Some(&dir), TTL, now(), || None);
        assert_eq!(same, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stale_note_asks_github_and_is_rewritten() {
        let dir = test_dir("stale");
        CheckCache::new(&v("1.0.5"), now() - chrono::Duration::hours(30))
            .save(&dir)
            .unwrap();
        let got = newer_than_with(&v("1.0.0"), Some(&dir), TTL, now(), || Some(v("1.1.0")));
        assert_eq!(got, Some(v("1.1.0")));
        let note = CheckCache::load(&dir).unwrap();
        assert_eq!(note.latest, "1.1.0");
        assert_eq!(note.checked_at, now());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_network_or_a_broken_note_is_no_notice_and_no_error() {
        let dir = test_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(CheckCache::path(&dir), "{ nope").unwrap();
        assert!(CheckCache::load(&dir).is_none());
        assert_eq!(
            newer_than_with(&v("1.0.0"), Some(&dir), TTL, now(), || None),
            None
        );
        // sin red no se escribe nada: la nota rota sigue igual
        assert_eq!(
            std::fs::read_to_string(CheckCache::path(&dir)).unwrap(),
            "{ nope"
        );
        // una nota con una versión que no lo es: se pregunta de nuevo
        std::fs::write(
            CheckCache::path(&dir),
            r#"{"checked_at":"2026-11-09T11:00:00Z","latest":"nope"}"#,
        )
        .unwrap();
        assert_eq!(
            newer_than_with(&v("1.0.0"), Some(&dir), TTL, now(), || Some(v("1.1.0"))),
            Some(v("1.1.0"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_a_cache_dir_it_asks_and_remembers_nothing() {
        let got = newer_than_with(&v("1.0.0"), None, TTL, now(), || Some(v("1.1.0")));
        assert_eq!(got, Some(v("1.1.0")));
        assert_eq!(
            newer_than_with(&v("1.2.0"), None, TTL, now(), || Some(v("1.1.0"))),
            None
        );
    }

    #[test]
    fn the_note_lives_next_to_runs_jsonl() {
        if let (Some(dir), Some(runs)) = (default_dir(), crate::runs::history::default_path()) {
            assert_eq!(Some(dir.as_path()), runs.parent());
            assert!(CheckCache::path(&dir).ends_with("claude-agents/update-check.json"));
        }
    }
}
