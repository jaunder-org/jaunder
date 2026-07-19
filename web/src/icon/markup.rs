/// One inline icon `<svg class="j-icon">`, matching the reactive [`Icon`].
#[must_use]
pub(crate) fn render(path: &str, size: u32) -> String {
    format!(
        concat!(
            "<svg class=\"j-icon\" width=\"{size}\" height=\"{size}\" viewBox=\"0 0 20 20\" ",
            "fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" ",
            "stroke-linejoin=\"round\"><path d=\"{path}\"></path></svg>",
        ),
        size = size,
        path = path,
    )
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::render::Icons;

    #[test]
    fn icon_matches_reactive_component_markup() {
        assert_eq!(
            render(Icons::HOME, 16),
            format!(
                "<svg class=\"j-icon\" width=\"16\" height=\"16\" viewBox=\"0 0 20 20\" \
                 fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" \
                 stroke-linejoin=\"round\"><path d=\"{}\"></path></svg>",
                Icons::HOME
            )
        );
    }
}
