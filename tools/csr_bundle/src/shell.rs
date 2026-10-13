//! Rendered-shell validation shared by bundle preparation and server staging.

use std::fmt;

/// A rendered shell disagrees with its verified asset URLs or initialization order.
#[derive(Debug)]
pub struct ShellValidationError(String);

impl fmt::Display for ShellValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ShellValidationError {}

/// Verify the rendered shell against the bundle's resolved role URLs.
///
/// # Errors
///
/// Returns an error for missing or duplicate URLs/markers or invalid initialization order.
pub fn validate_rendered_shell(
    shell: &str,
    glue: &str,
    wasm: &str,
    application_stylesheet_url: &str,
) -> Result<(), ShellValidationError> {
    for (role, url) in [("glue", glue), ("WASM", wasm)] {
        if shell.matches(url).count() != 1 {
            return Err(ShellValidationError(format!(
                "staged shell must contain exactly one {role} URL"
            )));
        }
    }
    let early_fetch = shell
        .find("window.__jaunderWasmFetch = fetch")
        .ok_or_else(|| {
            ShellValidationError("staged shell is missing the early WASM fetch".into())
        })?;
    let application_stylesheet =
        format!(r#"<link rel="stylesheet" href="{application_stylesheet_url}" />"#);
    let (Some(stylesheet), 1) = (
        shell.find(&application_stylesheet),
        shell.matches(&application_stylesheet).count(),
    ) else {
        return Err(ShellValidationError(
            "staged shell must contain exactly one verified application stylesheet URL".into(),
        ));
    };
    let import = shell
        .find("import {initMeasured}")
        .ok_or_else(|| ShellValidationError("staged shell is missing the glue import".into()))?;
    let mark = shell.find("performance.mark").ok_or_else(|| {
        ShellValidationError("staged shell is missing the initialization mark".into())
    })?;
    let init = shell
        .find("initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl)")
        .ok_or_else(|| {
            ShellValidationError("staged shell is missing the WASM fallback initializer".into())
        })?;
    if early_fetch < stylesheet && stylesheet < import && import < mark && mark < init {
        Ok(())
    } else {
        Err(ShellValidationError(
            "staged shell does not preserve early-fetch, fallback, and initialization ordering"
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::validate_rendered_shell;

    fn application_url() -> String {
        format!("/theme/{}", "a".repeat(64))
    }

    fn shell(glue: &str, wasm: &str) -> String {
        let application_url = application_url();
        format!(
            r#"<script>const __jaunderWasmUrl = "{wasm}"; window.__jaunderWasmFetch = fetch(__jaunderWasmUrl);</script>
<link rel="stylesheet" href="{application_url}" />
<script type="module">import {{initMeasured}} from "{glue}"; performance.mark("jaunder.module.before_init"); initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl);</script>"#
        )
    }

    #[test]
    fn shell_requires_unique_urls_and_required_order() {
        let glue = "/pkg/glue.js";
        let wasm = "/pkg/module.wasm";
        validate_rendered_shell(&shell(glue, wasm), glue, wasm, &application_url())
            .expect("valid shell");
        let wrong_application_url = format!("/theme/{}", "b".repeat(64));
        assert!(
            validate_rendered_shell(&shell(glue, wasm), glue, wasm, &wrong_application_url)
                .expect_err("wrong application digest")
                .to_string()
                .contains("verified application stylesheet URL")
        );
        let retired = shell(glue, wasm).replace(&application_url(), "/style/jaunder.css");
        assert!(
            validate_rendered_shell(&retired, glue, wasm, &application_url())
                .expect_err("retired stylesheet URL")
                .to_string()
                .contains("verified application stylesheet URL")
        );
        let duplicate = format!("{}\n{glue}", shell(glue, wasm));
        assert!(
            validate_rendered_shell(&duplicate, glue, wasm, &application_url())
                .expect_err("duplicate glue URL")
                .to_string()
                .contains("exactly one glue URL")
        );
        let misordered = format!(
            r#"<link rel="stylesheet" href="{}" />window.__jaunderWasmFetch = fetch import {{initMeasured}} performance.mark initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl) {glue} {wasm}"#,
            application_url()
        );
        assert!(
            validate_rendered_shell(&misordered, glue, wasm, &application_url())
                .expect_err("misordered shell")
                .to_string()
                .contains("does not preserve")
        );
    }

    #[test]
    fn shell_requires_all_initialization_markers() {
        for required in [
            "window.__jaunderWasmFetch = fetch",
            r#"<link rel="stylesheet" href="/theme/"#,
            "import {initMeasured}",
            "performance.mark",
            "initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl)",
        ] {
            let error = validate_rendered_shell(required, "glue", "wasm", &application_url())
                .expect_err("missing required position");
            assert!(
                error.to_string().contains("missing") || error.to_string().contains("exactly one")
            );
        }
        let shell_with_unique_urls = shell("/pkg/glue.js", "/pkg/module.wasm");
        for required in [
            "performance.mark",
            "initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl)",
        ] {
            let incomplete = shell_with_unique_urls.replacen(required, "", 1);
            let error = validate_rendered_shell(
                &incomplete,
                "/pkg/glue.js",
                "/pkg/module.wasm",
                &application_url(),
            )
            .expect_err("unique URLs do not excuse a missing required shell marker");
            assert!(error.to_string().contains("missing"));
        }
    }
}
