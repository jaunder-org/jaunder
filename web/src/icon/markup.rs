use maud::html;

use crate::html::Markup;

/// One inline icon `<svg class="j-icon">`, mirroring the reactive [`Icon`].
#[must_use]
pub(crate) fn render(path: &str, size: u32) -> Markup {
    Markup::new(html! {
        svg class="j-icon" width=(size) height=(size) viewBox="0 0 20 20"
            fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"
            stroke-linejoin="round"
        {
            path d=(path) {}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::icon::Icons;

    /// A renderer regression pin, not a claim about the reactive `Icon`: under CSR
    /// the component builds DOM nodes and emits no bytes to compare against.
    #[test]
    fn icon_markup_is_stable() {
        assert_eq!(
            render(Icons::HOME, 16).as_str(),
            format!(
                "<svg class=\"j-icon\" width=\"16\" height=\"16\" viewBox=\"0 0 20 20\" \
                 fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" \
                 stroke-linejoin=\"round\"><path d=\"{}\"></path></svg>",
                Icons::HOME
            )
        );
    }
}
