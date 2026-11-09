//! Los números de versión que publica claude-agents: `MAJOR.MINOR.PATCH` con
//! una etiqueta de pre-release opcional. Es poco para una dependencia, y lo
//! único que se le pide es el orden. Igual que `moon_updater::version`.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// Lo que va tras el `-`: `rc.1`, `beta`… Una pre-release va antes que la
    /// release a la que lleva, como dice semver.
    pub pre: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseVersionError(String);

impl fmt::Display for ParseVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not a version like 1.2.3", self.0)
    }
}

impl std::error::Error for ParseVersionError {}

impl Version {
    #[cfg(test)]
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            pre: None,
        }
    }

    /// La etiqueta de la release que lleva esta versión: `v1.1.0`.
    pub fn tag(&self) -> String {
        format!("v{self}")
    }
}

impl FromStr for Version {
    type Err = ParseVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || ParseVersionError(s.to_string());
        let t = s.trim();
        let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
        // los metadatos de compilación (`+abc`) no cuentan para el orden
        let t = t.split('+').next().unwrap_or(t);
        let (core, pre) = match t.split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, Some(p.to_string())),
            Some(_) => return Err(bad()),
            None => (t, None),
        };
        let mut it = core.split('.');
        let mut num = || -> Result<u64, ParseVersionError> {
            it.next()
                .filter(|p| !p.is_empty())
                .ok_or_else(bad)?
                .parse()
                .map_err(|_| bad())
        };
        let (major, minor, patch) = (num()?, num()?, num()?);
        if it.next().is_some() {
            return Err(bad());
        }
        Ok(Self {
            major,
            minor,
            patch,
            pre,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        match &self.pre {
            Some(p) => write!(f, "-{p}"),
            None => Ok(()),
        }
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                // 1.0.0 va por encima de 1.0.0-rc.1
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => cmp_pre(a, b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Etiquetas de pre-release, identificador a identificador: los números como
/// números, lo demás como texto, y un número por debajo de lo alfanumérico.
fn cmp_pre(a: &str, b: &str) -> Ordering {
    let mut ai = a.split('.');
    let mut bi = b.split('.');
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return Ordering::Equal,
            // el que se queda antes sin identificadores es el menor
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(nx), Ok(ny)) => nx.cmp(&ny),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        s.parse().expect("versión válida")
    }

    #[test]
    fn it_accepts_the_v_from_the_tag() {
        assert_eq!(v("v1.2.3"), Version::new(1, 2, 3));
        assert_eq!(v("1.2.3"), Version::new(1, 2, 3));
        assert_eq!(v("  v1.1.0 "), Version::new(1, 1, 0));
    }

    #[test]
    fn it_rejects_what_is_not_a_version() {
        for s in ["", "1.2", "1.2.3.4", "1.2.x", "v", "1.2.3-", "abc"] {
            assert!(s.parse::<Version>().is_err(), "{s} no debería pasar");
        }
        assert_eq!(
            "1.2".parse::<Version>().unwrap_err().to_string(),
            "`1.2` is not a version like 1.2.3"
        );
    }

    #[test]
    fn it_orders_by_number_and_puts_pre_releases_last() {
        assert!(v("1.1.0") > v("1.0.0"));
        assert!(v("1.0.10") > v("1.0.9"));
        assert!(v("2.0.0") > v("1.9.9"));
        assert!(v("1.0.0") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.2") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.10") > v("1.0.0-rc.2"));
        assert!(v("1.0.0-beta") < v("1.0.0-rc"));
        assert_eq!(v("1.1.0"), v("v1.1.0"));
    }

    #[test]
    fn it_prints_as_it_parses() {
        for s in ["1.1.0", "1.2.3-rc.1"] {
            assert_eq!(v(s).to_string(), s);
        }
        assert_eq!(v("1.1.0").tag(), "v1.1.0");
    }

    #[test]
    fn build_metadata_does_not_count() {
        assert_eq!(v("1.2.3+abc"), v("1.2.3"));
    }
}
