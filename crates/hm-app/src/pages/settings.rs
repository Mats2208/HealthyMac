//! Settings: how things get removed, where to look, and what has been done so far.

use std::path::PathBuf;

use gpui::{div, prelude::*, px, AnyElement, Context, ElementId, FontWeight, SharedString, Window};
use hm_core::fmt::{human_age, human_size};
use hm_core::{Appearance, CleanStatus, LanguageChoice, RemovalMode};

use super::{plural, scroll_pane};
use crate::assets::Icon;
use crate::state::Shell;
use crate::theme::{self, font, radius, sp, GUTTER};
use crate::ui::chrome::{card, notice, page_header, section_title, NoticeKind};
use crate::ui::controls::{icon_button, pill, segmented, stepper, Btn, BtnSize};
use crate::ui::label::path_label;
use crate::ui::{col, icon, row, TextExt};

impl Shell {
    pub fn render_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let width = (super::content_width(window) - GUTTER * 2.0).min(720.0);
        let subtitle = if self.mode.persists_config() {
            "Se guarda al momento.".to_string()
        } else {
            "En este modo los cambios duran hasta que cierres la app: no se guardan.".to_string()
        };
        let pane = self.settings.scroll.clone();
        col()
            .size_full()
            .child(page_header("Ajustes", subtitle, Vec::new()))
            .child(scroll_pane(
                "settings-scroll",
                &pane,
                col()
                    .w(px(width))
                    .ml(px(GUTTER))
                    .pb(sp(8.))
                    .gap(sp(7.))
                    .when_some(self.config_error.clone(), |this, error| {
                        this.child(notice(
                            NoticeKind::Warning,
                            "Tu configuración no se ha podido leer, así que usamos la de fábrica",
                            Some(error.into()),
                            None,
                        ))
                    })
                    .when_some(self.settings.save_error.clone(), |this, error| {
                        this.child(notice(
                            NoticeKind::Error,
                            "No se ha podido guardar el cambio",
                            Some(error.into()),
                            None,
                        ))
                    })
                    .child(self.render_appearance(cx))
                    .child(self.render_removal_mode(cx))
                    .child(self.render_access(cx))
                    .child(self.render_project_settings(cx))
                    .child(self.render_excluded(cx))
                    .child(self.render_history()),
            ))
            .into_any_element()
    }

    fn render_removal_mode(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.ctx.config.removal_mode;
        section("Al limpiar")
            .child(self.mode_option(
                RemovalMode::Trash,
                mode,
                "Mover a la Papelera",
                "Lo más prudente: todo se puede recuperar. A cambio, el espacio no se libera hasta que vacías la Papelera.",
                cx,
            ))
            .child(self.mode_option(
                RemovalMode::DeleteRegenerable,
                mode,
                "Borrar lo que se regenera solo",
                "Lo seguro que se regenera (cachés, compilaciones) se borra al momento y libera espacio ya. Todo lo demás sigue yendo a la Papelera.",
                cx,
            ))
    }

    fn mode_option(
        &self,
        value: RemovalMode,
        current: RemovalMode,
        title: &'static str,
        detail: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = value == current;
        let accent = theme::primary();
        row()
            .id(SharedString::from(format!("removal-{value:?}")))
            .gap(sp(3.))
            .items_start()
            .p(sp(3.))
            .rounded(px(radius::MD))
            .border_1()
            .cursor_pointer()
            .when(selected, |this| {
                this.bg(theme::surface_1()).border_color(accent.alpha(0.55))
            })
            .when(!selected, |this| {
                this.bg(theme::surface_1())
                    .border_color(theme::hairline())
                    .hover(|style| style.bg(theme::surface_2()))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.update_config(cx, |config| config.removal_mode = value);
            }))
            .child(
                row()
                    .mt(sp(0.5))
                    .flex_none()
                    .size(px(16.))
                    .justify_center()
                    .rounded_full()
                    .when(selected, |this| this.bg(accent))
                    .when(!selected, |this| {
                        this.border_1().border_color(theme::hairline_strong())
                    })
                    .when(selected, |this| {
                        this.child(div().size(px(6.)).rounded_full().bg(theme::on_primary()))
                    }),
            )
            .child(
                col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(sp(0.5))
                    .child(
                        div()
                            .fs(font::BODY)
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .fs(font::CAPTION)
                            .text_color(theme::text_2())
                            .child(detail),
                    ),
            )
    }

    fn render_access(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let access = self.settings.full_disk_access;
        section("Acceso total al disco").child(
            card()
                .w_full()
                .p(sp(3.5))
                .child(
                    row()
                        .gap(sp(3.))
                        .items_start()
                        .child(icon(
                            if access == Some(true) { Icon::Shield } else { Icon::Lock },
                            18.0,
                            if access == Some(true) { theme::green() } else { theme::amber() },
                        ))
                        .child(
                            col()
                                .flex_1()
                                .min_w(px(0.))
                                .gap(sp(0.5))
                                .child(
                                    div().fs(font::BODY).font_weight(FontWeight::MEDIUM).child(match access {
                                        Some(true) => "Concedido",
                                        Some(false) => "Sin conceder",
                                        None => "Comprobando…",
                                    }),
                                )
                                .child(
                                    div()
                                        .fs(font::CAPTION)
                                        .text_color(theme::text_2())
                                        .child(match access {
                                            Some(true) => "HealthyMac ve todo el disco: Documentos, Descargas, Escritorio y los datos de las apps. Sigue sin borrar nada que no le pidas.",
                                            _ => "Sin este permiso no miramos Documentos, Descargas, Escritorio ni los datos internos de las apps, así que el mapa de disco y los archivos grandes se quedan cortos. Con él lo vemos todo, y sigue sin borrarse nada que no pidas. Se concede en Ajustes del Sistema y se puede retirar cuando quieras.",
                                        }),
                                ),
                        )
                        .when(access != Some(true), |this| {
                            this.child(
                                Btn::secondary("Abrir Ajustes")
                                    .size(BtnSize::Sm)
                                    .icon(Icon::External)
                                    .build(
                                        "fda-open",
                                        cx.listener(|_, _, _, cx| cx.open_url(super::FULL_DISK_ACCESS_URL)),
                                    ),
                            )
                        }),
                ),
        )
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let labels: [SharedString; 3] = ["Sistema".into(), "Claro".into(), "Oscuro".into()];
        let selected = match self.ctx.config.appearance {
            Appearance::System => 0,
            Appearance::Light => 1,
            Appearance::Dark => 2,
        };
        let entity = cx.entity().downgrade();
        let languages: [SharedString; 3] = ["Sistema".into(), "Español".into(), "English".into()];
        let language = match self.ctx.config.language {
            LanguageChoice::System => 0,
            LanguageChoice::Spanish => 1,
            LanguageChoice::English => 2,
        };
        let language_entity = cx.entity().downgrade();
        section("Apariencia")
            .child(setting_row(
                "Idioma",
                "Sistema usa el primer idioma de macOS. Lo ya escaneado cambia al volver a escanear.",
                segmented("language", &languages, language, move |index, window, cx| {
                    let value = match index {
                        1 => LanguageChoice::Spanish,
                        2 => LanguageChoice::English,
                        _ => LanguageChoice::System,
                    };
                    language_entity
                        .update(cx, |shell, cx| shell.set_language_choice(value, window, cx))
                        .ok();
                })
                .into_any_element(),
            ))
            .child(setting_row(
            "Tema",
            "Sistema sigue el modo claro u oscuro de macOS.",
            segmented("appearance", &labels, selected, move |index, window, cx| {
                let value = match index {
                    1 => Appearance::Light,
                    2 => Appearance::Dark,
                    _ => Appearance::System,
                };
                entity
                    .update(cx, |shell, cx| shell.set_appearance(value, window, cx))
                    .ok();
            })
            .into_any_element(),
        ))
    }

    fn render_project_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let days = self.ctx.config.project_idle_days;
        let min_large = self.ctx.config.large_file_min_bytes;
        let roots = self.ctx.config.project_roots.clone();
        let effective = self.ctx.config.effective_project_roots(&self.ctx.home);
        section("Proyectos y archivos")
            .child(setting_row(
                "Un proyecto está parado tras",
                "A partir de ahí se marca como parado en Proyectos.",
                stepper(
                    "idle-days",
                    if days == 1 {
                        "1 día".to_string()
                    } else {
                        format!("{days} días")
                    },
                    cx.listener(|this, _, _, cx| {
                        this.update_config(cx, |config| {
                            config.project_idle_days =
                                config.project_idle_days.saturating_sub(1).max(1);
                        });
                        this.rebuild_project_rows();
                    }),
                    cx.listener(|this, _, _, cx| {
                        this.update_config(cx, |config| {
                            config.project_idle_days = (config.project_idle_days + 1).min(365);
                        });
                        this.rebuild_project_rows();
                    }),
                )
                .into_any_element(),
            ))
            .child(setting_row(
                "Un archivo es grande desde",
                "Desde ese tamaño aparece en Archivos. Se aplica en el siguiente escaneo.",
                stepper(
                    "large-file",
                    human_size(min_large),
                    cx.listener(|this, _, _, cx| {
                        this.update_config(cx, |config| {
                            config.large_file_min_bytes =
                                (config.large_file_min_bytes / 2).max(100_000_000);
                        });
                    }),
                    cx.listener(|this, _, _, cx| {
                        this.update_config(cx, |config| {
                            config.large_file_min_bytes =
                                (config.large_file_min_bytes * 2).min(64_000_000_000);
                        });
                    }),
                )
                .into_any_element(),
            ))
            .child(path_list(
                "Carpetas de proyectos",
                if roots.is_empty() {
                    "Miramos en las carpetas habituales. Añade las tuyas para mirar solo en ellas."
                } else {
                    "Solo miramos en estas."
                },
                "project-root",
                if roots.is_empty() { &effective } else { &roots },
                !roots.is_empty(),
                cx,
                |config, path| config.project_roots.retain(|root| root != path),
            ))
            .child(
                row().child(
                    Btn::secondary("Añadir carpeta…")
                        .size(BtnSize::Sm)
                        .icon(Icon::Plus)
                        .build(
                            "add-root",
                            cx.listener(|this, _, _, cx| {
                                this.pick_paths(cx, |config, paths| {
                                    for path in paths {
                                        if !config.project_roots.contains(&path) {
                                            config.project_roots.push(path);
                                        }
                                    }
                                });
                            }),
                        ),
                ),
            )
    }

    fn render_excluded(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let excluded = self.ctx.config.excluded_paths.clone();
        section("Nunca tocar")
            .child(path_list(
                "Rutas excluidas",
                if excluded.is_empty() {
                    "Nada excluido. Lo que añadas aquí queda fuera de cualquier limpieza, pase lo que pase."
                } else {
                    "Quedan fuera de cualquier limpieza, pase lo que pase."
                },
                "excluded",
                &excluded,
                true,
                cx,
                |config, path| config.excluded_paths.retain(|item| item != path),
            ))
            .child(row().child(Btn::secondary("Excluir una ruta…").size(BtnSize::Sm).icon(Icon::Plus).build(
                "add-excluded",
                cx.listener(|this, _, _, cx| {
                    this.pick_paths(cx, |config, paths| {
                        for path in paths {
                            if !config.excluded_paths.contains(&path) {
                                config.excluded_paths.push(path);
                            }
                        }
                    });
                }),
            )))
    }

    /// Asks macOS for folders, then applies them to the config.
    fn pick_paths(
        &mut self,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut hm_core::Config, Vec<PathBuf>) + 'static,
    ) {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Añadir".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                this.update(cx, |shell, cx| {
                    shell.update_config(cx, |config| apply(config, paths));
                })
                .ok();
            }
        })
        .detach();
    }

    fn render_history(&self) -> impl IntoElement {
        let now = std::time::SystemTime::now();
        let entries = self.settings.history.data.as_deref().unwrap_or(&[]);
        section("Historial de limpieza").child(if entries.is_empty() {
            div()
                .fs(font::CAPTION)
                .text_color(theme::text_2())
                .child(match &self.settings.history.error {
                    Some(error) => format!("No hemos podido leer el historial: {error}"),
                    None => "Todavía no has limpiado nada. Aquí quedará constancia de todo lo que hagas.".to_string(),
                })
                .into_any_element()
        } else {
            let trashed: u64 = entries.iter().map(|entry| entry.outcome.trashed_bytes).sum();
            let freed: u64 = entries.iter().map(|entry| entry.outcome.freed_bytes).sum();
            col()
                .gap(sp(2.))
                .child(
                    div()
                        .fs(font::CAPTION)
                        .text_color(theme::text_2())
                        .child(format!(
                            "{}: {} liberados y {} enviados a la Papelera.",
                            plural(entries.len(), "acción", "acciones"),
                            human_size(freed),
                            human_size(trashed)
                        )),
                )
                .child(
                    col().children(entries.iter().take(60).enumerate().map(|(index, entry)| {
                        let (label, color) = match entry.outcome.status {
                            CleanStatus::Done | CleanStatus::WouldClean => ("Hecho", theme::green()),
                            CleanStatus::Partial => ("A medias", theme::amber()),
                            CleanStatus::Skipped => ("Omitido", theme::text_3()),
                            CleanStatus::Failed => ("Falló", theme::rose()),
                        };
                        let bytes = entry.outcome.freed_bytes + entry.outcome.trashed_bytes;
                        row()
                            .id(ElementId::NamedInteger("history".into(), index as u64))
                            .h(px(32.))
                            .gap(sp(2.5))
                            .border_b_1()
                            .border_color(theme::hairline())
                            .child(crate::ui::clip().flex_1().fs(font::CAPTION).child(entry.title.clone()))
                            .child(pill(label, color))
                            .child(
                                div()
                                    .w(px(88.))
                                    .text_right()
                                    .fs(font::CAPTION)
                                    .whitespace_nowrap()
                                    .text_color(theme::text_3())
                                    .child(human_age(entry.at, now)),
                            )
                            .child(
                                div()
                                    .w(px(64.))
                                    .text_right()
                                    .fs(font::CAPTION)
                                    .tabular()
                                    .child(if bytes > 0 { human_size(bytes) } else { "—".to_string() }),
                            )
                    })),
                )
                .into_any_element()
        })
    }
}

fn section(title: &'static str) -> gpui::Div {
    col().w_full().gap(sp(2.5)).child(section_title(title))
}

fn setting_row(title: &'static str, detail: &'static str, control: AnyElement) -> impl IntoElement {
    row()
        .w_full()
        .gap(sp(4.))
        .py(sp(1.))
        .child(
            col()
                .flex_1()
                .min_w(px(0.))
                .child(div().fs(font::BODY).child(title))
                .child(
                    div()
                        .fs(font::CAPTION)
                        .text_color(theme::text_2())
                        .child(detail),
                ),
        )
        .child(control)
}

fn path_list(
    title: &'static str,
    detail: &'static str,
    id: &'static str,
    paths: &[PathBuf],
    removable: bool,
    cx: &mut Context<Shell>,
    remove: impl Fn(&mut hm_core::Config, &PathBuf) + Clone + 'static,
) -> impl IntoElement {
    col()
        .w_full()
        .gap(sp(1.5))
        .mt(sp(1.))
        .child(
            col().child(div().fs(font::BODY).child(title)).child(
                div()
                    .fs(font::CAPTION)
                    .text_color(theme::text_2())
                    .child(detail),
            ),
        )
        .when(!paths.is_empty(), |this| {
            this.child(
                card()
                    .w_full()
                    .py(sp(1.))
                    .children(paths.iter().enumerate().map(|(index, path)| {
                        let target = path.clone();
                        let remove = remove.clone();
                        row()
                            .h(px(32.))
                            .px(sp(2.5))
                            .gap(sp(2.))
                            .child(icon(Icon::Folder, 14.0, theme::text_3()))
                            .child(div().flex_1().min_w(px(0.)).child(path_label(
                                path.display().to_string(),
                                font::CAPTION,
                                theme::text_1(),
                            )))
                            .when(removable, |this| {
                                this.child(icon_button(
                                    ElementId::NamedInteger(id.into(), index as u64),
                                    Icon::X,
                                    theme::text_3(),
                                    cx.listener(move |this, _, _, cx| {
                                        let remove = remove.clone();
                                        let target = target.clone();
                                        this.update_config(cx, move |config| {
                                            remove(config, &target)
                                        });
                                    }),
                                ))
                            })
                    })),
            )
        })
}
