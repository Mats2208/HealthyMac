//! Installed apps: what they weigh with their leftovers, and how to get rid of them properly.

use gpui::{
    div, img, prelude::*, px, uniform_list, AnyElement, Context, ElementId, FontWeight, ObjectFit,
    SharedString, StyledImage, Window,
};
use hm_core::fmt::{human_age, human_size};
use hm_core::Area;
use hm_system::types::AppArch;

use super::{content_width, gutter_row, plural};
use crate::assets::Icon;
use crate::state::{AppSort, AppsTab, Shell};
use crate::theme::{self, font, radius, sp, GUTTER};
use crate::ui::chrome::{empty_state, notice, page_header, NoticeKind};
use crate::ui::controls::{dot, pill, segmented, tag, Btn, BtnSize};
use crate::ui::scrollbar::{scrollbar, ScrollSource};
use crate::ui::{clip, col, row, TextExt};

const ROW_HEIGHT: f32 = 56.0;

impl Shell {
    pub fn render_apps(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let leftovers_count = self
            .findings
            .iter()
            .filter(|finding| finding.area == Area::Apps)
            .count();
        let tab_labels: Vec<SharedString> = vec![
            "Instaladas".into(),
            if leftovers_count > 0 {
                format!("Restos ({leftovers_count})").into()
            } else {
                "Restos".into()
            },
        ];
        let tab = usize::from(self.apps.tab == AppsTab::Leftovers);
        let tab_entity = cx.entity().downgrade();
        let tabs = segmented("apps-tab", &tab_labels, tab, move |index, _, cx| {
            tab_entity
                .update(cx, |shell, cx| {
                    shell.apps.tab = if index == 0 {
                        AppsTab::Installed
                    } else {
                        AppsTab::Leftovers
                    };
                    cx.notify();
                })
                .ok();
        })
        .into_any_element();

        let count = self.apps.apps.data.as_ref().map_or(0, Vec::len);
        let subtitle = if self.apps.tab == AppsTab::Leftovers {
            "Lo que dejaron atrás las apps que ya no tienes.".to_string()
        } else if self.apps.apps.data.is_some() {
            format!(
                "{} en este Mac. Desinstalar también se lleva sus restos.",
                plural(count, "app", "apps")
            )
        } else {
            "Las apps de este Mac y lo que pesan de verdad, con sus restos.".to_string()
        };
        let mut actions = vec![tabs];
        if self.apps.tab == AppsTab::Installed && self.apps.apps.data.is_some() {
            let sort_labels: Vec<SharedString> =
                vec!["Tamaño".into(), "Último uso".into(), "Nombre".into()];
            let selected = match self.apps.sort {
                AppSort::Size => 0,
                AppSort::LastUsed => 1,
                AppSort::Name => 2,
            };
            let entity = cx.entity().downgrade();
            actions.push(
                segmented("apps-sort", &sort_labels, selected, move |index, _, cx| {
                    entity
                        .update(cx, |shell, cx| {
                            shell.apps.sort = match index {
                                0 => AppSort::Size,
                                1 => AppSort::LastUsed,
                                _ => AppSort::Name,
                            };
                            shell.sort_apps();
                            cx.notify();
                        })
                        .ok();
                })
                .into_any_element(),
            );
        }
        let header = page_header("Apps", subtitle, actions);

        let body: AnyElement = if self.apps.tab == AppsTab::Leftovers {
            self.render_findings_body(Area::Apps, window, cx)
        } else if let (None, Some(error)) = (&self.apps.apps.data, &self.apps.apps.error) {
            div()
                .px(px(GUTTER))
                .child(notice(
                    NoticeKind::Error,
                    "No hemos podido leer tus apps",
                    Some(error.clone().into()),
                    None,
                ))
                .into_any_element()
        } else if self.apps.apps.data.is_none() {
            empty_state(
                Icon::Grid,
                theme::area_color(Area::Apps),
                "Pesando tus apps",
                "Medimos cada app y buscamos lo que tiene repartido por la Biblioteca. Puede tardar un poco.",
                None,
            )
            .into_any_element()
        } else if count == 0 {
            empty_state(
                Icon::Grid,
                theme::area_color(Area::Apps),
                "No hemos encontrado apps",
                "Qué raro. Si tus apps están fuera de /Applications, no las vemos.",
                None,
            )
            .into_any_element()
        } else {
            let handle = self.apps.scroll.clone();
            let bar = self.apps.bar.clone();
            let wide = content_width(window) > 860.0;
            col()
                .flex_1()
                .min_h(px(0.))
                .child(
                    row()
                        .flex_none()
                        .h(px(26.))
                        .mx(px(GUTTER))
                        .gap(sp(3.))
                        .border_b_1()
                        .border_color(theme::hairline())
                        .fs(font::MICRO)
                        .text_color(theme::text_3())
                        .child(div().flex_1().min_w(px(0.)).child("App"))
                        .child(div().w(px(116.)).child("Arquitectura"))
                        .when(wide, |this| {
                            this.child(div().w(px(92.)).child("Último uso"))
                        })
                        .child(div().w(px(110.)).text_right().child("Tamaño"))
                        .child(div().w(px(94.))),
                )
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h(px(0.))
                        .child(
                            uniform_list(
                                "apps-list",
                                count,
                                cx.processor(
                                    move |this: &mut Shell,
                                          range: std::ops::Range<usize>,
                                          _window,
                                          cx| {
                                        range
                                            .map(|index| this.render_app(index, wide, cx))
                                            .collect::<Vec<_>>()
                                    },
                                ),
                            )
                            .track_scroll(handle.clone())
                            .size_full(),
                        )
                        .child(scrollbar(ScrollSource::Uniform(handle), bar)),
                )
                .into_any_element()
        };
        col()
            .size_full()
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_app(&mut self, index: usize, wide: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(app) = self
            .apps
            .order
            .get(index)
            .and_then(|&position| self.apps.apps.data.as_ref()?.get(position))
            .cloned()
        else {
            return div().into_any_element();
        };
        let now = std::time::SystemTime::now();
        let leftovers: u64 = app.leftovers.iter().map(|l| l.size).sum();
        let icon_image = app
            .icon_path
            .as_ref()
            .and_then(|path| self.apps.icons.get(path))
            .cloned();
        let initial = app
            .name
            .chars()
            .next()
            .unwrap_or('?')
            .to_uppercase()
            .to_string();
        let target = app.clone();
        gutter_row()
            .child(
                row()
                    .id(ElementId::NamedInteger("app".into(), index as u64))
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .px(sp(2.))
                    .gap(sp(3.))
                    .rounded(px(radius::MD))
                    .hover(|style| style.bg(theme::surface_1()))
                    .child(match icon_image {
                        Some(image) => img(image)
                            .flex_none()
                            .size(px(36.))
                            .object_fit(ObjectFit::Contain)
                            .into_any_element(),
                        None => row()
                            .flex_none()
                            .size(px(36.))
                            .justify_center()
                            .rounded(px(9.))
                            .bg(theme::surface_2())
                            .border_1()
                            .border_color(theme::hairline())
                            .fs(font::SUBHEAD)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text_2())
                            .child(initial)
                            .into_any_element(),
                    })
                    .child(
                        col()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                row()
                                    .gap(px(7.))
                                    .child(
                                        clip()
                                            .fs(font::BODY)
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(app.name.clone()),
                                    )
                                    .when(app.running, |this| {
                                        this.child(
                                            row()
                                                .flex_none()
                                                .gap(sp(1.))
                                                .fs(font::MICRO)
                                                .text_color(theme::green())
                                                .child(dot(theme::green(), 6.0))
                                                .child("Abierta"),
                                        )
                                    }),
                            )
                            .child(clip().fs(font::MICRO).text_color(theme::text_3()).child(
                                match (&app.uninstall_blocked, &app.version) {
                                    (Some(reason), _) if !app.is_system => reason.clone(),
                                    (_, Some(version)) => format!("Versión {version}"),
                                    _ => String::new(),
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .w(px(116.))
                            .child(arch_pill(app.arch)),
                    )
                    .when(wide, |this| {
                        this.child(
                            clip()
                                .flex_none()
                                .w(px(104.))
                                .fs(font::CAPTION)
                                .text_color(theme::text_3())
                                .child(
                                    app.last_used
                                        .map_or("—".to_string(), |when| human_age(when, now)),
                                ),
                        )
                    })
                    .child(
                        col()
                            .flex_none()
                            .w(px(110.))
                            .items_end()
                            .child(
                                div()
                                    .fs(font::BODY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .tabular()
                                    .child(human_size(app.size + leftovers)),
                            )
                            .when(leftovers >= 1_000_000, |this| {
                                this.child(
                                    div()
                                        .fs(font::MICRO)
                                        .tabular()
                                        .text_color(theme::text_3())
                                        .child(format!("{} de restos", human_size(leftovers))),
                                )
                            }),
                    )
                    .child(div().flex_none().w(px(94.)).flex().justify_end().child(
                        if app.is_system {
                            tag("Del sistema").into_any_element()
                        } else if app.uninstall_blocked.is_some() {
                            tag("No se puede").into_any_element()
                        } else {
                            Btn::ghost("Desinstalar")
                                .size(BtnSize::Sm)
                                .build(
                                    ElementId::NamedInteger("app-uninstall".into(), index as u64),
                                    cx.listener(move |this, _, _, cx| {
                                        this.review_uninstall(target.clone(), cx)
                                    }),
                                )
                                .into_any_element()
                        },
                    )),
            )
            .into_any_element()
    }
}

fn arch_pill(arch: AppArch) -> AnyElement {
    match arch {
        AppArch::Intel => pill("Intel · Rosetta", theme::rose()).into_any_element(),
        AppArch::AppleSilicon => tag("Apple Silicon").into_any_element(),
        AppArch::Universal => tag("Universal").into_any_element(),
        AppArch::Unknown => div()
            .fs(font::CAPTION)
            .text_color(theme::text_3())
            .child("—")
            .into_any_element(),
    }
}
