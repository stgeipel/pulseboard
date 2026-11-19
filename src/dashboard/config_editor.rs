use super::{Dashboard, button};
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FontWeight, KeyBinding, SharedString,
    Subscription, Task, Window, actions, div, prelude::*, rgb,
};
use gpui_component::{
    ThemeMode,
    highlighter::{Diagnostic, DiagnosticSeverity, LanguageConfig, LanguageRegistry},
    input::{Input, InputEvent, InputState, RopeExt},
};
use pulseboard::{
    config::{self, DocumentError},
    monitor::Command,
};
use std::{fs, ops::Range, time::Duration};

actions!(config_editor, [SaveConfiguration, CloseConfiguration]);

pub(crate) fn init(cx: &mut App) {
    gpui_component::init(cx);
    LanguageRegistry::singleton().register(
        "toml",
        &LanguageConfig::new(
            "toml",
            tree_sitter_toml_ng::LANGUAGE.into(),
            Vec::new(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    );
    let save_key = if cfg!(target_os = "macos") {
        "cmd-s"
    } else {
        "ctrl-s"
    };
    cx.bind_keys([
        KeyBinding::new(save_key, SaveConfiguration, Some("ConfigEditor")),
        KeyBinding::new("escape", CloseConfiguration, Some("ConfigEditor")),
    ]);
}

pub(super) struct ConfigEditor {
    input: Entity<InputState>,
    original: String,
    diagnostics: Option<String>,
    save_error: Option<String>,
    validating: bool,
    dirty: bool,
    confirm_discard: bool,
    validation: Option<Task<()>>,
    _subscription: Subscription,
}

impl Dashboard {
    pub(super) fn open_config_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = &self.path else { return };
        let original = match config::ensure_file(path).and_then(|()| {
            fs::read_to_string(path).map_err(|error| {
                anyhow::anyhow!("Konfiguration konnte nicht gelesen werden. {error}")
            })
        }) {
            Ok(original) => original,
            Err(error) => {
                self.action_error = Some(format!("{error:#}"));
                cx.notify();
                return;
            }
        };
        let mode = if self.theme.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        gpui_component::Theme::change(mode, Some(window), cx);
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("toml")
                .line_number(true)
                .default_value(original.clone())
        });
        let subscription = cx.subscribe_in(&input, window, |this, input, event, window, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let Some(editor) = &mut this.config_editor else {
                return;
            };
            let value = input.read(cx).value();
            editor.dirty = value.as_str() != editor.original;
            editor.confirm_discard = false;
            editor.save_error = None;
            editor.validating = true;
            editor.validation = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(220))
                    .await;
                let _ = this.update_in(cx, |this, _, cx| {
                    if let Some(editor) = &mut this.config_editor {
                        editor.validate(&value, cx);
                        cx.notify();
                    }
                });
            }));
            cx.notify();
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let mut editor = ConfigEditor {
            input,
            original,
            diagnostics: None,
            save_error: None,
            validating: false,
            dirty: false,
            confirm_discard: false,
            validation: None,
            _subscription: subscription,
        };
        let value = editor.input.read(cx).value();
        editor.validate(&value, cx);
        self.theme_cursor = None;
        self.config_editor = Some(editor);
        cx.notify();
    }

    fn close_config_editor(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.config_editor
            && editor.dirty
            && !editor.confirm_discard
        {
            editor.confirm_discard = true;
        } else {
            self.config_editor = None;
        }
        cx.notify();
    }

    fn save_config_editor(&mut self, cx: &mut Context<Self>) {
        let (Some(path), Some(editor)) = (&self.path, &mut self.config_editor) else {
            return;
        };
        let text = editor.input.read(cx).value();
        editor.validation = None;
        editor.validate(&text, cx);
        editor.confirm_discard = false;
        if editor.diagnostics.is_some() {
            cx.notify();
            return;
        }
        if let Err(error) = config::save_edit(path, &editor.original, &text) {
            editor.save_error = Some(format!("{error:#}"));
            cx.notify();
            return;
        }

        self.config_editor = None;
        self.action_error = None;
        if let Some(channel) = self.commands.clone() {
            // A reload can wait behind a running batch or an already queued command.
            cx.spawn(async move |this, cx| {
                if channel.send(Command::Reload).await.is_err() {
                    let _ = this.update(cx, |this, cx| {
                        this.action_error =
                            Some("Konfiguration gespeichert. Übernahme fehlgeschlagen.".into());
                        cx.notify();
                    });
                }
            })
            .detach();
        } else {
            self.action_error =
                Some("Konfiguration gespeichert. Der Prüfdienst ist nicht verfügbar.".into());
        }
        cx.notify();
    }

    pub(super) fn render_config_editor(
        &self,
        editor: &ConfigEditor,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let (status, color) = if editor.confirm_discard {
            ("Ungespeicherte Änderungen verwerfen?".into(), theme.warning)
        } else if let Some(error) = &editor.save_error {
            (error.clone(), theme.error)
        } else if editor.validating {
            ("Konfiguration wird geprüft …".into(), theme.muted)
        } else if let Some(error) = &editor.diagnostics {
            (error.clone(), theme.error)
        } else {
            ("Konfiguration gültig".into(), theme.success)
        };
        div()
            .id("config-editor")
            .key_context("ConfigEditor")
            .on_action(
                cx.listener(|this, _: &SaveConfiguration, _, cx| this.save_config_editor(cx)),
            )
            .on_action(
                cx.listener(|this, _: &CloseConfiguration, _, cx| this.close_config_editor(cx)),
            )
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_5()
            .bg(rgb(theme.background))
            .text_color(rgb(theme.text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Konfiguration bearbeiten"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(theme.muted))
                            .child(if editor.dirty {
                                "config.toml · Ungespeichert"
                            } else {
                                "config.toml"
                            }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(theme.border))
                    .child(Input::new(&editor.input).h_full()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(rgb(color))
                            .child(status),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                button(
                                    theme,
                                    "editor-close",
                                    if editor.confirm_discard {
                                        "Änderungen verwerfen"
                                    } else {
                                        "Zurück"
                                    },
                                    false,
                                    true,
                                )
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, _, cx| this.close_config_editor(cx),
                                )),
                            )
                            .child(
                                button(
                                    theme,
                                    "editor-save",
                                    "Speichern und anwenden",
                                    true,
                                    editor.validating || editor.diagnostics.is_none(),
                                )
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, _, cx| this.save_config_editor(cx),
                                )),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl ConfigEditor {
    fn validate(&mut self, value: &SharedString, cx: &mut Context<Dashboard>) {
        self.validating = false;
        let error = config::parse(value).err();
        self.diagnostics = error.as_ref().map(|error| format!("{error:#}"));
        let span = error
            .as_ref()
            .and_then(|error| error.downcast_ref::<DocumentError>())
            .and_then(|error| error.span.clone())
            .and_then(|span| marker_range(value, span));
        self.input.update(cx, |input, cx| {
            let diagnostic = span.zip(self.diagnostics.as_ref()).map(|(span, message)| {
                let range = input.text().offset_to_position(span.start)
                    ..input.text().offset_to_position(span.end);
                Diagnostic::new(range, message.clone()).with_severity(DiagnosticSeverity::Error)
            });
            if let Some(diagnostics) = input.diagnostics_mut() {
                diagnostics.clear();
                if let Some(diagnostic) = diagnostic {
                    diagnostics.push(diagnostic);
                }
            }
            cx.notify();
        });
    }
}

fn marker_range(text: &str, span: Range<usize>) -> Option<Range<usize>> {
    if text
        .get(span.clone())
        .is_some_and(|fragment| fragment.chars().any(|ch| ch != '\n' && ch != '\r'))
    {
        return Some(span);
    }
    let start = span.start.min(text.len());
    let prefix = text.get(..start)?;
    if let Some(next) = text
        .get(start..)?
        .chars()
        .next()
        .filter(|ch| *ch != '\n' && *ch != '\r')
    {
        return Some(start..start + next.len_utf8());
    }
    prefix
        .trim_end_matches(['\n', '\r'])
        .char_indices()
        .next_back()
        .map(|(index, ch)| index..index + ch.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::marker_range;

    #[test]
    fn marks_a_visible_character_at_end_of_input() {
        assert_eq!(marker_range("ä", 2..2), Some(0..2));
        assert_eq!(marker_range("a\n", 1..1), Some(0..1));
        assert_eq!(marker_range("a\n", 2..2), Some(0..1));
        assert_eq!(marker_range("ä", 0..0), Some(0..2));
        assert_eq!(marker_range("", 0..0), None);
    }
}
