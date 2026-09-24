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
    use crate::IconData;

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

    #[test]
    fn with_color_only_repaints_painted_icons() {
        let icon = IconData::PaintedSvg { template: TEMPLATE, color: Color::rgb(0, 0, 0) };
        assert_eq!(
            icon.with_color([255, 255, 255]),
            Some(IconData::PaintedSvg { template: TEMPLATE, color: Color::rgb(255, 255, 255) })
        );
        assert_eq!(IconData::Svg(TEMPLATE).with_color([255, 255, 255]), None);
        assert_eq!(IconData::Svg(TEMPLATE).svg_bytes(), Some(TEMPLATE));
    }
}
