use gpui::{AssetSource, Rgba, SharedString, Styled, Svg, px, svg};
use std::borrow::Cow;

pub struct Icons;

pub const BRAND: &str = "brand/fero.svg";
pub const BRAND_SVG: &str =
    include_str!("../design/brand/svg/magnet/colors/fero-magnet-harbor-current.svg");

macro_rules! solar_icons {
    ($($name:literal),+ $(,)?) => {
        impl AssetSource for Icons {
            fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
                let bytes: &[u8] = match path {
                    BRAND => BRAND_SVG.as_bytes(),
                    $($name => include_bytes!(concat!("../design/icons/", $name, ".svg")),)+
                    _ => return Ok(None),
                };
                Ok(Some(Cow::Borrowed(bytes)))
            }

            fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
                Ok(if path.is_empty() {
                    vec![BRAND.into(), $($name.into()),+]
                } else {
                    Vec::new()
                })
            }
        }
    };
}

solar_icons!(
    "add",
    "alt-arrow-down",
    "arrow-down",
    "arrow-left",
    "arrow-up",
    "check",
    "clock-circle",
    "close",
    "danger-circle",
    "folder-open",
    "list",
    "list-minus",
    "magnet",
    "magnifier",
    "panel-left-close",
    "panel-left-open",
    "pause",
    "play",
    "settings",
    "stop",
    "trash-bin-2",
);

pub fn icon(name: &'static str, color: Rgba, size: f32) -> Svg {
    svg()
        .path(name)
        .size(px(size))
        .flex_none()
        .text_color(color)
}
