//! The sidebar: the window's own background, running under the traffic lights.

use std::sync::{Arc, LazyLock};

use gpui::{
    div, img, prelude::*, px, AnyElement, Context, FontWeight, Image, ImageFormat, SharedString,
};
use hm_core::fmt::human_size;
use hm_core::Area;

use crate::assets::Icon;
use crate::logic::selection::reclaimable_by_area;
use crate::state::{AppsTab, Page, Shell};
use crate::theme::{self, font, radius, sp, SIDEBAR_WIDTH};
use crate::ui::{col, icon, row, TextExt};

/// The app's own icon, the brand mark, rather than a stand-in drawn in code.
static LOGO: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../assets/AppIcon.png").to_vec(),
    ))
});

impl Shell {
    pub fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let totals = reclaimable_by_area(&self.findings);
        let size_of = |area: Area| {
            totals
                .get(&area)
                .copied()
                .filter(|&bytes| bytes > 0)
                .map(human_size)
        };

        col()
            .relative()
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .px(sp(2.5))
            .pb(sp(3.))
            .child(self.drag_strip("sidebar-drag", 46.0, cx))
            .child(
                // Clears the traffic lights, then the wordmark.
                row()
                    .mt(sp(12.5))
                    .mb(sp(3.))
                    .px(sp(2.))
                    .gap(sp(2.))
                    .child(img(LOGO.clone()).flex_none().size(px(24.)))
                    .child(
                        div()
                            .fs(font::SUBHEAD)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("HealthyMac"),
                    ),
            )
            .child(self.nav_item(Page::Resumen, Icon::Gauge, "Resumen", None, cx))
            .child(group_label("Limpieza"))
            .child(self.nav_item(
                Page::Area(Area::System),
                Icon::Sparkle,
                "Sistema",
                size_of(Area::System),
                cx,
            ))
            .child(self.nav_item(
                Page::Area(Area::Developer),
                Icon::Code,
                "Developer",
                size_of(Area::Developer),
                cx,
            ))
            .child(self.nav_item(
                Page::Area(Area::LocalAi),
                Icon::Nodes,
                "IA local",
                size_of(Area::LocalAi),
                cx,
            ))
            .child(self.nav_item(
                Page::Area(Area::Files),
                Icon::File,
                "Archivos",
                size_of(Area::Files),
                cx,
            ))
            .child(group_label("Desarrollo e IA"))
            .child(self.nav_item(Page::Modelos, Icon::Cube, "Modelos", None, cx))
            .child(self.nav_item(Page::Proyectos, Icon::Folder, "Proyectos", None, cx))
            .child(self.nav_item(Page::Puertos, Icon::Plug, "Puertos", None, cx))
            .child(group_label("Tu Mac"))
            .child(self.nav_item(Page::Apps, Icon::Grid, "Apps", size_of(Area::Apps), cx))
            .child(self.nav_item(Page::Arranque, Icon::Power, "Arranque", None, cx))
            .child(self.nav_item(Page::Mapa, Icon::Treemap, "Mapa de disco", None, cx))
            .child(self.nav_item(Page::Monitor, Icon::Activity, "Monitor", None, cx))
            .child(div().flex_1().min_h(px(8.)))
            .when_some(self.mode_badges(), |this, badges| this.child(badges))
            .child(self.nav_item(Page::Ajustes, Icon::Sliders, "Ajustes", None, cx))
    }

    fn nav_item(
        &self,
        page: Page,
        which: Icon,
        label: &'static str,
        trailing: Option<String>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.page == page;
        row()
            .id(SharedString::from(format!("nav-{}", page.serial())))
            .h(px(30.))
            .px(sp(2.))
            .gap(px(9.))
            .rounded(px(radius::SM))
            .cursor_pointer()
            .when(selected, |this| this.bg(theme::surface_3()))
            .when(!selected, |this| {
                this.hover(|style| style.bg(theme::surface_1()))
                    .active(|style| style.bg(theme::surface_2()))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if page == Page::Apps {
                    this.apps.tab = AppsTab::Installed;
                }
                this.navigate(page, cx);
            }))
            .child(icon(
                which,
                16.0,
                if selected {
                    theme::text_1()
                } else {
                    theme::text_2()
                },
            ))
            .child(
                div()
                    .flex_1()
                    .fs(font::BODY)
                    .whitespace_nowrap()
                    .when(selected, |this| {
                        this.text_color(theme::text_1())
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!selected, |this| this.text_color(theme::text_2()))
                    .child(label),
            )
            .children(trailing.map(|text| {
                div()
                    .fs(font::MICRO)
                    .tabular()
                    .text_color(theme::text_3())
                    .whitespace_nowrap()
                    .child(text)
            }))
    }

    /// Permanently visible while nothing real can happen, so a screenshot can never mislead.
    fn mode_badges(&self) -> Option<AnyElement> {
        let lines = self.mode_lines();
        (!lines.is_empty()).then(|| {
            col()
                .mb(sp(2.))
                .gap(sp(1.5))
                .px(sp(2.5))
                .py(sp(2.))
                .rounded(px(radius::MD))
                .bg(theme::surface_2())
                .children(lines)
                .into_any_element()
        })
    }

    fn mode_lines(&self) -> Vec<AnyElement> {
        let mut badges = Vec::new();
        if self.mode.demo {
            badges.push(badge(Icon::Layers, "Demo", "datos inventados"));
        }
        if self.mode.dry_run || self.mode.demo {
            badges.push(badge(Icon::Flask, "Simulación", "no se borra nada"));
        }
        if let (Some(home), false) = (&self.mode.home_override, self.mode.demo) {
            badges.push(badge(
                Icon::Folder,
                "Home de prueba",
                &crate::logic::truncate::truncate_middle(&home.display().to_string(), 22),
            ));
        }
        badges
    }
}

fn group_label(text: &'static str) -> impl IntoElement {
    div()
        .mt(sp(3.5))
        .mb(sp(1.))
        .px(sp(2.))
        .fs(font::MICRO)
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::text_3())
        .child(text)
}

/// One quiet line per mode. Together they sit in a single box at the foot of the sidebar.
fn badge(which: Icon, title: &str, detail: &str) -> AnyElement {
    row()
        .gap(sp(2.))
        .child(icon(which, 13.0, theme::text_2()))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .fs(font::MICRO)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(theme::text_3())
                .child(
                    div()
                        .flex()
                        .gap(sp(1.))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::text_1())
                                .child(title.to_string()),
                        )
                        .child(detail.to_string()),
                ),
        )
        .into_any_element()
}
