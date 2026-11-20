use super::{Dashboard, ServerRow, button, outcome_mark};
use crate::dashboard::dot;
use gpui::{ClickEvent, Context, Div, FontWeight, SharedString, Window, div, prelude::*, px, rgb};
use pulseboard::config::Check;

impl Dashboard {
    pub(super) fn server_list(
        &self,
        rows: &[ServerRow<'_>],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = self.theme;
        let selected = rows
            .iter()
            .find(|row| self.selected_server.as_deref() == Some(row.server.id.as_str()))
            .or_else(|| rows.first());
        type GroupFilter = fn(&ServerRow<'_>) -> bool;
        let groups: [(&str, &str, u32, GroupFilter); 3] = [
            ("Mit Fehlern", "×", theme.error, |row: &ServerRow<'_>| {
                row.failed > 0
            }),
            (
                "Betriebsbereit",
                "✓",
                theme.success,
                |row: &ServerRow<'_>| {
                    row.failed == 0 && row.healthy > 0 && row.healthy == row.checks.len()
                },
            ),
            ("Ausstehend", "…", theme.warning, |row: &ServerRow<'_>| {
                row.failed == 0 && (row.healthy == 0 || row.healthy < row.checks.len())
            }),
        ];

        let sidebar =
            div()
                .flex()
                .flex_col()
                .w(px(245.0))
                .flex_none()
                .bg(rgb(theme.surface))
                .border_r_1()
                .border_color(rgb(theme.border))
                .child(
                    div()
                        .flex_none()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::BOLD)
                                .child("Server"),
                        )
                        .child(self.server_search(cx)),
                )
                .child(
                    div()
                        .id("server-list-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_3()
                        .pb_3()
                        .children(groups.into_iter().filter_map(
                            |(label, mark, color, includes)| {
                                let matching: Vec<_> =
                                    rows.iter().filter(|row| includes(row)).collect();
                                if matching.is_empty() {
                                    return None;
                                }
                                Some(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .mb_3()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .px_2()
                                                .py_1()
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(dot(color))
                                                .child(format!("{label} ({})", matching.len())),
                                        )
                                        .children(matching.into_iter().map(|row| {
                                            let id = row.server.id.clone();
                                            let active = selected
                                                .is_some_and(|selected| selected.server.id == id);
                                            div()
                                                .id(SharedString::from(format!("server-list:{id}")))
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .px_2()
                                                .py_1()
                                                .rounded_md()
                                                .border_1()
                                                .border_color(rgb(if active {
                                                    theme.accent
                                                } else {
                                                    theme.surface
                                                }))
                                                .when(active, |row| row.bg(rgb(theme.accent_soft)))
                                                .hover(|style| style.bg(rgb(theme.panel)))
                                                .cursor_pointer()
                                                .text_sm()
                                                .child(div().text_color(rgb(color)).child(mark))
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .child(row.server.name.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_color(rgb(theme.muted))
                                                        .child(row.ratio()),
                                                )
                                                .on_click(cx.listener(
                                                    move |this, _: &ClickEvent, _, cx| {
                                                        this.selected_server = Some(id.clone());
                                                        this.expanded_check = None;
                                                        cx.notify();
                                                    },
                                                ))
                                        })),
                                )
                            },
                        )),
                );

        div()
            .flex()
            .h((window.viewport_size().height - px(270.0)).max(px(310.0)))
            .rounded_md()
            .border_1()
            .border_color(rgb(theme.border))
            .overflow_hidden()
            .child(sidebar)
            .child(
                div()
                    .id("selected-server-scroll")
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scroll()
                    .p_4()
                    .child(selected.map_or_else(
                        || {
                            div().text_sm().text_color(rgb(theme.muted)).child(
                                if self.failed_only && self.server_query.trim().is_empty() {
                                    "Keine Server mit fehlgeschlagenen Checks."
                                } else {
                                    "Keine passenden Server gefunden."
                                },
                            )
                        },
                        |row| self.selected_server_view(row, cx),
                    )),
            )
    }

    fn selected_server_view(&self, row: &ServerRow<'_>, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let id = row.server.id.clone();
        let check_count = row.checks.len();
        let summary = format!("{} von {check_count} Checks erfolgreich", row.healthy);
        let meta = row.server.base_url.as_ref().map_or(summary.clone(), |url| {
            format!("{} · {summary}", url.origin().ascii_serialization())
        });
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(row.server.name.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(theme.muted))
                            .truncate()
                            .child(meta),
                    ),
            )
            .child(
                button(theme, "copy-listed-server", "Server kopieren", false, true).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let block = this.config.copy_server_toml(&id);
                        this.copy_to_clipboard(block, "Server und Checks als TOML kopiert", cx);
                    }),
                ),
            );
        let columns = div()
            .flex()
            .gap_2()
            .items_center()
            .px_3()
            .py_2()
            .bg(rgb(theme.panel))
            .rounded_md()
            .text_xs()
            .text_color(rgb(theme.muted))
            .child(div().flex_1().child("Check"))
            .child(div().w(px(58.0)).flex_none().child("Methode"))
            .child(div().w(px(105.0)).flex_none().child("HTTP"))
            .child(div().w(px(66.0)).flex_none().child("Dauer"))
            .child(div().w(px(16.0)));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(header)
            .when(check_count == 0, |view| {
                view.child(div().text_sm().child("Keine Checks eingerichtet."))
            })
            .when(check_count > 0, |view| {
                view.child(columns).children(
                    self.config
                        .checks
                        .iter()
                        .filter(|check| check.key.server.as_deref() == Some(row.server.id.as_str()))
                        .map(|check| self.server_list_check(check, cx)),
                )
            })
    }

    fn server_list_check(&self, check: &Check, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let result = self.check_result(check);
        let (mark, color) = outcome_mark(theme, result);
        let failed = result.is_some_and(|result| !result.outcome.is_healthy());
        let open = self.expanded_check.as_ref() == Some(&check.key);
        let key = check.key.clone();
        let server_id = check.key.server.clone();
        let copy_key = key.clone();
        let (http, duration) = result.map_or_else(
            || ("Ausstehend".into(), "—".into()),
            |result| {
                let status = match &result.outcome {
                    pulseboard::monitor::Outcome::Healthy(code)
                    | pulseboard::monitor::Outcome::Unhealthy(code) => code.to_string(),
                    _ => "Fehler".into(),
                };
                (status, format!("{} ms", result.elapsed.as_millis()))
            },
        );
        div()
            .rounded_md()
            .border_1()
            .border_color(rgb(if failed {
                theme.error_mark
            } else {
                theme.border
            }))
            .bg(rgb(if failed {
                theme.error_soft
            } else {
                theme.surface
            }))
            .child(
                div()
                    .id(SharedString::from(format!("listed-check:{}", key.id)))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(theme.panel)))
                    .child(div().text_color(rgb(color)).child(mark))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(check.request.name.clone()),
                    )
                    .child(
                        div()
                            .w(px(58.0))
                            .flex_none()
                            .text_sm()
                            .child(check.request.method.label()),
                    )
                    .child(
                        div()
                            .w(px(105.0))
                            .flex_none()
                            .text_sm()
                            .text_color(rgb(color))
                            .child(http),
                    )
                    .child(
                        div()
                            .w(px(66.0))
                            .flex_none()
                            .text_sm()
                            .text_color(rgb(theme.muted))
                            .child(duration),
                    )
                    .child(div().w(px(16.0)).child(if open { "⌃" } else { "⌄" }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.selected_server.clone_from(&server_id);
                        this.expanded_check = (!open).then(|| key.clone());
                        cx.notify();
                    })),
            )
            .when(open, |row| {
                row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(theme.border))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .child(check.url.path().to_owned())
                                .children(result.map(|result| {
                                    div().text_color(rgb(color)).child(result.outcome.label())
                                })),
                        )
                        .child(
                            button(theme, "copy-listed-check", "Check kopieren", false, true)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let block = this.config.copy_check_toml(&copy_key);
                                    this.copy_to_clipboard(
                                        block,
                                        "Healthcheck als TOML kopiert",
                                        cx,
                                    );
                                })),
                        ),
                )
            })
    }
}
