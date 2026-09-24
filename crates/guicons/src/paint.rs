use crate::Color;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

const CURRENT_COLOR: &[u8] = b"currentColor";

type PaintedCache = HashMap<(usize, usize, Color), &'static [u8]>;

static PAINTED: LazyLock<Mutex<PaintedCache>> = LazyLock::new(Default::default);

/// `template` with `currentColor` drawn in `color`, computed once per pair.
pub(crate) fn painted(template: &'static [u8], color: Color) -> &'static [u8] {
    let mut painted = PAINTED.lock().unwrap_or_else(PoisonError::into_inner);
    painted
        .entry((template.as_ptr() as usize, template.len(), color))
        .or_insert_with(|| paint_svg(template, color).leak())
}

fn paint_svg(template: &[u8], color: Color) -> Vec<u8> {
    let hex = format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b);
    let mut out = Vec::with_capacity(template.len());
    let mut rest = template;
    while !rest.is_empty() {
        if rest.len() >= CURRENT_COLOR.len() && rest[..CURRENT_COLOR.len()].eq_ignore_ascii_case(CURRENT_COLOR) {
            out.extend_from_slice(hex.as_bytes());
            rest = &rest[CURRENT_COLOR.len()..];
        } else {
            out.push(rest[0]);
            rest = &rest[1..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IconData, Theme, ThemeColors};

    const TEMPLATE: &[u8] = b"<svg><path stroke=\"currentColor\" style=\"fill:currentcolor\"/></svg>";

    #[test]
    fn current_color_is_replaced_in_any_case() {
        assert_eq!(
            paint_svg(TEMPLATE, Color::rgb(0x12, 0xab, 0xff)),
            b"<svg><path stroke=\"#12abff\" style=\"fill:#12abff\"/></svg>"
        );
    }

    #[test]
    fn painted_bytes_are_shared_per_color() {
        let first = painted(TEMPLATE, Color::rgb(1, 2, 3));
        let same = painted(TEMPLATE, Color::rgb(1, 2, 3));
        let other = painted(TEMPLATE, Color::rgb(3, 2, 1));
        assert!(std::ptr::eq(first, same));
        assert!(!std::ptr::eq(first, other));
    }

    const COLORS: ThemeColors = ThemeColors { light: Color::rgb(0, 0, 0), dark: Color::rgb(255, 255, 255) };

    #[test]
    fn painted_icon_follows_the_theme_unless_overridden() {
        let icon = IconData::PaintedSvg { template: TEMPLATE, colors: COLORS, color: None };
        let red = icon.with_color([255, 0, 0]).unwrap();

        crate::set_theme(Theme::Light);
        assert_eq!(icon.color(), Some(Color::rgb(0, 0, 0)));
        assert_eq!(red.color(), Some(Color::rgb(255, 0, 0)));

        crate::set_theme(Theme::Dark);
        assert_eq!(icon.color(), Some(Color::rgb(255, 255, 255)));
        assert_eq!(red.color(), Some(Color::rgb(255, 0, 0)));
        assert!(std::ptr::eq(icon.svg_bytes().unwrap(), painted(TEMPLATE, Color::rgb(255, 255, 255))));

        crate::set_theme(Theme::Light);
    }

    #[test]
    fn with_color_only_repaints_painted_icons() {
        assert_eq!(IconData::Svg(TEMPLATE).with_color([255, 255, 255]), None);
        assert_eq!(IconData::Svg(TEMPLATE).color(), None);
        assert_eq!(IconData::Svg(TEMPLATE).svg_bytes(), Some(TEMPLATE));
    }
}
