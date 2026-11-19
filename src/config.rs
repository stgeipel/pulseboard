use anyhow::{Context, Result, bail, ensure};
use directories::ProjectDirs;
use reqwest::{
    Url,
    header::{HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    ops::Range,
    path::{Path, PathBuf},
};

const EXAMPLE: &str = include_str!("../config.example.toml");

#[derive(Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub interval_seconds: u64,
    pub timeout_seconds: u64,
    pub close_to_tray: bool,
    pub notifications: bool,
    pub servers: Vec<Server>,
    #[serde(rename = "checks")]
    standalone: Vec<CheckDefinition>,
    #[serde(skip)]
    pub checks: Vec<Check>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_seconds: 60,
            timeout_seconds: 10,
            close_to_tray: true,
            notifications: true,
            servers: Vec::new(),
            standalone: Vec::new(),
            checks: Vec::new(),
        }
    }
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<Url>,
    #[serde(default)]
    checks: Vec<CheckDefinition>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CheckKey {
    pub server: Option<String>,
    pub id: String,
}

#[derive(Clone, PartialEq)]
pub struct Check {
    pub key: CheckKey,
    pub url: Url,
    pub request: CheckDefinition,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CheckDefinition {
    id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default)]
    pub method: Method,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers_from_env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basic_auth: Option<BasicAuth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(
        default,
        alias = "expected_results",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub expected_status: Vec<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    #[default]
    #[serde(alias = "get", alias = "Get")]
    Get,
    #[serde(alias = "post", alias = "Post")]
    Post,
}

impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BasicAuth {
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_env: Option<String>,
}

impl Config {
    pub fn copy_server_toml(&self, id: &str) -> Result<String> {
        #[derive(Serialize)]
        struct ServerBlock {
            servers: [Server; 1],
        }

        let mut server = self
            .servers
            .iter()
            .find(|server| server.id == id)
            .context("Servergruppe nicht gefunden")?
            .clone();
        server.id = copy_id(id, |candidate| {
            self.servers.iter().any(|server| server.id == candidate)
        });
        server.name.push_str(" (Kopie)");
        Ok(toml::to_string_pretty(&ServerBlock { servers: [server] })?)
    }

    pub fn copy_check_toml(&self, key: &CheckKey) -> Result<String> {
        #[derive(Serialize)]
        struct CheckBlock {
            checks: [CheckDefinition; 1],
        }

        let mut check = self
            .checks
            .iter()
            .find(|check| &check.key == key)
            .context("Healthcheck nicht gefunden")?
            .request
            .clone();
        check.id = copy_id(&key.id, |candidate| {
            self.checks
                .iter()
                .any(|check| check.key.server == key.server && check.key.id == candidate)
        });
        let block = CheckBlock { checks: [check] };
        if key.server.is_none() {
            return Ok(toml::to_string_pretty(&block)?);
        }

        // A check is pasted inside an existing [[servers]] block, including any header sub-tables.
        #[derive(Serialize)]
        struct GroupBlock {
            servers: CheckBlock,
        }
        Ok(toml::to_string_pretty(&GroupBlock { servers: block })?)
    }
}

fn copy_id(id: &str, in_use: impl Fn(&str) -> bool) -> String {
    let prefix = format!("{id}-copy");
    let mut candidate = prefix.clone();
    let mut suffix = 2;
    while in_use(&candidate) {
        candidate = format!("{prefix}-{suffix}");
        suffix += 1;
    }
    candidate
}

pub fn path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "Pulseboard").context("Benutzerverzeichnis fehlt")?;
    Ok(dirs.config_dir().join("config.toml"))
}

pub fn ensure_file(path: &Path) -> Result<()> {
    let Some(dir) = path.parent() else {
        bail!("Konfigurationsverzeichnis fehlt");
    };
    fs::create_dir_all(dir).context("Konfigurationsverzeichnis konnte nicht angelegt werden")?;
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file
            .write_all(EXAMPLE.as_bytes())
            .context("Konfiguration konnte nicht gespeichert werden"),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error).context("Konfiguration konnte nicht angelegt werden"),
    }
}

pub fn load(path: &Path) -> Result<Config> {
    let content = fs::read_to_string(path).context("Konfiguration konnte nicht gelesen werden")?;
    parse(&content)
}

/// A TOML or deserialization error without the raw source line, which can contain credentials.
#[derive(Debug)]
pub struct DocumentError {
    pub span: Option<Range<usize>>,
    line: Option<usize>,
}

impl std::fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Ungültiges TOML oder unbekannte Option")?;
        if let Some(line) = self.line {
            write!(formatter, " in Zeile {line}")?;
        }
        Ok(())
    }
}

impl std::error::Error for DocumentError {}

pub fn save_edit(path: &Path, original: &str, edited: &str) -> Result<()> {
    parse(edited)?;
    let directory = path.parent().context("Konfigurationsverzeichnis fehlt")?;
    let mut draft = tempfile::NamedTempFile::new_in(directory)
        .context("Temporäre Konfigurationsdatei konnte nicht angelegt werden")?;
    draft
        .write_all(edited.as_bytes())
        .context("Konfiguration konnte nicht geschrieben werden")?;

    let current = fs::read_to_string(path).context("Konfiguration konnte nicht gelesen werden")?;
    ensure!(
        current == original,
        "Die Datei wurde außerhalb von Pulseboard geändert. Sichere deinen Entwurf und öffne die Datei erneut."
    );
    draft
        .persist(path)
        .context("Konfiguration konnte nicht ersetzt werden")?;
    Ok(())
}

pub fn parse(content: &str) -> Result<Config> {
    let mut config: Config = toml::from_str(content).map_err(|error| {
        let span = error.span();
        let line = span
            .as_ref()
            .and_then(|span| content.get(..span.start))
            .map(|prefix| prefix.bytes().filter(|&byte| byte == b'\n').count() + 1);
        DocumentError { span, line }
    })?;

    ensure!(
        (1..=86_400).contains(&config.interval_seconds),
        "interval_seconds muss zwischen 1 und 86400 liegen"
    );
    ensure!(
        valid_timeout(config.timeout_seconds),
        "timeout_seconds muss zwischen 1 und 120 liegen"
    );
    config.checks = resolve_checks(&config.standalone, None)?;
    let mut ids = HashSet::new();
    for (index, server) in config.servers.iter().enumerate() {
        let result = (|| {
            ensure!(
                !server.id.trim().is_empty() && ids.insert(server.id.as_str()),
                "Jeder Server braucht eine eindeutige, nicht leere id"
            );
            ensure!(!server.name.trim().is_empty(), "name fehlt");
            if let Some(base_url) = &server.base_url {
                validate_url(base_url).context("Ungültige base_url")?;
                ensure!(
                    base_url.query().is_none(),
                    "base_url darf keinen Querystring enthalten"
                );
            }
            resolve_checks(&server.checks, Some(server))
        })()
        .with_context(|| format!("Server Nr. {}", index + 1))?;
        config.checks.extend(result);
    }
    Ok(config)
}

fn resolve_checks(checks: &[CheckDefinition], server: Option<&Server>) -> Result<Vec<Check>> {
    let mut ids = HashSet::new();
    checks
        .iter()
        .enumerate()
        .map(|(index, check)| {
            let result = (|| {
                validate_check(check, &mut ids)?;
                let url = match (&check.url, &check.path) {
                    (Some(url), None) => url.clone(),
                    (None, Some(path)) => {
                        let base_url = server
                            .and_then(|server| server.base_url.as_ref())
                            .context("path benötigt eine base_url in der Servergruppe")?;
                        ensure!(
                            path.starts_with('/')
                                && !path.starts_with("//")
                                && !path.contains('\\'),
                            "path muss mit einem einzelnen / beginnen"
                        );
                        let url = base_url.join(path).context("Ungültiger path")?;
                        ensure!(
                            url.origin() == base_url.origin(),
                            "path muss auf denselben Server zeigen"
                        );
                        url
                    }
                    _ => bail!("Jeder Check braucht entweder url oder path"),
                };
                validate_url(&url)?;
                Ok(Check {
                    key: CheckKey {
                        server: server.map(|server| server.id.clone()),
                        id: check.id.clone(),
                    },
                    url,
                    request: check.clone(),
                })
            })();
            result.with_context(|| format!("Check Nr. {}", index + 1))
        })
        .collect()
}

fn validate_url(url: &Url) -> Result<()> {
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host().is_some(),
        "url muss eine HTTP- oder HTTPS-Adresse sein"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Zugangsdaten bitte über Header oder basic_auth angeben"
    );
    ensure!(url.fragment().is_none(), "url darf kein Fragment enthalten");
    Ok(())
}

fn validate_check<'a>(check: &'a CheckDefinition, ids: &mut HashSet<&'a str>) -> Result<()> {
    ensure!(
        !check.id.trim().is_empty() && ids.insert(check.id.as_str()),
        "Jeder Check braucht eine eindeutige, nicht leere id"
    );
    ensure!(!check.name.trim().is_empty(), "name fehlt");
    ensure!(
        check.timeout_seconds.is_none_or(valid_timeout),
        "timeout_seconds muss zwischen 1 und 120 liegen"
    );
    ensure!(
        check
            .expected_status
            .iter()
            .all(|status| (100..=599).contains(status)),
        "expected_status muss HTTP-Statuscodes zwischen 100 und 599 enthalten"
    );
    let mut names = HashSet::new();
    for (name, value) in &check.headers {
        validate_header(name, &mut names, check.basic_auth.is_some())?;
        ensure!(
            HeaderValue::from_str(value).is_ok(),
            "Ungültiger Headerwert"
        );
    }
    for (name, variable) in &check.headers_from_env {
        validate_header(name, &mut names, check.basic_auth.is_some())?;
        ensure!(
            valid_env_name(variable),
            "Ungültiger Name einer Umgebungsvariable"
        );
    }
    if let Some(auth) = &check.basic_auth {
        ensure!(
            !auth.username.is_empty() && !auth.username.contains(':'),
            "basic_auth benötigt einen gültigen username"
        );
        ensure!(
            auth.password.is_some() ^ auth.password_env.is_some(),
            "basic_auth benötigt entweder password oder password_env"
        );
        ensure!(
            auth.password_env.as_deref().is_none_or(valid_env_name),
            "Ungültiger Name einer Umgebungsvariable"
        );
    }
    Ok(())
}

fn validate_header(name: &str, names: &mut HashSet<HeaderName>, basic_auth: bool) -> Result<()> {
    let name = HeaderName::from_bytes(name.as_bytes()).context("Ungültiger Headername")?;
    ensure!(
        !basic_auth || name != reqwest::header::AUTHORIZATION,
        "Authorization und basic_auth können nicht gemeinsam verwendet werden"
    );
    ensure!(names.insert(name), "Doppelt definierter Header");
    Ok(())
}

fn valid_timeout(seconds: u64) -> bool {
    (1..=120).contains(&seconds)
}

fn valid_env_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

#[cfg(test)]
mod editor_tests {
    use super::*;

    #[test]
    fn preserves_invalid_file_and_applies_valid_edits_verbatim() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        let original = "interval_seconds = 0\n";
        fs::write(&path, original)?;

        assert!(save_edit(&path, original, "interval_seconds = -1").is_err());
        assert_eq!(fs::read_to_string(&path)?, original);

        let edited = "# Benutzerkommentar\r\ninterval_seconds = 60\r\n";
        save_edit(&path, original, edited)?;
        assert_eq!(fs::read_to_string(&path)?, edited);
        assert_eq!(load(&path)?.interval_seconds, 60);

        assert!(save_edit(&path, original, "interval_seconds = 90").is_err());
        assert_eq!(fs::read_to_string(&path)?, edited);
        Ok(())
    }

    #[test]
    fn retains_source_span_without_including_secret_text_in_diagnostic() {
        let secret = "secret-content";
        let invalid = format!("# Konfiguration\nunknown = \"{secret}\"\n");
        let error = parse(&invalid).err().expect("expected a rejected field");
        let diagnostic = error.downcast_ref::<DocumentError>().unwrap();
        assert_eq!(diagnostic.line, Some(2));
        assert!(diagnostic.span.is_some());
        assert!(!error.to_string().contains(secret));
    }
}
