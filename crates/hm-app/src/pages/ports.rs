//! Who is listening on what. Handy when something already holds port 3000.

use gpui::{div, prelude::*, px, uniform_list, AnyElement, Context, ElementId, FontWeight, Window};

use super::{gutter_row, plural};
use crate::assets::Icon;
use crate::state::Shell;
use crate::theme::{self, font, radius, sp, GUTTER};
use crate::ui::chrome::{empty_state, notice, page_header, NoticeKind};
use crate::ui::controls::{tag, Btn, BtnSize};
use crate::ui::label::path_label;
use crate::ui::scrollbar::{scrollbar, ScrollSource};
use crate::ui::{clip, col, row, TextExt};

const ROW_HEIGHT: f32 = 52.0;
const PROCESS_WIDTH: f32 = 270.0;

impl Shell {
    pub fn render_ports(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let count = self.ports.ports.data.as_ref().map_or(0, Vec::len);
        let subtitle = if self.ports.ports.data.is_some() {
            format!(
                "{} de tus procesos. Se actualiza cada pocos segundos.",
                plural(count, "puerto escuchando", "puertos escuchando")
            )
        } else {
            "Qué procesos tuyos están escuchando en la red.".to_string()
        };
        let header = page_header("Puertos", subtitle, Vec::new());

        let body: AnyElement = if let (None, Some(error)) =
            (&self.ports.ports.data, &self.ports.ports.error)
        {
            div()
                .px(px(GUTTER))
                .child(notice(
                    NoticeKind::Error,
                    "No hemos podido leer los puertos",
                    Some(error.clone().into()),
                    None,
                ))
                .into_any_element()
        } else if self.ports.ports.data.is_none() {
            empty_state(
                Icon::Plug,
                theme::area_color(hm_core::Area::Developer),
                "Mirando los puertos",
                "Un segundo.",
                None,
            )
            .into_any_element()
        } else if count == 0 {
            empty_state(
                Icon::Plug,
                theme::area_color(hm_core::Area::Developer),
                "Nadie está escuchando",
                "Ninguno de tus procesos tiene un puerto abierto ahora mismo. Cuando arranques un servidor, lo verás aquí.",
                None,
            )
            .into_any_element()
        } else {
            let handle = self.ports.scroll.clone();
            let bar = self.ports.bar.clone();
            col()
                .flex_1()
                .min_h(px(0.))
                .when_some(self.ports.ports.error.clone(), |this, error| {
                    this.child(div().px(px(GUTTER)).pb(sp(2.)).child(notice(
                        NoticeKind::Warning,
                        "No se ha podido actualizar: esto es de hace un momento",
                        Some(error.into()),
                        None,
                    )))
                })
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
                        .child(div().w(px(72.)).child("Puerto"))
                        .child(div().w(px(PROCESS_WIDTH)).child("Proceso"))
                        .child(div().flex_1().min_w(px(0.)).child("Carpeta"))
                        .child(div().w(px(80.)).child("Usuario"))
                        .child(div().w(px(82.))),
                )
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h(px(0.))
                        .child(
                            uniform_list(
                                "ports-list",
                                count,
                                cx.processor(
                                    |this: &mut Shell,
                                     range: std::ops::Range<usize>,
                                     _window,
                                     cx| {
                                        range
                                            .map(|index| this.render_port(index, cx))
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

    fn render_port(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(port) = self
            .ports
            .ports
            .data
            .as_ref()
            .and_then(|ports| ports.get(index))
            .cloned()
        else {
            return div().into_any_element();
        };
        let target = port.clone();
        gutter_row()
            .child(
                row()
                    .id(ElementId::NamedInteger("port".into(), index as u64))
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .px(sp(2.))
                    .gap(sp(3.))
                    .rounded(px(radius::MD))
                    .hover(|style| style.bg(theme::surface_1()))
                    .child(
                        row()
                            .flex_none()
                            .w(px(72.))
                            .gap(sp(1.5))
                            .child(
                                div()
                                    .fs(font::SUBHEAD)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .tabular()
                                    .child(port.port.to_string()),
                            )
                            .when(port.protocol != "TCP", |this| {
                                this.child(tag(port.protocol.clone()))
                            }),
                    )
                    .child(
                        col()
                            .flex_none()
                            .w(px(PROCESS_WIDTH))
                            .child(
                                clip()
                                    .w_full()
                                    .fs(font::BODY)
                                    .child(port.process_name.clone()),
                            )
                            .child(
                                row()
                                    .w_full()
                                    .gap(sp(1.5))
                                    .when_some(port.service_hint.clone(), |this, hint| {
                                        this.child(
                                            clip()
                                                .fs(font::MICRO)
                                                .text_color(theme::area_color(
                                                    hm_core::Area::Developer,
                                                ))
                                                .child(hint),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex_none()
                                            .fs(font::MICRO)
                                            .tabular()
                                            .text_color(theme::text_3())
                                            .child(format!("PID {} · {}", port.pid, port.address)),
                                    ),
                            ),
                    )
                    .child(
                        // A working directory of "/" says nothing about which project this is.
                        div().flex_1().min_w(px(0.)).child(
                            match port.cwd.as_ref().filter(|cwd| cwd.as_os_str() != "/") {
                                Some(cwd) => path_label(
                                    cwd.display().to_string(),
                                    font::CAPTION,
                                    theme::text_2(),
                                )
                                .into_any_element(),
                                None => div()
                                    .fs(font::CAPTION)
                                    .text_color(theme::text_3())
                                    .child("—")
                                    .into_any_element(),
                            },
                        ),
                    )
                    .child(
                        clip()
                            .flex_none()
                            .w(px(80.))
                            .fs(font::CAPTION)
                            .text_color(theme::text_2())
                            .child(port.user.clone()),
                    )
                    .child(div().flex_none().w(px(82.)).flex().justify_end().child(
                        Btn::ghost("Terminar").size(BtnSize::Sm).build(
                            ElementId::NamedInteger("port-kill".into(), index as u64),
                            cx.listener(move |this, _, _, cx| {
                                this.open_terminate(target.clone(), cx)
                            }),
                        ),
                    )),
            )
            .into_any_element()
    }
}
