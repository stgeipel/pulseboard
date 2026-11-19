use async_channel::Sender;
use chrono::{DateTime, Local};
use gpui::{
    ClickEvent, ClipboardItem, Context, Div, FocusHandle, FontWeight, MouseButton, SharedString,
    Stateful, Window, deferred, div, prelude::*, px, rgb,
};
use pulseboard::{
    appearance::{self, DEFAULT_THEME, THEMES, Theme},
    config::{self, Check, CheckKey, Config, Server},
    monitor::{self, CheckResult, Command, Event},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    time::SystemTime,
};

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
    theme: &'static Theme,
    theme_cursor: Option<usize>,
    theme_focus: FocusHandle,
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
            theme: DEFAULT_THEME,
            theme_cursor: None,
            theme_focus: cx.focus_handle(),
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
                match appearance::load(&path.with_file_name("appearance.toml")) {
                    Ok(theme) => dashboard.theme = theme,
                    Err(error) => dashboard.action_error = Some(error.to_string()),
                }
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

            windows::decorate(window, dashboard.theme.dark);
            match windows::autostart_enabled() {
                Ok(enabled) => dashboard.autostart = enabled,
                Err(error) => dashboard.action_error = Some(error.to_string()),
            }
            let tray_available = tray.is_ok();
            let close_to_tray = dashboard.close_to_tray.clone();
            close_to_tray.set(dashboard.config.close_to_tray);
            window.on_window_should_close(cx, move |window, _| {
                let should_hide = tray_available && close_to_tray.get();
                let hidden = should_hide && windows::hide(window);
                windows::diagnostic_event(if hidden {
                    "window hidden in tray"
                } else if should_hide {
                    "could not hide window in tray; closing window"
                } else {
                    "window close accepted"
                });
                !hidden
            });
            match tray {
                Ok(tray) => {
                    let events = tray.events.clone();
                    cx.spawn_in(window, async move |weak, cx| {
                        while let Ok(event) = events.recv().await {
                            if weak
                                .update_in(cx, |this, window, cx| match event {
                                    TrayAction::Open => {
                                        windows::diagnostic_event("restoring window from tray");
                                        windows::restore(window);
                                    }
                                    TrayAction::Run => this.send(Command::Run, cx),
                                    TrayAction::Quit => {
                                        windows::diagnostic_event("quit requested from tray");
                                        cx.quit();
                                    }
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
                    windows::diagnostic_event("tray unavailable");
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
                #[cfg(target_os = "windows")]
                crate::windows::diagnostic_event("health check configuration loaded");
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
                #[cfg(target_os = "windows")]
                crate::windows::diagnostic_event("health check configuration failed to load");
                self.config_error = Some(error);
                self.working = false;
            }
            Event::Started => {
                #[cfg(target_os = "windows")]
                if self.last_run.is_none() {
                    crate::windows::diagnostic_event("first health check batch started");
                }
                self.working = true;
            }
            Event::Finished(results, timestamp) => {
                #[cfg(target_os = "windows")]
                if self.last_run.is_none() {
                    crate::windows::diagnostic_event("first health check batch finished");
                }
                self.working = false;
                self.last_run = Some(timestamp);
                self.apply_results(results, cx);
            }
            Event::Fatal => {
                #[cfg(target_os = "windows")]
                crate::windows::diagnostic_event("health check worker failed");
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

    #[cfg(target_os = "windows")]
    fn open_crash_log(&mut self, cx: &mut Context<Self>) {
        self.action_error = crate::windows::crash_log_path()
            .ok_or_else(|| anyhow::anyhow!("Das Absturzprotokoll konnte nicht angelegt werden."))
            .and_then(|path| {
                std::process::Command::new("notepad.exe")
                    .arg(path)
                    .spawn()?;
                Ok(())
            })
            .err()
            .map(|err| err.to_string());
        cx.notify();
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
        let theme = self.theme;
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
            .border_color(rgb(theme.border))
            .bg(rgb(theme.surface))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .bg(rgb(if collapsed {
                        theme.panel
                    } else {
                        theme.accent_soft
                    }))
                    .border_l_4()
                    .border_color(rgb(if collapsed { theme.panel } else { theme.accent }))
                    .px_4()
                    .py_3()
                    .child(
                        button(
                            theme,
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
                            .text_color(rgb(if failed == 0 {
                                theme.muted
                            } else {
                                theme.error
                            }))
                            .child(summary),
                    )
                    .child(
                        button(
                            theme,
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
        let theme = self.theme;
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
                    .text_color(rgb(theme.muted))
                    .child(div().flex_1().child("DIENST"))
                    .child(div().w(px(160.0)).flex_none().child("STATUS"))
                    .child(div().w(px(72.0)).flex_none().child("LATENZ"))
                    .child(div().w(px(122.0)).flex_none().child("LETZTE 12 CHECKS"))
                    .child(div().w(px(90.0)).flex_none()),
            )
            .children(checks.map(|check| self.service_row(check, cx)))
    }

    fn service_row(&self, check: &Check, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let state = self.services.get(&check.key);
        let result = state.and_then(|state| state.last.as_ref());
        let (label, accent, badge) = match result {
            Some(result) if result.outcome.is_healthy() => {
                ("Erreichbar", theme.success, theme.success_soft)
            }
            Some(_) => ("Fehler", theme.error, theme.error_soft),
            None => ("Ausstehend", theme.muted, theme.panel),
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
            .border_color(rgb(theme.border))
            .hover(|style| style.bg(rgb(theme.panel)))
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
                            .text_color(rgb(theme.muted))
                            .child(
                                div()
                                    .flex_none()
                                    .rounded_sm()
                                    .bg(rgb(theme.panel))
                                    .px_1()
                                    .text_color(rgb(theme.accent_ink))
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
                    .items_start()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_md()
                            .bg(rgb(badge))
                            .px_2()
                            .text_sm()
                            .text_color(rgb(accent))
                            .child(dot(accent))
                            .child(label),
                    )
                    .child(
                        div()
                            .w_full()
                            .text_xs()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_color(rgb(theme.muted))
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
                            Some(true) => theme.success_mark,
                            Some(false) => theme.error_mark,
                            None => theme.border,
                        };
                        div().w(px(7.0)).h(px(19.0)).rounded_sm().bg(rgb(color))
                    })),
            )
            .child(div().w(px(90.0)).flex_none().child(
                button(theme, button_id, "Kopieren", false, true).on_click(cx.listener(
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
        let theme = self.theme;
        let total = self.config.checks.len();
        let (healthy, failed, pending) = self.counts();
        let (status, status_color) = if self.working {
            ("Prüfung läuft", theme.accent_ink)
        } else if self.config_error.is_some() {
            ("Konfigurationsfehler", theme.warning)
        } else if total == 0 {
            ("Wartet auf Konfiguration", theme.muted)
        } else {
            ("Überwachung aktiv", theme.success)
        };
        let last_run = self
            .last_run
            .map(|timestamp| {
                let timestamp: DateTime<Local> = timestamp.into();
                format!("Letzte Prüfung {} Uhr", timestamp.format("%H:%M:%S"))
            })
            .unwrap_or_else(|| "Noch keine Prüfung abgeschlossen".into());

        div().flex().flex_col().size_full().bg(rgb(theme.background)).text_color(rgb(theme.text))
            .child(div().flex_none().flex().items_center().justify_between().gap_4().h(px(67.0)).px_8().bg(rgb(theme.surface))
                .border_b_1().border_color(rgb(theme.border))
                .child(div().flex().items_center().gap_3()
                    .child(wordmark(theme))
                    .child(div().text_lg().font_weight(FontWeight::BOLD).child("Pulseboard")))
                .child(div().flex().gap_2()
                    .child(button(theme, "open-config", "Konfiguration öffnen", false, self.path.is_some())
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_config(cx))))
                    .child(button(theme, "reload", "Neu laden", false, !self.working && self.commands.is_some())
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.send(Command::Reload, cx))))
                    .child(button(theme, "run", if self.working { "Prüfung läuft …" } else { "Jetzt prüfen" }, true, !self.working && total != 0 && self.commands.is_some())
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.send(Command::Run, cx))))
                    .child(self.theme_picker(cx))))
            .child(div().flex_1().min_h_0().id("dashboard-scroll").overflow_y_scroll()
                .child(div().flex().flex_col().gap_5().px_8().py_5()
                    .child(div().flex().flex_col().gap_2()
                        .child(div().flex().items_center().gap_2().text_xs().font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(status_color)).child(dot(status_color)).child(status))
                        .child(div().text_size(px(34.0)).font_weight(FontWeight::BOLD).child("Serviceübersicht"))
                        .child(div().flex().items_center().gap_3().text_sm().text_color(rgb(theme.muted))
                            .child(format!("Intervall {} s", self.config.interval_seconds))
                            .child("·")
                            .child(last_run)))
                    .children([self.config_error.as_ref(), self.action_error.as_ref()].into_iter().flatten().map(|error| {
                        div().bg(rgb(theme.error_soft)).rounded_md().px_4().py_3().text_sm().text_color(rgb(theme.error)).child(error.clone())
                    }))
                    .children(self.clipboard_status.map(|status| {
                        div().bg(rgb(theme.panel)).rounded_md().px_4().py_3().text_sm().text_color(rgb(theme.accent_ink)).child(status)
                    }))
                    .child(div().flex().gap_3()
                        .child(metric(theme, "Endpunkte", total, theme.text))
                        .child(metric(theme, "Erreichbar", healthy, theme.success))
                        .child(metric(theme, "Mit Fehlern", failed, theme.error))
                        .child(metric(theme, "Ausstehend", pending, theme.warning)))
                    .child(div().flex().flex_col().gap_3()
                        .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Healthchecks"))
                        .child(if total == 0 && self.config.servers.is_empty() {
                            div().flex().flex_col().items_center().justify_center().gap_3().py_8().rounded_xl()
                                .border_1().border_color(rgb(theme.border)).bg(rgb(theme.surface))
                                .child(wordmark(theme))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("Noch keine Endpunkte eingerichtet"))
                                .child(div().text_sm().text_color(rgb(theme.muted)).child("Füge deine Endpunkte in der TOML-Datei hinzu und lade sie neu."))
                                .child(button(theme, "empty-open", "Konfiguration öffnen", false, self.path.is_some())
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_config(cx))))
                        } else {
                            let mut groups = div().flex().flex_col().gap_3();
                            groups = groups.children(self.config.servers.iter().map(|server| self.server_group(server, cx)));
                            let mut standalone = self.config.checks.iter().filter(|check| check.key.server.is_none()).peekable();
                            if standalone.peek().is_some() {
                                groups = groups.child(div().flex().flex_col().rounded_xl().border_1().border_color(rgb(theme.border)).bg(rgb(theme.surface)).overflow_hidden()
                                    .child(div().bg(rgb(theme.panel)).px_4().py_3().font_weight(FontWeight::SEMIBOLD).child("Einzelne Endpunkte"))
                                    .child(self.service_table(standalone, cx)));
                            }
                            groups
                        }))))
            .child(self.footer(cx))
            .when_some(self.theme_cursor, |dashboard, cursor| dashboard.child(self.theme_menu(cursor, cx)))
    }
}

impl Dashboard {
    fn selected_theme_index(&self) -> usize {
        THEMES
            .iter()
            .position(|theme| theme.id == self.theme.id)
            .unwrap_or(0)
    }

    fn select_theme(&mut self, theme: &'static Theme, _window: &Window, cx: &mut Context<Self>) {
        self.theme = theme;
        self.theme_cursor = None;
        self.action_error = self
            .path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Verzeichnis für Design-Einstellungen fehlt"))
            .and_then(|path| appearance::save(&path.with_file_name("appearance.toml"), theme))
            .err()
            .map(|error| format!("Design konnte nicht gespeichert werden. {error}"));
        #[cfg(target_os = "windows")]
        crate::windows::decorate(_window, theme.dark);
        cx.notify();
    }

    fn theme_picker(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = self.theme;
        button(theme, "theme-picker", "Design ▾", false, true)
            .track_focus(&self.theme_focus)
            .tab_index(0)
            .in_focus(|style| style.border_color(rgb(theme.accent)))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.theme_focus.focus(window);
                this.theme_cursor = Some(this.selected_theme_index());
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                let selected = this.selected_theme_index();
                let cursor = this.theme_cursor;
                match event.keystroke.key.as_str() {
                    "escape" if cursor.is_some() => this.theme_cursor = None,
                    "down" => {
                        this.theme_cursor =
                            Some(cursor.map_or(selected, |index| (index + 1) % THEMES.len()))
                    }
                    "up" => {
                        this.theme_cursor =
                            Some(cursor.map_or(selected, |index| {
                                (index + THEMES.len() - 1) % THEMES.len()
                            }))
                    }
                    "enter" | "space" => {
                        if let Some(index) = cursor {
                            this.select_theme(THEMES[index], window, cx);
                        } else {
                            this.theme_cursor = Some(selected);
                        }
                    }
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
    }

    fn theme_menu(&self, cursor: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        deferred(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.theme_cursor = None;
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(56.0))
                        .right(px(32.0))
                        .w(px(255.0))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .rounded_lg()
                        .p_1()
                        .bg(rgb(theme.surface))
                        .border_1()
                        .border_color(rgb(theme.border))
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .children(THEMES.iter().copied().enumerate().map(|(index, option)| {
                            div()
                                .id(option.id)
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .py_2()
                                .rounded_md()
                                .text_sm()
                                .cursor_pointer()
                                .when(index == cursor, |row| row.bg(rgb(theme.accent_soft)))
                                .hover(|style| style.bg(rgb(theme.panel)))
                                .child(div().size(px(13.0)).rounded_sm().bg(rgb(option.accent)))
                                .child(div().flex_1().child(option.name))
                                .when(option.id == theme.id, |row| {
                                    row.child(div().text_color(rgb(theme.accent_ink)).child("✓"))
                                })
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.select_theme(option, window, cx)
                                }))
                        })),
                ),
        )
        .with_priority(1)
    }

    fn footer(&self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let footer = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .h(px(52.0))
            .px_8()
            .border_t_1()
            .border_color(rgb(theme.border))
            .text_sm()
            .text_color(rgb(theme.muted));
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
                            theme,
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
                    .child(tray_label)
                    .child(
                        button(theme, "crash-log", "Absturzprotokoll", false, true).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.open_crash_log(cx)),
                        ),
                    ),
            )
        };
        footer.child(div().flex_1()).child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(concat!("v", env!("CARGO_PKG_VERSION")))
                .when(release_repository().is_some(), |row| {
                    row.child(button(theme, "releases", "Updates", false, true).on_click(
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

fn wordmark(theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(3.0))
        .size(px(34.0))
        .rounded_md()
        .bg(rgb(theme.accent_soft))
        .border_1()
        .border_color(rgb(theme.border))
        .children([7.0, 15.0, 22.0, 12.0].into_iter().map(|h| {
            div()
                .w(px(2.0))
                .h(px(h))
                .rounded_full()
                .bg(rgb(theme.accent_ink))
        }))
}

fn metric(theme: &Theme, label: &'static str, value: usize, color: u32) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .rounded_lg()
        .border_1()
        .border_color(rgb(theme.border))
        .bg(rgb(theme.surface))
        .px_4()
        .py_3()
        .child(div().text_sm().text_color(rgb(theme.muted)).child(label))
        .child(
            div()
                .text_2xl()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(color))
                .child(value.to_string()),
        )
}

fn button(
    theme: &Theme,
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    primary: bool,
    enabled: bool,
) -> Stateful<Div> {
    let fill = if primary && enabled {
        theme.accent
    } else {
        theme.surface
    };
    let foreground = if !enabled {
        theme.muted
    } else if primary {
        theme.accent_text
    } else {
        theme.text
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
        .border_color(rgb(if primary && enabled {
            theme.accent
        } else {
            theme.border
        }))
        .bg(rgb(fill))
        .text_color(rgb(foreground))
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| {
                    style.bg(rgb(if primary {
                        theme.accent_hover
                    } else {
                        theme.panel
                    }))
                })
                .active(|style| {
                    style.bg(rgb(if primary {
                        theme.accent_active
                    } else {
                        theme.border
                    }))
                })
        })
        .when(!enabled, |button| {
            button.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        })
        .child(label.into())
}
