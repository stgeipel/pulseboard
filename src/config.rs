use anyhow::{Context, Result, bail, ensure};
use directories::ProjectDirs;
use reqwest::{
    Url,
    header::{HeaderName, HeaderValue},
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
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
    pub checks: Vec<Check>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_seconds: 60,
            timeout_seconds: 10,
            close_to_tray: true,
            notifications: true,
            checks: Vec::new(),
        }
    }
}

#[derive(Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: String,
    pub name: String,
    pub url: Url,
    #[serde(default)]
    pub method: Method,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub headers_from_env: BTreeMap<String, String>,
    pub basic_auth: Option<BasicAuth>,
    pub body: Option<String>,
    #[serde(default)]
    pub expected_status: Vec<u16>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    #[default]
    Get,
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

#[derive(Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BasicAuth {
    pub username: String,
    pub password: Option<String>,
    pub password_env: Option<String>,
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

pub fn parse(content: &str) -> Result<Config> {
    let config: Config = toml::from_str(content).map_err(|error| {
        let location = error
            .span()
            .and_then(|span| content.get(..span.start))
            .map(|prefix| {
                format!(
                    " in Zeile {}",
                    prefix.lines().count()
                        + usize::from(prefix.is_empty() || prefix.ends_with('\n'))
                )
            })
            .unwrap_or_default();
        // TOML's full error includes the source line, which can contain credentials.
        anyhow::anyhow!("Ungültiges TOML oder unbekannte Option{location}")
    })?;

    ensure!(
        (1..=86_400).contains(&config.interval_seconds),
        "interval_seconds muss zwischen 1 und 86400 liegen"
    );
    ensure!(
        valid_timeout(config.timeout_seconds),
        "timeout_seconds muss zwischen 1 und 120 liegen"
    );
    let mut ids = HashSet::new();
    for (index, check) in config.checks.iter().enumerate() {
        validate_check(check, &mut ids).with_context(|| format!("Check Nr. {}", index + 1))?;
    }
    Ok(config)
}

fn validate_check<'a>(check: &'a Check, ids: &mut HashSet<&'a str>) -> Result<()> {
    ensure!(
        !check.id.trim().is_empty() && ids.insert(check.id.as_str()),
        "Jeder Check braucht eine eindeutige, nicht leere id"
    );
    ensure!(!check.name.trim().is_empty(), "name fehlt");
    ensure!(
        matches!(check.url.scheme(), "http" | "https") && check.url.host().is_some(),
        "url muss eine HTTP- oder HTTPS-Adresse sein"
    );
    ensure!(
        check.url.username().is_empty() && check.url.password().is_none(),
        "Zugangsdaten bitte über Header oder basic_auth angeben"
    );
    ensure!(
        check.url.fragment().is_none(),
        "url darf kein Fragment enthalten"
    );
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
