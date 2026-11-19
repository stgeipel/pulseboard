use async_channel::Sender;
use chrono::{DateTime, Local};
use gpui::{
    ClickEvent, ClipboardItem, Context, Div, FontWeight, MouseButton, SharedString, Stateful,
    Window, div, prelude::*, px, rgb,
};
use pulseboard::{
    config::{self, Check, CheckKey, Config, Server},
    monitor::{self, CheckResult, Command, Event},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    time::SystemTime,
};

const BACKGROUND: u32 = 0x0C121C;
const SURFACE: u32 = 0x121C2A;
const PANEL: u32 = 0x172435;
const STROKE: u32 = 0x263749;
const TEXT: u32 = 0xE6EEF7;
const MUTED: u32 = 0x93A9BC;
const TEAL: u32 = 0x66EAD2;
const RED: u32 = 0xFF9B9E;
const AMBER: u32 = 0xF4C482;
const HISTORY_SIZE: usize = 12;

#[derive(Default)]
struct ServiceState {
    last: Option<CheckResult>,
    history: VecDeque<bool>,
}

pub struct Dashboard {
    path: Option<PathBuf>,
    commands: Option<Sender<Command>>,
    config: Config,
    services: HashMap<CheckKey, ServiceState>,
    collapsed_servers: HashSet<String>,
    last_run: Option<SystemTime>,
    working: bool,
    config_error: Option<String>,
    action_error: Option<String>,
    clipboard_status: Option<&'static str>,
    #[cfg(target_os = "windows")]
    tray: Option<crate::windows::Tray>,
    #[cfg(target_os = "windows")]
    close_to_tray: std::rc::Rc<std::cell::Cell<bool>>,
    #[cfg(target_os = "windows")]
    autostart: bool,
}

impl Dashboard {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        #[cfg(target_os = "windows")] tray: anyhow::Result<crate::windows::Tray>,
    ) -> Self {
        let mut dashboard = Self {
            path: None,
            commands: None,
            config: Config::default(),
            services: HashMap::new(),
            collapsed_servers: HashSet::new(),
            last_run: None,
            working: true,
            config_error: None,
            action_error: None,
            clipboard_status: None,
            #[cfg(target_os = "windows")]
            tray: None,
            #[cfg(target_os = "windows")]
            close_to_tray: Default::default(),
            #[cfg(target_os = "windows")]
            autostart: false,
        };

        match config::path() {
            Ok(path) => {
                match monitor::start(path.clone()) {
                    Ok((commands, events)) => {
                        dashboard.commands = Some(commands);
                        cx.spawn_in(window, async move |weak, cx| {
                            while let Ok(event) = events.recv().await {
                                if weak
                                    .update_in(cx, |this, _, cx| this.receive(event, cx))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                    }
                    Err(_) => dashboard.receive(Event::Fatal, cx),
                }
                dashboard.path = Some(path);
            }
            Err(error) => dashboard.receive(Event::ConfigError(error.to_string()), cx),
        }

        #[cfg(target_os = "windows")]
        {
            use crate::windows::{self, TrayAction};

            windows::decorate(window);
            match windows::autostart_enabled() {
                Ok(enabled) => dashboard.autostart = enabled,
                Err(error) => dashboard.action_error = Some(error.to_string()),
            }
            match tray {
                Ok(tray) => {
                    let close_to_tray = dashboard.close_to_tray.clone();
                    close_to_tray.set(dashboard.config.close_to_tray);
                    window.on_window_should_close(cx, move |window, _| {
                        !close_to_tray.get() || !windows::hide(window)
                    });
                    let events = tray.events.clone();
                    cx.spawn_in(window, async move |weak, cx| {
                        while let Ok(event) = events.recv().await {
                            if weak
                                .update_in(cx, |this, window, cx| match event {
                                    TrayAction::Open => windows::restore(window),
                                    TrayAction::Run => this.send(Command::Run, cx),
                                    TrayAction::Quit => cx.quit(),
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                    dashboard.tray = Some(tray);
                }
                Err(_) => {
                    dashboard.action_error =
                        Some("Das Windows-Tray konnte nicht eingerichtet werden.".into());
                }
            }
        }

        dashboard
    }

    fn receive(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Configured(config) => {
                self.services.retain(|id, _| {
                    self.config.timeout_seconds == config.timeout_seconds
                        && self
                            .config
                            .checks
                            .iter()
                            .find(|check| &check.key == id)
                            .is_some_and(|old| config.checks.iter().any(|new| old == new))
                });
                self.collapsed_servers
                    .retain(|id| config.servers.iter().any(|server| &server.id == id));
                self.working = !config.checks.is_empty();
                self.config = config;
                self.config_error = None;
                self.clipboard_status = None;
                #[cfg(target_os = "windows")]
                self.close_to_tray.set(self.config.close_to_tray);
            }
            Event::ConfigError(error) => {
                self.config_error = Some(error);
                self.working = false;
            }
            Event::Started => self.working = true,
            Event::Finished(results, timestamp) => {
                self.working = false;
                self.last_run = Some(timestamp);
                self.apply_results(results, cx);
            }
            Event::Fatal => {
                self.commands = None;
                self.working = false;
                self.config_error =
                    Some("Der Healthcheck-Dienst konnte nicht gestartet werden.".into());
            }
        }
        #[cfg(target_os = "windows")]
        if let Some(tray) = &self.tray {
            let (healthy, failed, _) = self.counts();
            tray.update_tooltip(healthy, failed, self.config.checks.len());
        }
        cx.notify();
    }

    fn apply_results(&mut self, results: Vec<CheckResult>, _cx: &mut Context<Self>) {
        #[cfg(target_os = "windows")]
        let mut changes = Vec::new();
        for result in results {
            let state = self.services.entry(result.key.clone()).or_default();
            let healthy = result.outcome.is_healthy();
            #[cfg(target_os = "windows")]
            {
                let previous = state.last.as_ref().map(|last| last.outcome.is_healthy());
                let changed = previous.map_or(!healthy, |previous| previous != healthy);
                if self.config.notifications
                    && changed
                    && let Some(check) = self
                        .config
                        .checks
                        .iter()
                        .find(|check| check.key == result.key)
                {
                    let name = check
                        .key
                        .server
                        .as_ref()
                        .and_then(|id| self.config.servers.iter().find(|server| &server.id == id))
                        .map_or_else(
                            || check.request.name.clone(),
                            |server| format!("{} / {}", server.name, check.request.name),
                        );
                    changes.push((healthy, name, result.outcome.label()));
                }
            }
            if state.history.len() == HISTORY_SIZE {
                state.history.pop_front();
            }
            state.history.push_back(healthy);
            state.last = Some(result);
        }
        #[cfg(target_os = "windows")]
        if !changes.is_empty() {
            _cx.spawn(async move |weak, cx| {
                let error = cx
                    .background_executor()
                    .spawn(async move {
                        let mut error = None;
                        for healthy in [false, true] {
                            let mut matching =
                                changes.iter().filter(|(state, _, _)| *state == healthy);
                            let Some((_, name, outcome)) = matching.next() else {
                                continue;
                            };
                            let remaining = matching.count();
                            let title = if healthy {
                                "Dienst wieder erreichbar"
                            } else {
                                "Healthcheck fehlgeschlagen"
                            };
                            let body = if remaining == 0 {
                                format!("{name} · {outcome}")
                            } else {
                                format!("{name} und {remaining} weitere")
                            };
                            if let Err(failure) = crate::windows::notify(title, &body) {
                                error = Some(failure.to_string());
                            }
                        }
                        error
                    })
                    .await;
                if let Some(error) = error {
                    let _ = weak.update(cx, |this, cx| {
                        this.action_error = Some(error);
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    }

    fn counts(&self) -> (usize, usize, usize) {
        let healthy = self
            .services
            .values()
            .filter(|state| {
                state
                    .last
                    .as_ref()
                    .is_some_and(|last| last.outcome.is_healthy())
            })
            .count();
        let failed = self.services.len() - healthy;
        (
            healthy,
            failed,
            self.config.checks.len().saturating_sub(healthy + failed),
        )
    }

    fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        if self.working || matches!(command, Command::Run) && self.config.checks.is_empty() {
            return;
        }
        if self
            .commands
            .as_ref()
            .is_some_and(|channel| channel.try_send(command).is_ok())
        {
            self.working = true;
            cx.notify();
        }
    }

    fn open_config(&mut self, cx: &mut Context<Self>) {
        let result = self
            .path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Konfigurationspfad fehlt"))
            .and_then(|path| {
                config::ensure_file(path)?;
                #[cfg(target_os = "windows")]
                std::process::Command::new("notepad.exe")
                    .arg(path)
                    .spawn()?;
                #[cfg(not(target_os = "windows"))]
                open::that_detached(path)?;
                Ok(())
            });
        self.action_error = result.err().map(|error| error.to_string());
        cx.notify();
    }

    #[cfg(target_os = "windows")]
    fn toggle_autostart(&mut self, cx: &mut Context<Self>) {
        match crate::windows::set_autostart(!self.autostart) {
            Ok(()) => {
                self.autostart = !self.autostart;
                self.action_error = None;
            }
            Err(_) => {
                self.action_error =
                    Some("Der Windows-Autostart konnte nicht geändert werden.".into())
            }
        }
        cx.notify();
    }

    fn open_releases(&mut self, cx: &mut Context<Self>) {
        if let Some(repository) = release_repository() {
            self.action_error =
                open::that_detached(format!("https://github.com/{repository}/releases/latest"))
                    .err()
                    .map(|_| "Die GitHub Releases konnten nicht geöffnet werden.".into());
            cx.notify();
        }
    }

    fn copy_to_clipboard(
        &mut self,
        block: anyhow::Result<String>,
        status: &'static str,
        cx: &mut Context<Self>,
    ) {
        self.clipboard_status = None;
        match block {
            Ok(block) => {
                cx.write_to_clipboard(ClipboardItem::new_string(block));
                self.clipboard_status = Some(status);
            }
            Err(error) => self.action_error = Some(format!("Kopieren fehlgeschlagen. {error}")),
        }
        cx.notify();
    }

    fn server_group(&self, server: &Server, cx: &mut Context<Self>) -> Div {
        let checks = || {
            self.config
                .checks
                .iter()
                .filter(|check| check.key.server.as_deref() == Some(server.id.as_str()))
        };
        let (total, failed) = checks().fold((0, 0), |(total, failed), check| {
            let failed_check = self
                .services
                .get(&check.key)
                .and_then(|state| state.last.as_ref())
                .is_some_and(|result| !result.outcome.is_healthy());
            (total + 1, failed + usize::from(failed_check))
        });
        let collapsed = self.collapsed_servers.contains(&server.id);
        let toggle_id = server.id.clone();
        let copy_id = server.id.clone();
        let summary = if failed == 0 {
            format!("{total} Checks")
        } else {
            format!("{total} Checks · {failed} Fehler")
        };
        div()
            .flex()
            .flex_col()
            .rounded_xl()
            .border_1()
            .border_color(rgb(STROKE))
            .bg(rgb(SURFACE))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .bg(rgb(PANEL))
                    .px_4()
                    .py_3()
                    .child(
                        button(
                            format!("toggle-server:{}", server.id),
                            if collapsed {
                                "Aufklappen"
                            } else {
                                "Einklappen"
                            },
                            false,
                            true,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                if !this.collapsed_servers.remove(&toggle_id) {
                                    this.collapsed_servers.insert(toggle_id.clone());
                                }
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(server.name.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(if failed == 0 { MUTED } else { RED }))
                            .child(summary),
                    )
                    .child(
                        button(
                            format!("copy-server:{}", server.id),
                            "Server kopieren",
                            false,
                            true,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                let block = this.config.copy_server_toml(&copy_id);
                                this.copy_to_clipboard(
                                    block,
                                    "Server und Checks als TOML kopiert",
                                    cx,
                                );
                            },
                        )),
                    ),
            )
            .when(!collapsed && total > 0, |group| {
                group.child(self.service_table(checks(), cx))
            })
    }

    fn service_table<'a>(
        &self,
        checks: impl Iterator<Item = &'a Check>,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .px_4()
                    .py_3()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(MUTED))
                    .child(div().flex_1().child("DIENST"))
                    .child(div().w(px(160.0)).flex_none().child("STATUS"))
                    .child(div().w(px(72.0)).flex_none().child("LATENZ"))
                    .child(div().w(px(122.0)).flex_none().child("LETZTE 12 CHECKS"))
                    .child(div().w(px(90.0)).flex_none()),
            )
            .children(checks.map(|check| self.service_row(check, cx)))
    }

    fn service_row(&self, check: &Check, cx: &mut Context<Self>) -> Div {
        let state = self.services.get(&check.key);
        let result = state.and_then(|state| state.last.as_ref());
        let (label, accent) = match result {
            Some(result) if result.outcome.is_healthy() => ("Erreichbar", TEAL),
            Some(_) => ("Fehler", RED),
            None => ("Ausstehend", MUTED),
        };
        let endpoint = format!(
            "{}{}",
            check.url.origin().ascii_serialization(),
            check.url.path()
        );
        let key = check.key.clone();
        let scope = key.server.as_deref().unwrap_or_default();
        let button_id = format!("copy-check:{}:{scope}:{}", scope.len(), key.id);
        let details = result.map_or_else(
            || "Noch kein Ergebnis".to_string(),
            |result| result.outcome.label(),
        );
        let latency = result.map_or_else(
            || "—".to_string(),
            |result| format!("{} ms", result.elapsed.as_millis()),
        );
        let history = state.map(|state| &state.history);

        div()
            .flex()
            .items_center()
            .gap_4()
            .py_3()
            .px_4()
            .border_t_1()
            .border_color(rgb(STROKE))
            .hover(|style| style.bg(rgb(PANEL)))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(check.request.name.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(
                                div()
                                    .flex_none()
                                    .rounded_sm()
                                    .bg(rgb(PANEL))
                                    .px_1()
                                    .text_color(rgb(TEAL))
                                    .child(check.request.method.label()),
                            )
                            .child(
                                div()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .min_w_0()
                                    .child(endpoint),
                            ),
                    ),
            )
            .child(
                div()
                    .w(px(160.0))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .text_color(rgb(accent))
                            .child(dot(accent))
                            .child(label),
                    )
                    .child(
                        div()
                            .text_xs()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_color(rgb(MUTED))
                            .child(details),
                    ),
            )
            .child(div().w(px(72.0)).flex_none().text_sm().child(latency))
            .child(
                div()
                    .w(px(122.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .children((0..HISTORY_SIZE).map(|index| {
                        let sample = history
                            .and_then(|history| {
                                index
                                    .checked_sub(HISTORY_SIZE - history.len())
                                    .and_then(|index| history.get(index))
                            })
                            .copied();
                        let color = match sample {
                            Some(true) => 0x3AAE9E,
                            Some(false) => 0xEB7F88,
                            None => STROKE,
                        };
                        div().w(px(7.0)).h(px(19.0)).rounded_sm().bg(rgb(color))
                    })),
            )
            .child(div().w(px(90.0)).flex_none().child(
                button(button_id, "Kopieren", false, true).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        let block = this.config.copy_check_toml(&key);
                        this.copy_to_clipboard(block, "Healthcheck als TOML kopiert", cx);
                    },
                )),
            ))
    }
}

impl Render for Dashboard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.config.checks.len();
        let (healthy, failed, pending) = self.counts();
        let (status, status_color) = if self.working {
            ("Prüfung läuft", TEAL)
        } else if self.config_error.is_some() {
            ("Konfigurationsfehler", AMBER)
        } else if total == 0 {
            ("Wartet auf Konfiguration", MUTED)
        } else {
            ("Überwachung aktiv", TEAL)
        };
        let last_run = self
            .last_run
            .map(|timestamp| {
                let timestamp: DateTime<Local> = timestamp.into();
                format!("Letzte Prüfung {} Uhr", timestamp.format("%H:%M:%S"))
            })
            .unwrap_or_else(|| "Noch keine Prüfung abgeschlossen".into());

        div().flex().flex_col().size_full().bg(rgb(BACKGROUND)).text_color(rgb(TEXT))
            .child(div().flex_none().flex().items_center().justify_between().gap_4().h(px(67.0)).px_8()
                .border_b_1().border_color(rgb(STROKE))
                .child(div().flex().items_center().gap_3()
                    .child(wordmark())
                    .child(div().text_lg().font_weight(FontWeight::BOLD).child("Pulseboard")))
                .child(div().flex().gap_2()
                    .child(button("open-config", "Konfiguration öffnen", false, self.path.is_some())
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_config(cx))))
                    .child(button("reload", "Neu laden", false, !self.working && self.commands.is_some())
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.send(Command::Reload, cx))))
                    .child(button("run", if self.working { "Prüfung läuft …" } else { "Jetzt prüfen" }, true, !self.working && total != 0 && self.commands.is_some())
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.send(Command::Run, cx))))))
            .child(div().flex_1().min_h_0().id("dashboard-scroll").overflow_y_scroll()
                .child(div().flex().flex_col().gap_5().px_8().py_5()
                    .child(div().flex().flex_col().gap_3().bg(rgb(0x112334)).border_1().border_color(rgb(0x284356)).rounded_xl().px_6().py_5()
                        .child(div().flex().items_center().gap_2().text_xs().font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(status_color)).child(dot(status_color)).child(status))
                        .child(div().text_size(px(34.0)).font_weight(FontWeight::BOLD).child("Serviceübersicht"))
                        .child(div().flex().items_center().gap_3().text_sm().text_color(rgb(MUTED))
                            .child(format!("Intervall {} s", self.config.interval_seconds))
                            .child("·")
                            .child(last_run)))
                    .children([self.config_error.as_ref(), self.action_error.as_ref()].into_iter().flatten().map(|error| {
                        div().bg(rgb(0x30251D)).rounded_md().px_4().py_3().text_sm().text_color(rgb(AMBER)).child(error.clone())
                    }))
                    .children(self.clipboard_status.map(|status| {
                        div().bg(rgb(PANEL)).rounded_md().px_4().py_3().text_sm().text_color(rgb(TEAL)).child(status)
                    }))
                    .child(div().flex().gap_3()
                        .child(metric("Endpunkte", total, TEXT))
                        .child(metric("Erreichbar", healthy, TEAL))
                        .child(metric("Mit Fehlern", failed, RED))
                        .child(metric("Ausstehend", pending, MUTED)))
                    .child(div().flex().flex_col().gap_3()
                        .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Healthchecks"))
                        .child(if total == 0 && self.config.servers.is_empty() {
                            div().flex().flex_col().items_center().justify_center().gap_3().py_8().rounded_xl()
                                .border_1().border_color(rgb(STROKE)).bg(rgb(SURFACE))
                                .child(wordmark())
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("Noch keine Endpunkte eingerichtet"))
                                .child(div().text_sm().text_color(rgb(MUTED)).child("Füge deine Endpunkte in der TOML-Datei hinzu und lade sie neu."))
                                .child(button("empty-open", "Konfiguration öffnen", false, self.path.is_some())
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_config(cx))))
                        } else {
                            let mut groups = div().flex().flex_col().gap_3();
                            groups = groups.children(self.config.servers.iter().map(|server| self.server_group(server, cx)));
                            let mut standalone = self.config.checks.iter().filter(|check| check.key.server.is_none()).peekable();
                            if standalone.peek().is_some() {
                                groups = groups.child(div().flex().flex_col().rounded_xl().border_1().border_color(rgb(STROKE)).bg(rgb(SURFACE)).overflow_hidden()
                                    .child(div().bg(rgb(PANEL)).px_4().py_3().font_weight(FontWeight::SEMIBOLD).child("Einzelne Endpunkte"))
                                    .child(self.service_table(standalone, cx)));
                            }
                            groups
                        }))))
            .child(self.footer(cx))
    }
}

impl Dashboard {
    fn footer(&self, cx: &mut Context<Self>) -> Div {
        let footer = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .h(px(52.0))
            .px_8()
            .border_t_1()
            .border_color(rgb(STROKE))
            .text_sm()
            .text_color(rgb(MUTED));
        #[cfg(target_os = "windows")]
        let footer = {
            let tray_label = if self.tray.is_some() && self.config.close_to_tray {
                "Schließen · System-Tray"
            } else {
                "Schließen · Programm beenden"
            };
            footer.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        button(
                            "autostart",
                            if self.autostart {
                                "Autostart an"
                            } else {
                                "Autostart aus"
                            },
                            false,
                            true,
                        )
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_autostart(cx)),
                        ),
                    )
                    .child(tray_label),
            )
        };
        footer.child(div().flex_1()).child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(concat!("v", env!("CARGO_PKG_VERSION")))
                .when(release_repository().is_some(), |row| {
                    row.child(button("releases", "Updates", false, true).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.open_releases(cx)),
                    ))
                }),
        )
    }
}

fn release_repository() -> Option<&'static str> {
    option_env!("PULSEBOARD_REPOSITORY")
}

fn dot(color: u32) -> Div {
    div()
        .size(px(7.0))
        .flex_none()
        .rounded_full()
        .bg(rgb(color))
}

fn wordmark() -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(3.0))
        .size(px(34.0))
        .rounded_md()
        .bg(rgb(0x182C3D))
        .border_1()
        .border_color(rgb(0x305261))
        .children(
            [7.0, 15.0, 22.0, 12.0]
                .into_iter()
                .map(|h| div().w(px(2.0)).h(px(h)).rounded_full().bg(rgb(TEAL))),
        )
}

fn metric(label: &'static str, value: usize, color: u32) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .rounded_lg()
        .border_1()
        .border_color(rgb(STROKE))
        .bg(rgb(SURFACE))
        .px_4()
        .py_3()
        .child(div().text_sm().text_color(rgb(MUTED)).child(label))
        .child(
            div()
                .text_2xl()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(color))
                .child(value.to_string()),
        )
}

fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    primary: bool,
    enabled: bool,
) -> Stateful<Div> {
    let fill = if primary && enabled { TEAL } else { SURFACE };
    let foreground = if !enabled {
        MUTED
    } else if primary {
        BACKGROUND
    } else {
        TEXT
    };
    let id: SharedString = id.into();
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .h(px(32.0))
        .px_3()
        .rounded_md()
        .border_1()
        .border_color(rgb(if primary && enabled { TEAL } else { STROKE }))
        .bg(rgb(fill))
        .text_color(rgb(foreground))
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(rgb(if primary { 0x9CF2E2 } else { PANEL })))
                .active(|style| style.bg(rgb(if primary { 0x53BBAA } else { STROKE })))
        })
        .when(!enabled, |button| {
            button.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        })
        .child(label.into())
}
