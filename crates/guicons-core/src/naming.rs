/// Converts a manifest key (e.g. `settings-filled`) into a Rust `SCREAMING_SNAKE_CASE`
/// identifier fragment, shared by the codegen in `guicons` and the `guicons::icon!` macro
/// so the two never drift apart on what a given key's constant is named.
pub fn rust_const_name(key: &str) -> String {
    key.replace(['.', '-'], "_").to_ascii_uppercase()
}

/// Converts a manifest name (a family or variant) into a Rust `snake_case`
/// fn/method name, guarding against the two ways an otherwise valid segment
/// can fail to be a valid identifier: starting with a digit (`20` ->
/// `icon_20`) and colliding with a reserved keyword (`type` -> `r#type`).
pub fn rust_fn_name(name: &str) -> String {
    let mut result = String::new();
    for segment in name.split(['.', '-', '_']) {
        if segment.is_empty() {
            continue;
        }
        if !result.is_empty() {
            result.push('_');
        }
        result.push_str(&segment.to_ascii_lowercase());
    }
    if result.is_empty() {
        return "icon".to_string();
    }
    if result.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        result = format!("icon_{result}");
    }
    if is_rust_keyword(&result) {
        result = format!("r#{result}");
    }
    result
}

fn is_rust_keyword(name: &str) -> bool {
    matches!(
        name,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
            | "union"
    )
}

#[cfg(test)]
mod tests {
    use super::rust_fn_name;

    #[test]
    fn converts_dashes_to_snake_case() {
        assert_eq!(rust_fn_name("group-list"), "group_list");
    }

    #[test]
    fn lowercases_mixed_case_segments() {
        assert_eq!(rust_fn_name("Settings"), "settings");
    }

    #[test]
    fn prefixes_a_leading_digit() {
        assert_eq!(rust_fn_name("20"), "icon_20");
    }

    #[test]
    fn escapes_a_reserved_keyword() {
        assert_eq!(rust_fn_name("type"), "r#type");
    }

    #[test]
    fn falls_back_to_icon_for_an_empty_name() {
        assert_eq!(rust_fn_name(""), "icon");
    }
}
