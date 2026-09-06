//! Parser-backed CSS validation and isolation for Theme Packages.

use std::collections::BTreeMap;

use lightningcss::{
    properties::{Property, animation::AnimationName, font::FontFamily},
    rules::{CssRule, font_face::FontFaceProperty, keyframes::KeyframesName},
    selector::{Combinator, Component, Selector, SelectorList},
    stylesheet::{ParserOptions, PrinterOptions, StyleSheet},
    values::{ident::CustomIdent, url::Url},
    visit_types,
    visitor::{Visit, VisitTypes, Visitor},
};
use sha2::{Digest, Sha256};

use super::{ThemePackageError, ThemePackageLimits};

const SURFACE: &str = r#"[data-jaunder-theme-surface][data-jaunder-style-contract="1"]"#;

/// The deterministic, isolated stylesheet and its raw content digest.
#[derive(Debug)]
pub struct CompiledCss {
    bytes: Vec<u8>,
    digest: [u8; 32],
}

impl CompiledCss {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

/// Validates and scopes a complete authored stylesheet.
///
/// # Errors
///
/// Returns [`ThemePackageError`] when the stylesheet violates package limits or
/// its accepted CSS subset.
pub fn compile_stylesheet(
    authored_css: &[u8],
    _canonical_manifest: &[u8],
    source_digest: [u8; 32],
    asset_urls: &BTreeMap<String, String>,
    limits: ThemePackageLimits,
) -> Result<CompiledCss, ThemePackageError> {
    let authored_css = std::str::from_utf8(authored_css)
        .map_err(|error| ThemePackageError::Css(format!("stylesheet is not UTF-8: {error}")))?;
    let stylesheet_source = format!("{SURFACE} {{}}{authored_css}");
    let mut stylesheet = StyleSheet::parse(&stylesheet_source, ParserOptions::default())
        .map_err(|error| ThemePackageError::Css(error.to_string()))?;
    let CssRule::Style(boundary_rule) = stylesheet.rules.0.remove(0) else {
        return Err(ThemePackageError::Css(
            "could not construct surface selector".into(),
        ));
    };
    let boundary = boundary_rule
        .selectors
        .0
        .into_iter()
        .next()
        .ok_or_else(|| ThemePackageError::Css("could not construct surface selector".into()))?;
    let mut state = RuleState {
        rules: 0,
        limits,
        boundary,
    };
    validate_rules(&mut stylesheet.rules.0, 0, true, &mut state)?;
    let namespace = hex_namespace(source_digest);
    let mut fonts = BTreeMap::new();
    namespace_fonts(&mut stylesheet.rules.0, &namespace, &mut fonts)?;
    let mut keyframes = BTreeMap::new();
    namespace_keyframes(&mut stylesheet.rules.0, &namespace, &mut keyframes)?;
    stylesheet.visit(&mut AssetUrlVisitor {
        asset_urls,
        fonts: &fonts,
        keyframes: &keyframes,
    })?;

    let code = stylesheet
        .to_css(PrinterOptions {
            minify: true,
            ..PrinterOptions::default()
        })
        .map_err(|error| ThemePackageError::Css(error.to_string()))?
        .code;
    let bytes = code.into_bytes();
    Ok(CompiledCss {
        digest: Sha256::digest(&bytes).into(),
        bytes,
    })
}

struct RuleState<'i> {
    rules: usize,
    limits: ThemePackageLimits,
    boundary: Selector<'i>,
}

fn validate_rules<'a>(
    rules: &mut Vec<CssRule<'a>>,
    depth: usize,
    top_level: bool,
    state: &mut RuleState<'a>,
) -> Result<(), ThemePackageError> {
    if depth > state.limits.max_css_nesting {
        return Err(ThemePackageError::LimitExceeded {
            limit: "max_css_nesting",
        });
    }
    for rule in rules {
        state.rules += 1;
        if state.rules > state.limits.max_css_rules {
            return Err(ThemePackageError::LimitExceeded {
                limit: "max_css_rules",
            });
        }
        match rule {
            CssRule::Style(style) => {
                if !style.rules.0.is_empty() {
                    return Err(ThemePackageError::Css(
                        "native CSS nesting is forbidden".into(),
                    ));
                }
                scope_selectors(&mut style.selectors, &state.boundary)?;
            }
            CssRule::Media(media) => validate_rules(&mut media.rules.0, depth + 1, false, state)?,
            CssRule::Supports(supports) => {
                validate_rules(&mut supports.rules.0, depth + 1, false, state)?;
            }
            CssRule::Container(container) => {
                validate_rules(&mut container.rules.0, depth + 1, false, state)?;
            }
            CssRule::FontFace(_) | CssRule::Keyframes(_) if top_level => {}
            CssRule::FontFace(_) | CssRule::Keyframes(_) => {
                return Err(ThemePackageError::Css(
                    "@font-face and @keyframes must be top-level".into(),
                ));
            }
            _ => return Err(ThemePackageError::Css("unsupported at-rule".into())),
        }
    }
    Ok(())
}

fn namespace_keyframes(
    rules: &mut Vec<CssRule<'_>>,
    namespace: &str,
    names: &mut BTreeMap<String, String>,
) -> Result<(), ThemePackageError> {
    for rule in rules {
        if let CssRule::Keyframes(keyframes) = rule {
            let original = match &keyframes.name {
                KeyframesName::Ident(name) => name.0.as_ref(),
                KeyframesName::Custom(name) => name.as_ref(),
            };
            let renamed = format!("jaunder-{namespace}-{original}");
            if names.insert(original.to_owned(), renamed.clone()).is_some() {
                return Err(ThemePackageError::Css("duplicate @keyframes name".into()));
            }
            keyframes.name = KeyframesName::Ident(CustomIdent(renamed.into()));
        }
    }
    Ok(())
}

fn namespace_fonts(
    rules: &mut Vec<CssRule<'_>>,
    namespace: &str,
    names: &mut BTreeMap<String, String>,
) -> Result<(), ThemePackageError> {
    for rule in rules {
        let CssRule::FontFace(font_face) = rule else {
            continue;
        };
        let mut declaration_count = 0;
        for property in &mut font_face.properties {
            match property {
                FontFaceProperty::FontFamily(family) => {
                    declaration_count += 1;
                    if declaration_count > 1 {
                        return Err(ThemePackageError::Css(
                            "duplicate @font-face font-family declaration".into(),
                        ));
                    }
                    let original = custom_font_family_name(family)?;
                    let renamed = format!("jaunder-{namespace}-{original}");
                    if names.insert(original, renamed.clone()).is_some() {
                        return Err(ThemePackageError::Css("duplicate @font-face family".into()));
                    }
                    *family = font_family_from_name(&renamed)?;
                }
                FontFaceProperty::Custom(_) => {
                    return Err(ThemePackageError::Css(
                        "custom-property token streams cannot hide global references".into(),
                    ));
                }
                _ => {}
            }
        }
        if declaration_count == 0 {
            return Err(ThemePackageError::Css(
                "@font-face requires exactly one font-family declaration".into(),
            ));
        }
    }
    Ok(())
}

fn custom_font_family_name(family: &FontFamily<'_>) -> Result<String, ThemePackageError> {
    let FontFamily::FamilyName(_) = family else {
        return Err(ThemePackageError::Css(
            "@font-face font-family must be a custom family name".into(),
        ));
    };
    let serde_json::Value::String(name) = serde_json::to_value(family)
        .map_err(|error| ThemePackageError::Css(format!("font-family schema changed: {error}")))?
    else {
        return Err(ThemePackageError::Css(
            "font-family schema changed: expected a string".into(),
        ));
    };
    if name.is_empty() {
        return Err(ThemePackageError::Css("font-family cannot be empty".into()));
    }
    Ok(name)
}

fn font_family_from_name(name: &str) -> Result<FontFamily<'static>, ThemePackageError> {
    let family = <FontFamily<'static> as serde::Deserialize>::deserialize(
        serde_json::Value::String(name.to_owned()),
    )
    .map_err(|error| ThemePackageError::Css(format!("font-family schema changed: {error}")))?;
    if !matches!(family, FontFamily::FamilyName(_)) || custom_font_family_name(&family)? != name {
        return Err(ThemePackageError::Css(
            "font-family schema changed: custom family did not round-trip".into(),
        ));
    }
    Ok(family)
}

fn rewrite_font_families(
    families: &mut Vec<FontFamily<'_>>,
    names: &BTreeMap<String, String>,
) -> Result<(), ThemePackageError> {
    let mut referenced = None;
    for family in families.iter() {
        if matches!(family, FontFamily::FamilyName(_))
            && referenced
                .replace(custom_font_family_name(family)?)
                .is_some()
        {
            return Err(ThemePackageError::Css(
                "font-family reference is ambiguous".into(),
            ));
        }
    }
    if let Some(original) = referenced {
        let renamed = names.get(&original).ok_or_else(|| {
            ThemePackageError::Css(format!("undeclared font-family reference: {original}"))
        })?;
        let replacement = font_family_from_name(renamed)?;
        for family in families {
            if matches!(family, FontFamily::FamilyName(_)) {
                *family = replacement;
                break;
            }
        }
    }
    Ok(())
}

fn scope_selectors<'i>(
    selectors: &mut SelectorList<'i>,
    boundary: &Selector<'i>,
) -> Result<(), ThemePackageError> {
    for selector in &mut selectors.0 {
        let mut components: Vec<_> = selector.iter_raw_match_order().cloned().collect();
        if components
            .iter()
            .any(|component| matches!(component, Component::Scope))
        {
            return Err(ThemePackageError::Css(
                ":scope cannot be safely isolated".into(),
            ));
        }

        let mut mapped_root = false;
        let mut needs_descendant = false;
        loop {
            let compound_start = components
                .iter()
                .rposition(|component| matches!(component, Component::Combinator(_)))
                .map_or(0, |index| index + 1);
            let root = components[compound_start..]
                .iter()
                .position(is_document_root)
                .map(|index| compound_start + index);
            let Some(root) = root else {
                break;
            };
            mapped_root = true;
            components.remove(root);
            if components[compound_start..].is_empty() && compound_start > 0 {
                components.remove(compound_start - 1);
                needs_descendant = true;
            } else {
                break;
            }
        }
        if components.iter().any(is_document_root) {
            return Err(ThemePackageError::Css(
                "document roots must be a selector prefix".into(),
            ));
        }

        let mut scoped: Vec<_> = boundary.iter_raw_match_order().cloned().collect();
        if !mapped_root || needs_descendant {
            scoped.push(Component::Combinator(Combinator::Descendant));
        }
        scoped.extend(components);
        *selector = Selector::from(scoped);
    }
    Ok(())
}

fn is_document_root(component: &Component<'_>) -> bool {
    matches!(component, Component::Root)
        || matches!(
            component,
            Component::LocalName(name) if matches!(name.lower_name.0.as_ref(), "html" | "body")
        )
}

fn rewrite_animation_names(
    names: &mut [AnimationName<'_>],
    keyframes: &BTreeMap<String, String>,
) -> Result<(), ThemePackageError> {
    let mut referenced = None;
    for name in names.iter() {
        let original = match name {
            AnimationName::None => continue,
            AnimationName::Ident(name) => name.0.as_ref(),
            AnimationName::String(name) => name.0.as_ref(),
        };
        if referenced.replace(original.to_owned()).is_some() {
            return Err(ThemePackageError::Css(
                "animation reference is ambiguous".into(),
            ));
        }
    }
    let Some(original) = referenced else {
        return Ok(());
    };
    let renamed = keyframes.get(&original).ok_or_else(|| {
        ThemePackageError::Css(format!("undeclared animation reference: {original}"))
    })?;
    for name in names {
        match name {
            AnimationName::None => {}
            AnimationName::Ident(name) => {
                name.0 = renamed.clone().into();
                break;
            }
            AnimationName::String(name) => {
                name.0 = renamed.clone().into();
                break;
            }
        }
    }
    Ok(())
}

struct AssetUrlVisitor<'a> {
    asset_urls: &'a BTreeMap<String, String>,
    fonts: &'a BTreeMap<String, String>,
    keyframes: &'a BTreeMap<String, String>,
}

impl<'i> Visitor<'i> for AssetUrlVisitor<'_> {
    type Error = ThemePackageError;

    fn visit_types(&self) -> VisitTypes {
        visit_types!(URLS | PROPERTIES)
    }

    fn visit_url(&mut self, url: &mut Url<'i>) -> Result<(), Self::Error> {
        let path = url.url.as_ref();
        if url.is_absolute() || path.starts_with("//") {
            return Err(ThemePackageError::Url(path.to_owned()));
        }
        let immutable_url = self
            .asset_urls
            .get(path)
            .ok_or_else(|| ThemePackageError::Url(path.to_owned()))?;
        url.url = immutable_url.clone().into();
        Ok(())
    }

    fn visit_property(&mut self, property: &mut Property<'i>) -> Result<(), Self::Error> {
        match property {
            Property::Unparsed(_) | Property::Custom(_) => {
                return Err(ThemePackageError::Css(
                    "custom-property token streams cannot hide global references".into(),
                ));
            }
            Property::FontFamily(families) => rewrite_font_families(families, self.fonts)?,
            Property::Font(font) => rewrite_font_families(&mut font.family, self.fonts)?,
            Property::AnimationName(names, _) => rewrite_animation_names(names, self.keyframes)?,
            Property::Animation(animations, _) => {
                for animation in animations {
                    rewrite_animation_names(
                        std::slice::from_mut(&mut animation.name),
                        self.keyframes,
                    )?;
                }
            }
            _ => {}
        }
        property.visit_children(self)
    }
}

fn hex_namespace(digest: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut namespace = String::with_capacity(16);
    for byte in &digest[..8] {
        namespace.push(char::from(HEX[usize::from(byte >> 4)]));
        namespace.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    namespace
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn compile(
        css: &str,
        assets: &BTreeMap<String, String>,
    ) -> Result<CompiledCss, ThemePackageError> {
        compile_stylesheet(
            css.as_bytes(),
            b"{}",
            [7; 32],
            assets,
            ThemePackageLimits::default(),
        )
    }

    fn compile_with_limits(
        css: &str,
        assets: &BTreeMap<String, String>,
        limits: ThemePackageLimits,
    ) -> Result<CompiledCss, ThemePackageError> {
        compile_stylesheet(css.as_bytes(), b"{}", [7; 32], assets, limits)
    }

    #[test]
    fn scopes_nested_conditionals_deterministically() {
        let css = "@media (width > 1px) { @supports (display: grid) { .card { color: red } } }";
        let first = compile(css, &BTreeMap::new()).unwrap();
        let second = compile(css, &BTreeMap::new()).unwrap();
        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.digest(), second.digest());
        assert!(
            std::str::from_utf8(first.bytes())
                .unwrap()
                .contains("data-jaunder-theme-surface")
        );
    }

    #[test]
    fn maps_document_roots_and_rewrites_assets() {
        let assets = BTreeMap::from([(
            "assets/logo.webp".to_owned(),
            "/theme-assets/abc".to_owned(),
        )]);
        let compiled = compile("html { background: url(assets/logo.webp) }", &assets).unwrap();
        let css = std::str::from_utf8(compiled.bytes()).unwrap();
        assert!(css.contains("/theme-assets/abc"));
        assert!(!css.contains("assets/logo.webp"));
    }

    #[test]
    fn namespaces_keyframes_and_animation_references() {
        let compiled = compile(
            "@keyframes pulse { to { opacity: 0 } } .a { animation-name: pulse }",
            &BTreeMap::new(),
        )
        .unwrap();
        let css = std::str::from_utf8(compiled.bytes()).unwrap();
        assert!(css.contains("jaunder-0707070707070707-pulse"));
        assert!(!css.contains("animation-name:pulse"));
    }

    #[test]
    fn maps_document_root_prefixes_and_rejects_unsafe_root_placement() {
        let compiled = compile(
            "html, body.theme, :root .card, html body .nested { color: red }",
            &BTreeMap::new(),
        )
        .unwrap();
        let css = std::str::from_utf8(compiled.bytes()).unwrap();
        assert!(css.contains("data-jaunder-theme-surface"));
        assert!(!css.contains("html"), "{css}");
        assert!(!css.contains("body"), "{css}");
        assert!(!css.contains(":root"), "{css}");
        assert!(compile(".outer :root { color: red }", &BTreeMap::new()).is_err());
    }

    #[test]
    fn rewrites_only_declared_animation_references() {
        let compiled = compile(
            "@keyframes pulse { to { opacity: 0 } } .a { animation: 1s pulse; container-name: pulse }",
            &BTreeMap::new(),
        )
        .unwrap();
        let css = std::str::from_utf8(compiled.bytes()).unwrap();
        assert!(css.contains("animation:1s jaunder-0707070707070707-pulse"));
        assert!(css.contains("container-name:pulse"));
        assert!(compile(".a { animation-name: missing }", &BTreeMap::new()).is_err());
    }

    #[test]
    fn rejects_forbidden_rules_urls_and_nesting() {
        for css in [
            "@import \"other.css\";",
            "@page { margin: 0 }",
            ".a { & .b { color: red } }",
            ".a { background: url(https://example.test/x) }",
            ".a { background: url(//example.test/x) }",
            ".a { background: url(data:text/plain,x) }",
            ".a { background: url(#fragment) }",
        ] {
            assert!(compile(css, &BTreeMap::new()).is_err(), "{css}");
        }
    }

    #[test]
    fn rejects_undeclared_assets_and_nested_global_rules() {
        assert!(matches!(
            compile(
                ".a { background: url(assets/missing.png) }",
                &BTreeMap::new()
            ),
            Err(ThemePackageError::Url(_))
        ));
        assert!(
            compile(
                "@media all { @keyframes spin { to { opacity: 0 } } }",
                &BTreeMap::new()
            )
            .is_err()
        );
    }

    #[test]
    fn namespaces_declared_font_families_and_references() {
        let source = "@font-face { font-family: \"Display Sans\"; src: url(font.woff2) } .a { font-family: Display Sans, serif; font: italic 16px \"Display Sans\", sans-serif }";
        let assets = BTreeMap::from([("font.woff2".to_owned(), "/theme-assets/font".to_owned())]);
        let compiled = compile(source, &assets).unwrap();
        let repeat = compile(source, &assets).unwrap();
        let css = std::str::from_utf8(compiled.bytes()).unwrap();
        assert_eq!(compiled.bytes(), repeat.bytes());
        assert_eq!(compiled.digest(), repeat.digest());
        assert!(
            css.contains("jaunder-0707070707070707-Display Sans"),
            "{css}"
        );
        assert!(!css.contains("font-family:Display Sans"), "{css}");
        assert!(!css.contains("16px Display Sans"), "{css}");
        assert!(css.contains("serif"));
        assert!(css.contains("sans-serif"));
    }

    #[test]
    fn pins_lightningcss_family_name_serde_schema() {
        let stylesheet = StyleSheet::parse(
            ".a { font-family: \"Display Sans\" }",
            ParserOptions::default(),
        )
        .unwrap();
        let CssRule::Style(rule) = &stylesheet.rules.0[0] else {
            panic!("expected a style rule");
        };
        let Property::FontFamily(families) = &rule.declarations.declarations[0] else {
            panic!("expected a font-family property");
        };
        assert_eq!(
            serde_json::to_value(&families[0]).unwrap(),
            serde_json::Value::String("Display Sans".into())
        );
    }

    #[test]
    fn rejects_ambiguous_or_hidden_font_references() {
        for css in [
            ".a { font-family: First, Second }",
            ".a { font-family: Missing }",
            ".a { font-family: var(--font) }",
            "@font-face { font-family: serif; src: url(font.woff2) }",
            "@font-face { font-family: Brand; src: url(font.woff2) } @font-face { font-family: Brand; src: url(font.woff2) }",
        ] {
            assert!(
                compile(
                    css,
                    &BTreeMap::from([("font.woff2".to_owned(), "/theme-assets/font".to_owned())])
                )
                .is_err(),
                "{css}"
            );
        }
    }
    #[test]
    fn rejects_every_external_and_opaque_url_scheme() {
        for url in [
            "http://example.test/asset",
            "https://example.test/asset",
            "ftp://example.test/asset",
            "file:///tmp/asset",
            "javascript:alert(1)",
            "data:text/plain,asset",
            "blob:https://example.test/id",
            "//example.test/asset",
            "#fragment",
        ] {
            assert!(
                matches!(
                    compile(
                        &format!(".a {{ background-image: url(\"{url}\") }}"),
                        &BTreeMap::new()
                    ),
                    Err(ThemePackageError::Url(_))
                ),
                "{url}"
            );
        }
    }

    #[test]
    fn rejects_unscopable_selectors_and_every_disallowed_at_rule_position() {
        for css in [
            ":scope { color: red }",
            ".outer :root { color: red }",
            "@font-face { font-family: Brand; src: url(font.woff2) } @media all { @font-face { font-family: Nested; src: url(font.woff2) } }",
            "@keyframes pulse { to { opacity: 0 } } @supports (display: grid) { @keyframes nested { to { opacity: 1 } } }",
            "@layer theme { .a { color: red } }",
            "@namespace svg url(http://www.w3.org/2000/svg);",
        ] {
            assert!(compile(css, &BTreeMap::new()).is_err(), "{css}");
        }
    }

    #[test]
    fn enforces_rule_and_conditional_nesting_limits() {
        assert!(matches!(
            compile_with_limits(
                ".a { color: red }",
                &BTreeMap::new(),
                ThemePackageLimits {
                    max_css_rules: 0,
                    ..ThemePackageLimits::default()
                }
            ),
            Err(ThemePackageError::LimitExceeded {
                limit: "max_css_rules"
            })
        ));
        assert!(matches!(
            compile_with_limits(
                "@media all { @supports (display: grid) { .a { color: red } } }",
                &BTreeMap::new(),
                ThemePackageLimits {
                    max_css_nesting: 1,
                    ..ThemePackageLimits::default()
                }
            ),
            Err(ThemePackageError::LimitExceeded {
                limit: "max_css_nesting"
            })
        ));
    }
}
