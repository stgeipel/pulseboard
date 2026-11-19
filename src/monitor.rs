use crate::config::{self, Check, Config, Method};
use async_channel::{Receiver, Sender};
use futures_util::{StreamExt, stream};
use reqwest::{Client, header::HeaderValue, redirect::Policy};
use std::{
    env,
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};
use tokio::time::{Instant as TokioInstant, sleep_until};

pub enum Command {
    Run,
    Reload,
}

pub enum Event {
    Configured(Config),
    ConfigError(String),
    Started,
    Finished(Vec<CheckResult>, SystemTime),
    Fatal,
}

#[derive(Clone)]
pub struct CheckResult {
    pub id: String,
    pub outcome: Outcome,
    pub elapsed: Duration,
}

#[derive(Clone)]
pub enum Outcome {
    Healthy(u16),
    Unhealthy(u16),
    Timeout,
    Unreachable,
    RequestFailed,
    CredentialUnavailable,
}

impl Outcome {
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy(_))
    }

    pub fn label(&self) -> String {
        match self {
            Self::Healthy(code) | Self::Unhealthy(code) => format!("HTTP {code}"),
            Self::Timeout => "Zeitlimit überschritten".into(),
            Self::Unreachable => "Verbindung fehlgeschlagen".into(),
            Self::RequestFailed => "Anfrage fehlgeschlagen".into(),
            Self::CredentialUnavailable => "Zugangsdaten fehlen oder sind ungültig".into(),
        }
    }
}

pub fn start(path: PathBuf) -> std::io::Result<(Sender<Command>, Receiver<Event>)> {
    let (commands, incoming) = async_channel::bounded(1);
    let (events, outgoing) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("pulseboard-monitor".into())
        .spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .and_then(|runtime| runtime.block_on(worker(path, incoming, &events)));
            if result.is_err() {
                let _ = events.send_blocking(Event::Fatal);
            }
        })?;
    Ok((commands, outgoing))
}

async fn worker(
    path: PathBuf,
    incoming: Receiver<Command>,
    outgoing: &Sender<Event>,
) -> std::io::Result<()> {
    let client = Client::builder()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(std::io::Error::other)?;
    let mut config = None;
    let mut deadline = None;
    let mut command = Command::Reload;

    loop {
        if matches!(command, Command::Reload) {
            match config::ensure_file(&path).and_then(|()| config::load(&path)) {
                Ok(loaded) => {
                    if outgoing
                        .send(Event::Configured(loaded.clone()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    deadline = (!loaded.checks.is_empty()).then(TokioInstant::now);
                    config = Some(loaded);
                }
                Err(error) => {
                    if outgoing
                        .send(Event::ConfigError(format!("{error:#}")))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        } else if let Some(config) = &config
            && !config.checks.is_empty()
        {
            let started = TokioInstant::now();
            if outgoing.send(Event::Started).await.is_err() {
                break;
            }
            let results = stream::iter(
                config
                    .checks
                    .iter()
                    .map(|check| execute(&client, check, config.timeout_seconds)),
            )
            .buffer_unordered(12)
            .collect()
            .await;
            if outgoing
                .send(Event::Finished(results, SystemTime::now()))
                .await
                .is_err()
            {
                break;
            }
            let period = Duration::from_secs(config.interval_seconds);
            let now = TokioInstant::now();
            deadline = Some(if started + period > now {
                started + period
            } else {
                now + period
            });
        }

        command = if let Some(deadline) = deadline {
            tokio::select! {
                biased;
                value = incoming.recv() => match value {
                    Ok(value) => value,
                    Err(_) => break,
                },
                () = sleep_until(deadline) => Command::Run,
            }
        } else {
            match incoming.recv().await {
                Ok(value) => value,
                Err(_) => break,
            }
        };
    }
    Ok(())
}

async fn execute(client: &Client, check: &Check, default_timeout: u64) -> CheckResult {
    let started = Instant::now();
    let outcome = send(client, check, default_timeout).await;
    CheckResult {
        id: check.id.clone(),
        outcome,
        elapsed: started.elapsed(),
    }
}

async fn send(client: &Client, check: &Check, default_timeout: u64) -> Outcome {
    let verb = match check.method {
        Method::Get => reqwest::Method::GET,
        Method::Post => reqwest::Method::POST,
    };
    let mut request = client
        .request(verb, check.url.clone())
        .timeout(Duration::from_secs(
            check.timeout_seconds.unwrap_or(default_timeout),
        ));

    for (name, value) in &check.headers {
        request = request.header(name, value);
    }
    for (name, variable) in &check.headers_from_env {
        let Some(mut value) = env::var(variable)
            .ok()
            .and_then(|value| HeaderValue::from_str(&value).ok())
        else {
            return Outcome::CredentialUnavailable;
        };
        value.set_sensitive(true);
        request = request.header(name, value);
    }
    if let Some(auth) = &check.basic_auth {
        let password = if let Some(variable) = &auth.password_env {
            match env::var(variable) {
                Ok(password) => password,
                Err(_) => return Outcome::CredentialUnavailable,
            }
        } else if let Some(password) = &auth.password {
            password.clone()
        } else {
            return Outcome::CredentialUnavailable;
        };
        request = request.basic_auth(&auth.username, Some(password));
    }
    if let Some(body) = &check.body {
        request = request.body(body.clone());
    }

    match request.send().await {
        Ok(response) => {
            let status = response.status();
            let code = status.as_u16();
            let healthy = if check.expected_status.is_empty() {
                status.is_success()
            } else {
                check.expected_status.contains(&code)
            };
            if healthy {
                Outcome::Healthy(code)
            } else {
                Outcome::Unhealthy(code)
            }
        }
        Err(error) if error.is_timeout() => Outcome::Timeout,
        Err(error) if error.is_connect() => Outcome::Unreachable,
        Err(_) => Outcome::RequestFailed,
    }
}
