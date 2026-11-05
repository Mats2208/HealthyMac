//! Hand-written SVG icons, embedded in the binary. One consistent style: a 20×20 grid, 1.5 stroke,
//! round caps and joins, no fill. GPUI uses them as alpha masks, tinted by `text_color`.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

macro_rules! icons {
    ($($variant:ident => $name:literal : $body:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Icon { $($variant),+ }

        impl Icon {
            pub fn path(self) -> &'static str {
                match self { $(Icon::$variant => concat!("icons/", $name, ".svg")),+ }
            }
        }

        fn lookup(path: &str) -> Option<&'static str> {
            match path {
                $(concat!("icons/", $name, ".svg") => Some(concat!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">"##,
                    $body,
                    "</svg>"
                )),)+
                _ => None,
            }
        }
    };
}

icons! {
    Gauge => "gauge": r#"<path d="M4.2 15.2a7.5 7.5 0 1 1 11.6 0"/><path d="M10 11.2l3-3.6"/><circle cx="10" cy="11.4" r=".9"/>"#,
    Sparkle => "sparkle": r#"<path d="M8.75 3l1.55 4.2 4.2 1.55-4.2 1.55-1.55 4.2-1.55-4.2L3 8.75 7.2 7.2z"/><path d="M15.25 13.25v3.5M13.5 15h3.5"/>"#,
    Code => "code": r#"<path d="M6.75 6l-4 4 4 4M13.25 6l4 4-4 4M11.2 4.5l-2.4 11"/>"#,
    Nodes => "nodes": r#"<circle cx="5" cy="5.25" r="2.25"/><circle cx="15" cy="5.25" r="2.25"/><circle cx="10" cy="14.75" r="2.25"/><path d="M7.25 5.25h5.5M6 7.3l3 5.4M14 7.3l-3 5.4"/>"#,
    File => "file": r#"<path d="M5 2.75h6l4 4v10.5H5z"/><path d="M11 2.75v4h4"/>"#,
    Cube => "cube": r#"<path d="M10 2.5l6.5 3.75v7.5L10 17.5l-6.5-3.75v-7.5z"/><path d="M3.5 6.25L10 10l6.5-3.75M10 10v7.5"/>"#,
    Folder => "folder": r#"<path d="M2.75 5.5A1.75 1.75 0 0 1 4.5 3.75h2.9L9.4 5.75h6.1a1.75 1.75 0 0 1 1.75 1.75v7a1.75 1.75 0 0 1-1.75 1.75h-11a1.75 1.75 0 0 1-1.75-1.75z"/>"#,
    Plug => "plug": r#"<path d="M7 2.75v4M13 2.75v4M5 6.75h10v2.5a5 5 0 0 1-10 0zM10 14.25v3"/>"#,
    Grid => "grid": r#"<rect x="3" y="3" width="5.75" height="5.75" rx="1.5"/><rect x="11.25" y="3" width="5.75" height="5.75" rx="1.5"/><rect x="3" y="11.25" width="5.75" height="5.75" rx="1.5"/><rect x="11.25" y="11.25" width="5.75" height="5.75" rx="1.5"/>"#,
    Power => "power": r#"<path d="M10 2.75v6.5"/><path d="M14.6 5.4a7 7 0 1 1-9.2 0"/>"#,
    Treemap => "treemap": r#"<rect x="2.75" y="3.25" width="14.5" height="13.5" rx="2.25"/><path d="M9.5 3.25v13.5M9.5 10h7.75M13.5 10v6.75"/>"#,
    Activity => "activity": r#"<path d="M2.5 10.5h3.2l2-6 4 11 2-5h3.8"/>"#,
    Sliders => "sliders": r#"<path d="M3 6h7M14 6h3M3 14h3M10 14h7"/><circle cx="12" cy="6" r="2"/><circle cx="8" cy="14" r="2"/>"#,
    Check => "check": r#"<path d="M4.5 10.5l3.75 3.75L15.5 6.25"/>"#,
    Minus => "minus": r#"<path d="M5 10h10"/>"#,
    ChevronRight => "chevron-right": r#"<path d="M8 5l5 5-5 5"/>"#,
    ChevronDown => "chevron-down": r#"<path d="M5 8l5 5 5-5"/>"#,
    ArrowUp => "arrow-up": r#"<path d="M10 16V4M5 9l5-5 5 5"/>"#,
    X => "x": r#"<path d="M5.25 5.25l9.5 9.5M14.75 5.25l-9.5 9.5"/>"#,
    Plus => "plus": r#"<path d="M10 4v12M4 10h12"/>"#,
    Trash => "trash": r#"<path d="M3.5 5.5h13M8 5.5V3.5h4v2M5 5.5l.75 11.25h8.5L15 5.5M8.5 8.75v4.75M11.5 8.75v4.75"/>"#,
    Refresh => "refresh": r#"<path d="M16.5 10a6.5 6.5 0 1 1-2-4.7l2 1.7M16.5 3.5V7H13"/>"#,
    Scan => "scan": r#"<circle cx="9" cy="9" r="5.75"/><path d="M13.25 13.25L17 17"/>"#,
    Shield => "shield": r#"<path d="M10 2.75l6 2.25v4.75c0 3.6-2.5 6.2-6 7.5-3.5-1.3-6-3.9-6-7.5V5z"/><path d="M7.5 10l1.9 1.9 3.1-3.6"/>"#,
    Alert => "alert": r#"<path d="M10 3.25l7.5 13.25h-15z"/><path d="M10 8.5v3.75M10 14.25v.05"/>"#,
    Info => "info": r#"<circle cx="10" cy="10" r="7.25"/><path d="M10 9.25v4.25M10 6.5v.05"/>"#,
    Lock => "lock": r#"<rect x="4.5" y="9" width="11" height="8" rx="1.75"/><path d="M7 9V6.5a3 3 0 0 1 6 0V9"/>"#,
    External => "external": r#"<path d="M8.5 4H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2v-3.5M11.5 3H17v5.5M17 3l-7.5 7.5"/>"#,
    Branch => "branch": r#"<circle cx="6" cy="4.75" r="1.75"/><circle cx="6" cy="15.25" r="1.75"/><circle cx="14" cy="6.75" r="1.75"/><path d="M6 6.5v7M14 8.5c0 3-3 3.5-8 4"/>"#,
    Disk => "disk": r#"<rect x="2.75" y="4.25" width="14.5" height="11.5" rx="2.25"/><path d="M2.75 11.5h14.5M13.75 13.6v.05"/>"#,
    Battery => "battery": r#"<rect x="2.5" y="6" width="13" height="8" rx="2"/><path d="M18 9v2"/>"#,
    Thermometer => "thermometer": r#"<path d="M12 11.7V4.75a2 2 0 0 0-4 0v6.95a3.5 3.5 0 1 0 4 0z"/>"#,
    Cpu => "cpu": r#"<rect x="5" y="5" width="10" height="10" rx="1.75"/><rect x="8" y="8" width="4" height="4" rx=".5"/><path d="M8 2.5V5M12 2.5V5M8 15v2.5M12 15v2.5M2.5 8H5M2.5 12H5M15 8h2.5M15 12h2.5"/>"#,
    Memory => "memory": r#"<rect x="2.5" y="5.5" width="15" height="8.5" rx="1.5"/><path d="M6 8.75v2M10 8.75v2M14 8.75v2M5.5 14v2.25M10 14v2.25M14.5 14v2.25"/>"#,
    Network => "network": r#"<path d="M6.5 16V4M3 7.5L6.5 4 10 7.5M13.5 4v12M10 12.5l3.5 3.5 3.5-3.5"/>"#,
    Container => "container": r#"<path d="M3 7.25h14v8.75H3z"/><path d="M3 7.25L5 4h10l2 3.25M7 10.25v2.75M10 10.25v2.75M13 10.25v2.75"/>"#,
    Stop => "stop": r#"<rect x="5" y="5" width="10" height="10" rx="2"/>"#,
    Bolt => "bolt": r#"<path d="M11 2.75L4.5 11.25H10l-1 6 6.5-8.5H10z"/>"#,
    Copy => "copy": r#"<rect x="7" y="7" width="10" height="10" rx="2"/><path d="M13 7V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h2"/>"#,
    Heart => "heart": r#"<path d="M10 16.75S3 12.5 3 7.75A3.75 3.75 0 0 1 10 5.9a3.75 3.75 0 0 1 7 1.85c0 4.75-7 9-7 9z"/>"#,
    Flask => "flask": r#"<path d="M7.75 3h4.5M8.75 3v5.25L4.6 15.4A1.1 1.1 0 0 0 5.55 17h8.9a1.1 1.1 0 0 0 .95-1.6L11.25 8.25V3M6.75 12.5h6.5"/>"#,
    Spinner => "spinner": r#"<path d="M10 2.75a7.25 7.25 0 1 1-7.25 7.25"/>"#,
    Layers => "layers": r#"<path d="M10 3l7 3.75-7 3.75-7-3.75z"/><path d="M3 10.5l7 3.75 7-3.75M3 14l7 3.75L17 14"/>"#,
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(lookup(path).map(|svg| Cow::Borrowed(svg.as_bytes())))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}
