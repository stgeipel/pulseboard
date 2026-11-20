mod server_list;

use super::{Dashboard, button, with_glow};
use gpui::{
    ClickEvent, Context, Div, FontWeight, KeyDownEvent, SharedString, Window, div, prelude::*, px,
    rgb,
};
use pulseboard::{
    appearance::{ServerLayout, Theme},
    config::{Check, Server},
    monitor::CheckResult,
};
use std::collections::{HashMap, HashSet};

struct ServerRow<'a> {
    server: &'a Server,
    checks: HashMap<&'a str, &'a Check>,
    healthy: usize,
    failed: usize,
    first_failure: Option<(&'a Check, &'a CheckResult)>,
}

impl ServerRow<'_> {
    fn ratio(&self) -> String {
        format!("{}/{}", self.healthy, self.checks.len())
    }
}

impl Dashboard {
    pub(super) fn server_overview(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let mut rows: Vec<_> = self
            .config
            .servers
            .iter()
            .map(|server| ServerRow {
                server,
                checks: HashMap::new(),
                healthy: 0,
                failed: 0,
                first_failure: None,
            })
            .collect();
        let indices: HashMap<_, _> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.server.id.as_str(), index))
            .collect();
        let mut seen = HashSet::new();
        let mut columns = Vec::new();
        let mut standalone = Vec::new();
        for check in &self.config.checks {
            let result = self.check_result(check);
            if let Some(index) = check.key.server.as_deref().and_then(|id| indices.get(id)) {
                let row = &mut rows[*index];
                let id = check.key.id.as_str();
                row.checks.insert(id, check);
                if seen.insert(id) {
                    columns.push((id, check.request.name.as_str()));
                }
                if let Some(result) = result {
                    if result.outcome.is_healthy() {
                        row.healthy += 1;
                    } else {
                        row.failed += 1;
                        row.first_failure.get_or_insert((check, result));
                    }
                }
            } else {
                standalone.push(check);
            }
        }
        let query = self.server_query.trim().to_lowercase();
        rows.retain(|row| {
            (!self.failed_only || row.failed > 0)
                && (query.is_empty()
                    || row.server.name.to_lowercase().contains(&query)
                    || row.server.id.to_lowercase().contains(&query)
                    || row
                        .server
                        .base_url
                        .as_ref()
                        .is_some_and(|url| url.as_str().to_lowercase().contains(&query)))
        });
        rows.sort_by_key(|row| std::cmp::Reverse(row.failed));
        let theme = self.theme;
        let mut view = div().flex().flex_col().gap_3();
        if !self.config.servers.is_empty() {
            view = view.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .child("Serverstatus"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .when(self.server_layout != ServerLayout::ServerList, |toolbar| {
                                toolbar.child(self.server_search(cx))
                            })
                            .child(
                                button(
                                    theme,
                                    "failed-only",
                                    if self.failed_only {
                                        "✓ Nur Fehler"
                                    } else {
                                        "Nur Fehler"
                                    },
                                    self.failed_only,
                                    true,
                                )
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, _, cx| {
                                        this.failed_only = !this.failed_only;
                                        cx.notify();
                                    },
                                )),
                            )
                            .children(
                                [
                                    (
                                        ServerLayout::ServerList,
                                        "layout-server-list",
                                        "Serverliste",
                                    ),
                                    (ServerLayout::Matrix, "layout-matrix", "Matrix"),
                                    (ServerLayout::Cards, "layout-cards", "Kacheln"),
                                ]
                                .map(|(layout, id, label)| {
                                    let active = self.server_layout == layout;
                                    button(theme, id, label, active, true).on_click(cx.listener(
                                        move |this, _: &ClickEvent, _, cx| {
                                            if this.server_layout != layout {
                                                this.server_layout = layout;
                                                this.selected_card = None;
                                                this.save_appearance();
                                                cx.notify();
                                            }
                                        },
                                    ))
                                }),
                            ),
                    ),
            );
            view = view.child(if self.server_layout == ServerLayout::ServerList {
                self.server_list(&rows, window, cx)
            } else if rows.is_empty() {
                div()
                    .rounded_md()
                    .bg(rgb(theme.surface))
                    .p_4()
                    .text_sm()
                    .text_color(rgb(theme.muted))
                    .child(if query.is_empty() {
                        "Keine Server mit fehlgeschlagenen Checks."
                    } else {
                        "Keine passenden Server gefunden."
                    })
            } else {
                if self.server_layout == ServerLayout::Matrix {
                    self.server_matrix(&columns, &rows)
                } else {
                    self.server_cards(&columns, &rows, window, cx)
                }
            });
        }
        if !standalone.is_empty() {
            view = view.child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(theme.border))
                    .bg(rgb(theme.surface))
                    .child(
                        div()
                            .bg(rgb(theme.panel))
                            .px_4()
                            .py_2()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Einzelne Endpunkte"),
                    )
                    .child(
                        div()
                            .id("standalone-scroll")
                            .overflow_x_scroll()
                            .child(self.service_table(standalone.into_iter(), cx)),
                    ),
            );
        }
        view
    }

    fn server_search(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let empty = self.server_query.is_empty();
        div()
            .id("server-search")
            .track_focus(&self.server_search_focus)
            .tab_index(0)
            .w(px(190.0))
            .h(px(34.0))
            .flex()
            .items_center()
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(rgb(theme.border))
            .in_focus(|style| style.border_color(rgb(theme.accent)))
            .bg(rgb(theme.surface))
            .cursor_text()
            .text_sm()
            .text_color(rgb(if empty { theme.muted } else { theme.text }))
            .child(div().whitespace_nowrap().overflow_hidden().child(if empty {
                "Server suchen".into()
            } else {
                self.server_query.clone()
            }))
            .on_click(cx.listener(|this, _: &ClickEvent, window, _| {
                this.server_search_focus.focus(window);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let stroke = &event.keystroke;
                match stroke.key.as_str() {
                    "backspace" => {
                        this.server_query.pop();
                    }
                    "space" => this.server_query.push(' '),
                    "escape" => {
                        this.server_query.clear();
                        window.blur();
                    }
                    "v" if stroke.modifiers.control || stroke.modifiers.platform => {
                        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                            this.server_query
                                .extend(text.chars().filter(|ch| !ch.is_control()));
                        }
                    }
                    _ => {
                        if let Some(text) = &stroke.key_char {
                            this.server_query
                                .extend(text.chars().filter(|ch| !ch.is_control()));
                        } else {
                            return;
                        }
                    }
                }
                cx.stop_propagation();
                cx.notify();
            }))
    }

    fn server_matrix(&self, columns: &[(&str, &str)], rows: &[ServerRow<'_>]) -> Div {
        let theme = self.theme;
        let header = div()
            .flex()
            .bg(rgb(theme.panel))
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .child(div().w(px(190.0)).flex_none().px_3().py_2().child("Server"))
            .children(columns.iter().map(|(_, name)| {
                matrix_cell(theme)
                    .px_2()
                    .py_2()
                    .text_center()
                    .child((*name).to_owned())
            }))
            .child(matrix_total(theme, "Ergebnis".into()));
        let table = div()
            .min_w(px(274.0 + columns.len() as f32 * 108.0))
            .flex()
            .flex_col()
            .child(header)
            .children(rows.iter().map(|row| {
                let total_color = if row.failed > 0 {
                    theme.error
                } else if row.healthy > 0 && row.healthy == row.checks.len() {
                    theme.success
                } else {
                    theme.muted
                };
                div()
                    .flex()
                    .min_h(px(30.0))
                    .border_t_1()
                    .border_color(rgb(theme.border))
                    .child(
                        div()
                            .w(px(190.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .px_3()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(div().truncate().child(row.server.name.clone())),
                    )
                    .children(columns.iter().map(|(id, _)| {
                        let cell = matrix_cell(theme);
                        let Some(check) = row.checks.get(id) else {
                            return cell.text_color(rgb(theme.muted)).child("·");
                        };
                        let result = self.check_result(check);
                        let (symbol, color) = outcome_mark(theme, result);
                        cell.text_color(rgb(color))
                            .font_weight(FontWeight::BOLD)
                            .when(
                                result.is_some_and(|result| !result.outcome.is_healthy()),
                                |cell| cell.bg(rgb(theme.error_soft)),
                            )
                            .child(symbol)
                    }))
                    .child(matrix_total(theme, row.ratio()).text_color(rgb(total_color)))
            }));
        div()
            .rounded_md()
            .border_1()
            .border_color(rgb(theme.border))
            .bg(rgb(theme.surface))
            .child(div().id("matrix-scroll").overflow_x_scroll().child(table))
    }

    fn server_cards(
        &self,
        columns: &[(&str, &str)],
        rows: &[ServerRow<'_>],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = self.theme;
        let width = window.viewport_size().width;
        let columns_per_row = if width >= px(1380.0) {
            4
        } else if width >= px(1020.0) {
            3
        } else {
            2
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(rows.chunks(columns_per_row).map(|chunk| {
                div()
                    .flex()
                    .gap_3()
                    .children(chunk.iter().map(|row| {
                        let server_id = row.server.id.clone();
                        let selected = self.selected_card.as_deref() == Some(server_id.as_str());
                        let border = if selected {
                            theme.accent
                        } else if row.failed > 0 {
                            theme.error_mark
                        } else {
                            theme.border
                        };
                        div()
                            .id(SharedString::from(format!("server-card:{server_id}")))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(border))
                            .bg(rgb(if row.failed > 0 {
                                theme.error_soft
                            } else {
                                theme.surface
                            }))
                            .p_3()
                            .when(selected, |card| with_glow(card, theme, 11.0, 0.24))
                            .hover(|style| style.bg(rgb(theme.panel)))
                            .cursor_pointer()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(row.server.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(if row.failed > 0 {
                                                theme.error
                                            } else {
                                                theme.muted
                                            }))
                                            .child(row.ratio()),
                                    )
                                    .child(div().text_color(rgb(theme.muted)).child("›")),
                            )
                            .child(div().flex().flex_wrap().gap_1().children(
                                columns.iter().filter_map(|(id, _)| {
                                    let check = row.checks.get(id)?;
                                    let (symbol, color) =
                                        outcome_mark(theme, self.check_result(check));
                                    Some(
                                        div()
                                            .max_w_full()
                                            .truncate()
                                            .rounded_sm()
                                            .bg(rgb(theme.background))
                                            .px_1()
                                            .text_xs()
                                            .text_color(rgb(color))
                                            .child(format!("{symbol} {}", check.request.name)),
                                    )
                                }),
                            ))
                            .when_some(row.first_failure, |card, (check, result)| {
                                let extra = if row.failed > 1 {
                                    format!(" · +{}", row.failed - 1)
                                } else {
                                    String::new()
                                };
                                card.child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(rgb(theme.error))
                                        .child(format!(
                                            "{} · {}{extra}",
                                            check.request.name,
                                            result.outcome.label()
                                        )),
                                )
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let already_selected =
                                    this.selected_card.as_deref() == Some(server_id.as_str());
                                this.selected_card = (!already_selected).then(|| server_id.clone());
                                cx.notify();
                            }))
                    }))
                    .children((chunk.len()..columns_per_row).map(|_| div().flex_1()))
            }))
    }

    pub(super) fn server_details(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.server_layout != ServerLayout::Cards {
            return None;
        }
        let selected_id = self.selected_card.as_deref()?;
        let server = self
            .config
            .servers
            .iter()
            .find(|server| server.id == selected_id)?;
        let server_id = server.id.clone();
        let theme = self.theme;
        let mut checks = self
            .config
            .checks
            .iter()
            .filter(|check| check.key.server.as_deref() == Some(&server.id))
            .peekable();
        let body = if checks.peek().is_some() {
            self.service_table(checks, cx)
        } else {
            div()
                .px_4()
                .py_3()
                .text_sm()
                .child("Keine Checks eingerichtet.")
        };

        Some(
            div()
                .flex_none()
                .border_t_1()
                .border_color(rgb(theme.border))
                .bg(rgb(theme.surface))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(server.name.clone()),
                        )
                        .child(
                            button(theme, "details-copy", "Server kopieren", false, true).on_click(
                                cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let block = this.config.copy_server_toml(&server_id);
                                    this.copy_to_clipboard(
                                        block,
                                        "Server und Checks als TOML kopiert",
                                        cx,
                                    );
                                }),
                            ),
                        )
                        .child(
                            button(theme, "details-close", "Schließen", false, true).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.selected_card = None;
                                    cx.notify();
                                }),
                            ),
                        ),
                )
                .child(
                    div()
                        .id("server-details-scroll")
                        .max_h(px(190.0))
                        .overflow_scroll()
                        .child(body),
                ),
        )
    }

    fn check_result(&self, check: &Check) -> Option<&CheckResult> {
        self.services.get(&check.key)?.last.as_ref()
    }
}

fn matrix_cell(theme: &Theme) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .justify_center()
        .border_l_1()
        .border_color(rgb(theme.border))
}

fn matrix_total(theme: &Theme, label: String) -> Div {
    div()
        .w(px(84.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .border_l_1()
        .border_color(rgb(theme.border))
        .text_sm()
        .child(label)
}

fn outcome_mark(theme: &Theme, result: Option<&CheckResult>) -> (&'static str, u32) {
    match result {
        Some(result) if result.outcome.is_healthy() => ("✓", theme.success),
        Some(_) => ("×", theme.error),
        None => ("…", theme.warning),
    }
}
