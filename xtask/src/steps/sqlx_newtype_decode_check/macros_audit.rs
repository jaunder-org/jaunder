/// The derive crate whose `#[proc_macro_derive]`s this gate must account for.
pub(super) const MACROS_LIB: &str = "macros/src/lib.rs";

/// Derives that emit the shared sqlx bridge (`macros/src/sqlx_bridge.rs::bridge`), so a
/// type carrying one is a legitimate decode target.
///
/// **This is the gate's model of the newtype families, and a wrong model fails closed** —
/// a family missing here means every decode into those types is unrecognised, so the gate
/// bites rather than waving them through. That is the whole reason reading *declaration*
/// spellings is legitimate under ADR-0085 while reading *violation* spellings is not: an
/// incomplete approval detector is loud, an incomplete violation detector is silent.
///
/// Failing closed is safe but noisy — a forgotten family would produce dozens of confusing
/// failures at once. [`macro_enumeration_problems`] turns that into a single clear message.
pub(super) const BRIDGE_DERIVES: &[&str] = &["StrNewtype", "IdNewtype", "NumNewtype", "SqlxBridge"];

/// **Attribute** macros that emit the bridge. A bridge-emitting macro need not be a
/// derive: `#[macros::text_enum(sqlx, …)]` (#746) replaces the whole strum + parse-error +
/// serde stack for a closed string enum, and emits the sqlx bridge when asked.
///
/// Enumerated separately because the *approval* rule differs, not just the spelling. The
/// bridge is **opt-in** here — `#[text_enum(…)]` without `sqlx` emits no `Decode`, and
/// several enums are declared exactly that way (`Channel`, `SubscriptionStatus`,
/// `TargetKind`, `AudienceBase` are FK-normalized and bind a `&'static str` instead). So a
/// type carrying this attribute is approved **only when the `sqlx` flag is present**.
///
/// Worth noting the asymmetry: for the derives, "does it emit a bridge?" is *not* a static
/// property (`StrNewtype` suppresses it under `no_sqlx`/`secret`), so they are approved on
/// the derive alone and the module doc records the resulting over-approval. Here the flag
/// is right there in the attribute, so the gate can be exact for free.
pub(super) const BRIDGE_ATTRIBUTES: &[&str] = &["text_enum"];

/// Macros in the same crate that deliberately emit **no** sqlx bridge, and why.
///
/// It exists so adding a non-bridge macro is a deliberate one-line statement rather than a
/// silent omission — [`macro_enumeration_problems`] requires every macro to be in one list
/// or the other.
const NON_BRIDGE_MACROS: &[(&str, &str)] = &[(
    "server",
    "the #[server] server-fn attribute (ADR-0016); nothing to do with column types",
)];

/// Every macro `source` exports — `#[proc_macro_derive(Name)]` and
/// `#[proc_macro_attribute]` alike — or the parse error.
///
/// **Both kinds, because a bridge-emitting macro need not be a derive.** #746 shipped
/// `#[macros::text_enum(sqlx, …)]` as an attribute, and a gate that enumerated only
/// derives would have declared itself complete while the newest bridge family was
/// invisible to it — the exact self-blindness this check exists to prevent.
///
/// Deliberately **not** "which macros reach `sqlx_bridge::bridge()`". That is not a
/// property `syn` can decide: the call is hops deep through module-shadowing local
/// functions, and for `StrNewtype` it is conditional on the derive's own attributes
/// (`no_sqlx` / `secret` suppress it), so "reaches `bridge()`" is not static at all.
/// Enumerating the declarations and forcing each into one of two lists gets the same
/// guarantee from something that can actually be read.
fn declared_macros(source: &str) -> Result<Vec<String>, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut out = Vec::new();
    for item in &file.items {
        let syn::Item::Fn(f) = item else { continue };
        for attr in &f.attrs {
            if attr.path().is_ident("proc_macro_attribute") {
                // An attribute macro is named by the function it decorates.
                out.push(f.sig.ident.to_string());
            } else if attr.path().is_ident("proc_macro_derive") {
                // `#[proc_macro_derive(Name)]` or `#[proc_macro_derive(Name, attributes(..))]`
                // — the derive's name is the first ident in the list either way.
                if let Ok(list) = attr.meta.require_list()
                    && let Some(name) = list.tokens.clone().into_iter().find_map(|t| match t {
                        proc_macro2::TokenTree::Ident(i) => Some(i.to_string()),
                        _ => None,
                    })
                {
                    out.push(name);
                }
            }
        }
    }
    Ok(out)
}

/// Every macro name the gate claims to know, bridge-emitting or not.
fn known_macros() -> Vec<&'static str> {
    BRIDGE_DERIVES
        .iter()
        .chain(BRIDGE_ATTRIBUTES)
        .copied()
        .chain(NON_BRIDGE_MACROS.iter().map(|(n, _)| *n))
        .collect()
}

/// Failures where the gate's macro lists and the macro crate disagree, in either
/// direction.
///
/// A macro the gate has never heard of is the dangerous case (its types silently stop
/// being approved); a listed macro that no longer exists is the stale case (the model has
/// drifted). Both are one clear message here instead of a scatter of decode failures.
pub(super) fn macro_enumeration_problems(source: &str) -> Vec<String> {
    let declared = match declared_macros(source) {
        Ok(d) => d,
        Err(e) => {
            return vec![format!(
                "{MACROS_LIB}: {e} — this gate's approved-type set is derived from the macros \
                 declared here, so a file it cannot parse silently shrinks what it approves."
            )];
        }
    };
    let mut lines = Vec::new();
    for name in &declared {
        if !known_macros().contains(&name.as_str()) {
            lines.push(format!(
                "{MACROS_LIB}: `{name}` is declared but this gate does not know it. If it emits \
                 the sqlx bridge, add it to BRIDGE_DERIVES or BRIDGE_ATTRIBUTES so types carrying \
                 it are approved decode targets; if it does not, add it to NON_BRIDGE_MACROS with \
                 a reason. Leaving it out is not neutral — every decode into a `{name}` type would \
                 fail as unrecognised."
            ));
        }
    }
    for name in BRIDGE_DERIVES.iter().chain(BRIDGE_ATTRIBUTES) {
        if !declared.iter().any(|d| d == name) {
            lines.push(format!(
                "{MACROS_LIB}: `{name}` is listed as bridge-emitting but is no longer declared \
                 there. Delete it — a stale entry means this gate is approving types on the \
                 strength of a macro that does not exist."
            ));
        }
    }
    for (name, _) in NON_BRIDGE_MACROS {
        if !declared.iter().any(|d| d == name) {
            lines.push(format!(
                "{MACROS_LIB}: NON_BRIDGE_MACROS lists `{name}`, which is no longer declared \
                 there. Delete it."
            ));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A macros crate declaring exactly `derives` and `attributes`.
    fn macros_lib_with(derives: &[&str], attributes: &[&str]) -> String {
        let d = derives.iter().map(|n| {
            format!("#[proc_macro_derive({n}, attributes(x))]\npub fn {n}_d(item: TokenStream) -> TokenStream {{ item }}\n")
        });
        let a = attributes.iter().map(|n| {
            format!("#[proc_macro_attribute]\npub fn {n}(a: TokenStream, i: TokenStream) -> TokenStream {{ i }}\n")
        });
        d.chain(a).collect()
    }

    /// The macros crate exactly as the gate currently models it.
    fn macros_lib_as_modelled() -> String {
        let attrs: Vec<&str> = BRIDGE_ATTRIBUTES
            .iter()
            .copied()
            .chain(NON_BRIDGE_MACROS.iter().map(|(n, _)| *n))
            .collect();
        macros_lib_with(BRIDGE_DERIVES, &attrs)
    }

    #[test]
    fn the_shipped_macro_lists_match_the_macros_crate() {
        // The real enforcement is in `run`, which reads macros/src/lib.rs on every gate
        // invocation. Pinning the model here means a drift shows up as a message about the
        // macro lists rather than as thirty unrelated decode failures.
        assert!(
            macro_enumeration_problems(&macros_lib_as_modelled()).is_empty(),
            "{:?}",
            macro_enumeration_problems(&macros_lib_as_modelled())
        );
    }

    #[test]
    fn a_macro_the_gate_has_never_heard_of_is_one_clear_failure() {
        // #746's `SqlxBridge` arrived exactly this way, and the gate caught it on the
        // first run after the rebase. Failing closed is correct but noisy; this message is
        // what makes the cause obvious.
        let mut derives = BRIDGE_DERIVES.to_vec();
        derives.push("NewFamily");
        let attrs: Vec<&str> = BRIDGE_ATTRIBUTES
            .iter()
            .copied()
            .chain(NON_BRIDGE_MACROS.iter().map(|(n, _)| *n))
            .collect();
        let problems = macro_enumeration_problems(&macros_lib_with(&derives, &attrs));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("NewFamily"), "{problems:?}");
        assert!(
            problems[0].contains("BRIDGE_DERIVES") && problems[0].contains("BRIDGE_ATTRIBUTES"),
            "the message must name both fixes: {problems:?}"
        );
    }

    #[test]
    fn an_unknown_attribute_macro_is_caught_too() {
        // The hole #746 would have opened: `text_enum` is an ATTRIBUTE, not a derive, so a
        // check that enumerated only derives would have declared itself complete while the
        // newest bridge family was invisible to it.
        let attrs: Vec<&str> = BRIDGE_ATTRIBUTES
            .iter()
            .copied()
            .chain(NON_BRIDGE_MACROS.iter().map(|(n, _)| *n))
            .chain(std::iter::once("new_attr"))
            .collect();
        let problems = macro_enumeration_problems(&macros_lib_with(BRIDGE_DERIVES, &attrs));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("new_attr"), "{problems:?}");
    }

    #[test]
    fn a_listed_macro_that_no_longer_exists_is_a_failure() {
        // The stale direction: the gate would otherwise keep approving types on the
        // strength of a macro that has been deleted.
        let attrs: Vec<&str> = BRIDGE_ATTRIBUTES
            .iter()
            .copied()
            .chain(NON_BRIDGE_MACROS.iter().map(|(n, _)| *n))
            .collect();
        let fewer = &BRIDGE_DERIVES[..BRIDGE_DERIVES.len() - 1];
        let problems = macro_enumeration_problems(&macros_lib_with(fewer, &attrs));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains(BRIDGE_DERIVES[BRIDGE_DERIVES.len() - 1]),
            "{problems:?}"
        );
    }

    #[test]
    fn an_unparseable_macros_crate_is_a_failure_not_a_skip() {
        let problems = macro_enumeration_problems("pub fn f( {{{");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("silently shrinks"), "{problems:?}");
    }

    #[test]
    fn macro_names_are_read_from_every_declaration_spelling() {
        // A derive with and without an `attributes(..)` trailer (the name is the first
        // ident either way), and an attribute macro (named by its function).
        let src = "#[proc_macro_derive(IdNewtype)]\npub fn a(i: TokenStream) -> TokenStream { i }\n\
                   #[proc_macro_derive(StrNewtype, attributes(str_newtype))]\npub fn b(i: TokenStream) -> TokenStream { i }\n\
                   #[proc_macro_attribute]\npub fn text_enum(a: TokenStream, i: TokenStream) -> TokenStream { i }\n";
        let mut got = declared_macros(src).expect("parses");
        got.sort();
        assert_eq!(
            got,
            vec![
                "IdNewtype".to_string(),
                "StrNewtype".to_string(),
                "text_enum".to_string()
            ]
        );
    }
}
