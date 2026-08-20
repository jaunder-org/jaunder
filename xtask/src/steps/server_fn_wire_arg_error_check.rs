//! The `server-fn-wire-arg-error` static check (#846): server-fn argument
//! decode telemetry must not export raw newtype parse errors.
//!
//! The check starts from the same `#[macros::server]` inventory as the registrar
//! and tracing gates, then expands local request aggregates to the leaf wire
//! types whose `FromStr::Err` displays can reach decode errors.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use quote::ToTokens;

use crate::result::{CommandResult, StepResult};
use crate::{files, web_server_fns};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct WireInput {
    pub(crate) server_fn: String,
    pub(crate) root: String,
    pub(crate) field_path: Vec<String>,
    pub(crate) ty: String,
}

#[derive(Default)]
pub(crate) struct TypeIndex {
    aggregates: BTreeMap<String, Vec<Field>>,
    errors: BTreeMap<String, ErrorType>,
    from_str_errors: BTreeMap<String, String>,
    reachable_variants: BTreeMap<(String, String), BTreeSet<String>>,
}

#[derive(Clone)]
struct Field {
    name: String,
    ty: syn::Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ErrorType {
    name: String,
    variants: Vec<ErrorVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ErrorVariant {
    name: String,
    fields: BTreeMap<String, String>,
    display: Option<String>,
    transparent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DisplayClass {
    TelemetrySafe,
    Unsafe(String),
}

pub(crate) fn type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
        syn::Type::Reference(reference) => type_name(&reference.elem),
        syn::Type::Paren(paren) => type_name(&paren.elem),
        syn::Type::Group(group) => type_name(&group.elem),
        _ => None,
    }
}

fn named_fields(fields: &syn::Fields) -> Option<Vec<Field>> {
    let syn::Fields::Named(fields) = fields else {
        return None;
    };
    Some(
        fields
            .named
            .iter()
            .map(|field| Field {
                name: field
                    .ident
                    .as_ref()
                    .expect("named fields have idents")
                    .to_string(),
                ty: field.ty.clone(),
            })
            .collect(),
    )
}

fn fields_by_name(fields: &syn::Fields) -> BTreeMap<String, String> {
    match fields {
        syn::Fields::Named(fields) => fields
            .named
            .iter()
            .filter_map(|field| {
                Some((
                    field.ident.as_ref()?.to_string(),
                    field.ty.to_token_stream().to_string().replace(' ', ""),
                ))
            })
            .collect(),
        syn::Fields::Unnamed(fields) => fields
            .unnamed
            .iter()
            .enumerate()
            .map(|(index, field)| {
                (
                    index.to_string(),
                    field.ty.to_token_stream().to_string().replace(' ', ""),
                )
            })
            .collect(),
        syn::Fields::Unit => BTreeMap::new(),
    }
}

fn error_attr(attrs: &[syn::Attribute]) -> Option<(String, bool)> {
    attrs
        .iter()
        .find(|attr| attr.path().is_ident("error"))
        .map(|attr| {
            let tokens = attr.meta.to_token_stream().to_string();
            (tokens.clone(), tokens.contains("transparent"))
        })
}

fn record_error_struct(index: &mut TypeIndex, item: &syn::ItemStruct) {
    let Some((display, transparent)) = error_attr(&item.attrs) else {
        return;
    };
    index.errors.insert(
        item.ident.to_string(),
        ErrorType {
            name: item.ident.to_string(),
            variants: vec![ErrorVariant {
                name: item.ident.to_string(),
                fields: fields_by_name(&item.fields),
                display: Some(display),
                transparent,
            }],
        },
    );
}

fn record_error_enum(index: &mut TypeIndex, item: &syn::ItemEnum) {
    let variants = item
        .variants
        .iter()
        .filter_map(|variant| {
            let (display, transparent) = error_attr(&variant.attrs)?;
            Some(ErrorVariant {
                name: variant.ident.to_string(),
                fields: fields_by_name(&variant.fields),
                display: Some(display),
                transparent,
            })
        })
        .collect::<Vec<_>>();
    if !variants.is_empty() {
        index.errors.insert(
            item.ident.to_string(),
            ErrorType {
                name: item.ident.to_string(),
                variants,
            },
        );
    }
}

fn impl_trait_last_segment(item: &syn::ItemImpl) -> Option<String> {
    item.trait_
        .as_ref()?
        .1
        .segments
        .last()
        .map(|s| s.ident.to_string())
}

fn record_from_str_impl(index: &mut TypeIndex, item: &syn::ItemImpl) {
    if impl_trait_last_segment(item).as_deref() != Some("FromStr") {
        return;
    }
    let Some(self_type_name) = type_name(&item.self_ty) else {
        return;
    };
    let mut error_name = None;
    let mut body = String::new();
    for impl_item in &item.items {
        match impl_item {
            syn::ImplItem::Type(item) if item.ident == "Err" => {
                error_name = type_name(&item.ty);
            }
            syn::ImplItem::Fn(item) if item.sig.ident == "from_str" => {
                body = item.block.to_token_stream().to_string();
            }
            _ => {}
        }
    }
    let Some(error_name) = error_name else {
        return;
    };
    let reachable = reachable_variants(&self_type_name, &error_name, &body);
    index
        .reachable_variants
        .insert((self_type_name.clone(), error_name.clone()), reachable);
    index.from_str_errors.insert(self_type_name, error_name);
}

fn reachable_variants(self_type_name: &str, error_name: &str, body: &str) -> BTreeSet<String> {
    if matches!(self_type_name, "Password" | "ProfferedPassword") && error_name == "PasswordError" {
        return ["PasswordTooShort", "PasswordTooLong"]
            .into_iter()
            .map(String::from)
            .collect();
    }
    variants_named_in_body(body, error_name)
}

fn variants_named_in_body(body: &str, error_name: &str) -> BTreeSet<String> {
    let mut variants = BTreeSet::new();
    let needle = format!("{error_name} ::");
    let mut rest = body;
    while let Some(offset) = rest.find(&needle) {
        rest = &rest[offset + needle.len()..];
        let name = rest
            .trim_start()
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect::<String>();
        if !name.is_empty() {
            variants.insert(name);
        }
    }
    if variants.is_empty()
        && body
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .any(|word| word == error_name)
    {
        variants.insert(error_name.to_string());
    }
    variants
}

fn from_str_error_for(type_name: &str, index: &TypeIndex) -> Option<ErrorType> {
    let error_name = index.from_str_errors.get(type_name)?;
    index.errors.get(error_name).cloned()
}

fn is_safe_scalar(ty: &str) -> bool {
    matches!(
        ty,
        "usize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "isize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "bool"
    )
}

fn placeholders(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        if rest.starts_with('{') {
            rest = &rest[1..];
            continue;
        }
        let Some(close) = rest.find('}') else {
            break;
        };
        let mut name = rest[..close].trim().trim_start_matches(':');
        if let Some((before, _)) = name.split_once(':') {
            name = before;
        }
        if let Some((before, _)) = name.split_once('?') {
            name = before;
        }
        let name = name.trim();
        if !name.is_empty() && !name.chars().all(|ch| ch.is_ascii_uppercase() || ch == '_') {
            out.push(name.to_string());
        }
        rest = &rest[close + 1..];
    }
    out
}

fn display_classification_for_variants(
    error: &ErrorType,
    reachable: Option<&BTreeSet<String>>,
) -> DisplayClass {
    for variant in error
        .variants
        .iter()
        .filter(|variant| reachable.is_none_or(|set| set.contains(&variant.name)))
    {
        if variant.transparent {
            return DisplayClass::Unsafe(format!(
                "{}::{} is #[error(transparent)]",
                error.name, variant.name
            ));
        }
        let Some(display) = &variant.display else {
            return DisplayClass::Unsafe(format!(
                "{}::{} has no display shape",
                error.name, variant.name
            ));
        };
        for placeholder in placeholders(display) {
            let field_name = placeholder.trim_start_matches('.');
            let Some(ty) = variant.fields.get(field_name) else {
                continue;
            };
            if ty == "String" || !is_safe_scalar(ty) {
                return DisplayClass::Unsafe(format!(
                    "{}::{} interpolates {field_name}: {ty}",
                    error.name, variant.name
                ));
            }
        }
    }
    DisplayClass::TelemetrySafe
}

#[cfg(test)]
fn display_classification(type_name: &str, error: &ErrorType, index: &TypeIndex) -> DisplayClass {
    let reachable_key = (type_name.to_string(), error.name.clone());
    let reachable = index
        .reachable_variants
        .get(&reachable_key)
        .filter(|variants| !variants.is_empty());
    display_classification_for_variants(error, reachable)
}

fn index_sources(sources: &[(String, String)]) -> Result<TypeIndex, String> {
    let mut index = TypeIndex::default();
    for (path, src) in sources {
        let file = syn::parse_file(src).map_err(|e| format!("{path}: cannot parse: {e}"))?;
        for item in file.items {
            match item {
                syn::Item::Struct(item) => {
                    record_error_struct(&mut index, &item);
                    if let Some(fields) = named_fields(&item.fields) {
                        index.aggregates.insert(item.ident.to_string(), fields);
                    }
                }
                syn::Item::Enum(item) => record_error_enum(&mut index, &item),
                syn::Item::Impl(item) => record_from_str_impl(&mut index, &item),
                _ => {}
            }
        }
    }
    Ok(index)
}

fn inner_container_types<'a>(ty: &'a syn::Type, out: &mut Vec<&'a syn::Type>) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    let Some(segment) = path.path.segments.last() else {
        return false;
    };
    if !matches!(
        segment.ident.to_string().as_str(),
        "Option" | "Vec" | "Box" | "Rc" | "Arc"
    ) {
        return false;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return false;
    };
    for arg in &args.args {
        if let syn::GenericArgument::Type(inner) = arg {
            out.push(inner);
        }
    }
    true
}

fn expand_type(
    server_fn: &str,
    root: &str,
    field_path: &mut Vec<String>,
    ty: &syn::Type,
    index: &TypeIndex,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<WireInput>,
) {
    match ty {
        syn::Type::Reference(reference) => {
            expand_type(
                server_fn,
                root,
                field_path,
                &reference.elem,
                index,
                seen,
                out,
            );
        }
        syn::Type::Paren(paren) => {
            expand_type(server_fn, root, field_path, &paren.elem, index, seen, out);
        }
        syn::Type::Group(group) => {
            expand_type(server_fn, root, field_path, &group.elem, index, seen, out);
        }
        syn::Type::Array(array) => {
            expand_type(server_fn, root, field_path, &array.elem, index, seen, out);
        }
        syn::Type::Slice(slice) => {
            expand_type(server_fn, root, field_path, &slice.elem, index, seen, out);
        }
        syn::Type::Tuple(tuple) => {
            for (i, elem) in tuple.elems.iter().enumerate() {
                field_path.push(i.to_string());
                expand_type(server_fn, root, field_path, elem, index, seen, out);
                field_path.pop();
            }
        }
        syn::Type::Path(_) => {
            let mut inners = Vec::new();
            if inner_container_types(ty, &mut inners) {
                for inner in inners {
                    expand_type(server_fn, root, field_path, inner, index, seen, out);
                }
                return;
            }

            let Some(name) = type_name(ty) else {
                return;
            };
            if let Some(fields) = index.aggregates.get(&name) {
                if !seen.insert(name.clone()) {
                    return;
                }
                for field in fields {
                    field_path.push(field.name.clone());
                    expand_type(server_fn, root, field_path, &field.ty, index, seen, out);
                    field_path.pop();
                }
                seen.remove(&name);
                return;
            }
            out.push(WireInput {
                server_fn: server_fn.to_string(),
                root: root.to_string(),
                field_path: field_path.clone(),
                ty: name,
            });
        }
        _ => {
            out.push(WireInput {
                server_fn: server_fn.to_string(),
                root: root.to_string(),
                field_path: field_path.clone(),
                ty: ty.to_token_stream().to_string().replace(' ', ""),
            });
        }
    }
}

fn wire_inputs(
    web_sources: &[(String, String)],
    common_sources: &[(String, String)],
) -> Result<Vec<WireInput>, String> {
    let mut indexed_sources = web_sources.to_vec();
    indexed_sources.extend_from_slice(common_sources);
    let index = index_sources(&indexed_sources)?;
    let mut out = Vec::new();
    for (path, src) in web_sources {
        let fns = web_server_fns::server_fns_in(src).map_err(|e| format!("{path}: {e}"))?;
        for f in fns {
            for param in &f.params {
                let root = param
                    .name
                    .clone()
                    .unwrap_or_else(|| "<pattern>".to_string());
                expand_type(
                    &f.ident,
                    &root,
                    &mut Vec::new(),
                    &param.ty,
                    &index,
                    &mut BTreeSet::new(),
                    &mut out,
                );
            }
        }
    }
    out.sort();
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalDisplayCategory {
    TelemetrySafe,
    UserFacingOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AllowedExternalDisplay {
    wire_type: &'static str,
    error_type: &'static str,
    wrapped_type: &'static str,
    crate_name: &'static str,
    crate_version: &'static str,
    category: ExternalDisplayCategory,
    reason: &'static str,
}

const ALLOWED_EXTERNAL_DISPLAYS: &[AllowedExternalDisplay] = &[
    AllowedExternalDisplay {
        wire_type: "Email",
        error_type: "InvalidEmail",
        wrapped_type: "email_address::Error",
        crate_name: "email_address",
        crate_version: "0.2.9",
        category: ExternalDisplayCategory::TelemetrySafe,
        reason: "email_address 0.2.9 Error is a unit-variant enum whose Display emits literals and constants only; re-review on version change (#846)",
    },
    AllowedExternalDisplay {
        wire_type: "BackupSchedule",
        error_type: "InvalidBackupSchedule",
        wrapped_type: "croner::errors::CronError",
        crate_name: "croner",
        crate_version: "2.2.0",
        category: ExternalDisplayCategory::UserFacingOnly,
        reason: "croner's detailed schedule parse message is useful user feedback, but decode telemetry is source-sanitized; re-review on croner version change (#846)",
    },
];

fn cargo_lock_version(lockfile: &str, crate_name: &str) -> Option<String> {
    let mut in_package = false;
    let mut current_name = None;
    for line in lockfile.lines().map(str::trim) {
        if line == "[[package]]" {
            in_package = true;
            current_name = None;
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(value) = line.strip_prefix("name = ") {
            current_name = Some(value.trim_matches('"').to_string());
            continue;
        }
        if current_name.as_deref() == Some(crate_name)
            && let Some(value) = line.strip_prefix("version = ")
        {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

fn decode_telemetry_is_sanitized(server_error_src: &str) -> bool {
    let Ok(file) = syn::parse_file(server_error_src) else {
        return false;
    };
    let Some(body) = file.items.into_iter().find_map(|item| match item {
        syn::Item::Fn(item) if item.sig.ident == "emit_arg_decode_failure" => {
            Some(item.block.to_token_stream().to_string())
        }
        _ => None,
    }) else {
        return false;
    };
    body.contains("InternalError :: validation")
        && !body.contains("validation_source")
        && !body.contains("InternalError :: masked")
        && !body.contains("value . clone")
        && !body.contains("anyhow :: Error :: new")
}

fn field_type_path(ty: &str) -> String {
    ty.replace(' ', "")
}

fn variant_wraps_external(variant: &ErrorVariant, wrapped_type: &str) -> bool {
    let wrapped_type = field_type_path(wrapped_type);
    variant
        .fields
        .values()
        .any(|ty| field_type_path(ty) == wrapped_type)
}

fn reachable_error_variants<'a>(
    wire_type: &str,
    error: &'a ErrorType,
    index: &TypeIndex,
) -> Vec<&'a ErrorVariant> {
    let reachable_key = (wire_type.to_string(), error.name.clone());
    match index
        .reachable_variants
        .get(&reachable_key)
        .filter(|variants| !variants.is_empty())
    {
        Some(reachable) => error
            .variants
            .iter()
            .filter(|variant| reachable.contains(&variant.name))
            .collect(),
        None => error.variants.iter().collect(),
    }
}

fn strictly_reachable_error_variants<'a>(
    wire_type: &str,
    error: &'a ErrorType,
    index: &TypeIndex,
) -> Vec<&'a ErrorVariant> {
    let reachable_key = (wire_type.to_string(), error.name.clone());
    let Some(reachable) = index
        .reachable_variants
        .get(&reachable_key)
        .filter(|variants| !variants.is_empty())
    else {
        return Vec::new();
    };
    error
        .variants
        .iter()
        .filter(|variant| reachable.contains(&variant.name))
        .collect()
}

fn allowlist_entry_is_live(
    entry: &AllowedExternalDisplay,
    inputs: &[WireInput],
    index: &TypeIndex,
) -> bool {
    inputs.iter().any(|input| {
        if input.ty != entry.wire_type {
            return false;
        }
        let Some(error) = from_str_error_for(&input.ty, index) else {
            return false;
        };
        error.name == entry.error_type
            && strictly_reachable_error_variants(&input.ty, &error, index)
                .iter()
                .any(|variant| variant_wraps_external(variant, entry.wrapped_type))
    })
}

fn variant_is_allowlisted_external(
    wire_type: &str,
    error_name: &str,
    variant: &ErrorVariant,
    allowlist: &[AllowedExternalDisplay],
) -> bool {
    allowlist.iter().any(|entry| {
        entry.wire_type == wire_type
            && entry.error_type == error_name
            && variant_wraps_external(variant, entry.wrapped_type)
    })
}

fn validate_allowlist(
    inputs: &[WireInput],
    index: &TypeIndex,
    lockfile: &str,
    server_error_src: &str,
    allowlist: &[AllowedExternalDisplay],
) -> Result<(), Vec<String>> {
    let mut problems = Vec::new();
    let reachable_wire_types = inputs
        .iter()
        .map(|input| input.ty.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for entry in allowlist {
        let key = (entry.wire_type, entry.error_type, entry.wrapped_type);
        if !seen.insert(key) {
            problems.push(format!(
                "duplicate allowlist entry for {} -> {}",
                entry.wire_type, entry.error_type
            ));
        }
        if entry.reason.trim().is_empty() {
            problems.push(format!(
                "blank allowlist reason for {} -> {}",
                entry.wire_type, entry.error_type
            ));
        }
        if !reachable_wire_types.contains(entry.wire_type)
            || !allowlist_entry_is_live(entry, inputs, index)
        {
            problems.push(format!(
                "stale allowlist entry for unreachable external display {} -> {}({})",
                entry.wire_type, entry.error_type, entry.wrapped_type
            ));
        }
        match cargo_lock_version(lockfile, entry.crate_name) {
            Some(version) if version == entry.crate_version => {}
            Some(version) => problems.push(format!(
                "{} version drift: allowlist has {}, lockfile has {}",
                entry.crate_name, entry.crate_version, version
            )),
            None => problems.push(format!("{} is missing from Cargo.lock", entry.crate_name)),
        }
        if entry.category == ExternalDisplayCategory::UserFacingOnly
            && !decode_telemetry_is_sanitized(server_error_src)
        {
            problems.push(format!(
                "{} -> {} is user-facing-only but decode telemetry preserves source",
                entry.wire_type, entry.error_type
            ));
        }
    }

    for input in inputs {
        let Some(error) = from_str_error_for(&input.ty, index) else {
            continue;
        };
        for variant in reachable_error_variants(&input.ty, &error, index) {
            let mut singleton = BTreeSet::new();
            singleton.insert(variant.name.clone());
            let class = display_classification_for_variants(&error, Some(&singleton));
            if matches!(class, DisplayClass::TelemetrySafe) {
                continue;
            }
            if !variant_is_allowlisted_external(&input.ty, &error.name, variant, allowlist) {
                problems.push(format!(
                    "{} uses unsafe display {}::{} without a matching external allowlist entry",
                    input.ty, error.name, variant.name
                ));
            }
        }
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn read_sources(root: &str) -> Result<Vec<(String, String)>, String> {
    let paths = files::with_extension(Path::new(root), "rs")
        .map_err(|e| format!("cannot scan {root}: {e}"))?;
    paths
        .into_iter()
        .map(|path| {
            let display = path.display().to_string();
            let src = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            Ok((display, src))
        })
        .collect()
}

pub fn run(result: &mut CommandResult) {
    let web_sources = match read_sources(web_server_fns::WEB_SRC) {
        Ok(sources) => sources,
        Err(e) => {
            result.push(StepResult::fail("server-fn-wire-arg-error").detail(e));
            return;
        }
    };
    let common_sources = match read_sources("common/src") {
        Ok(sources) => sources,
        Err(e) => {
            result.push(StepResult::fail("server-fn-wire-arg-error").detail(e));
            return;
        }
    };
    let mut index_sources_input = web_sources.clone();
    index_sources_input.extend_from_slice(&common_sources);
    let index = match index_sources(&index_sources_input) {
        Ok(index) => index,
        Err(e) => {
            result.push(StepResult::fail("server-fn-wire-arg-error").detail(e));
            return;
        }
    };
    let inputs = match wire_inputs(&web_sources, &common_sources) {
        Ok(inputs) => inputs,
        Err(e) => {
            result.push(StepResult::fail("server-fn-wire-arg-error").detail(e));
            return;
        }
    };
    let lockfile = match std::fs::read_to_string("Cargo.lock") {
        Ok(lockfile) => lockfile,
        Err(e) => {
            result.push(
                StepResult::fail("server-fn-wire-arg-error")
                    .detail(format!("cannot read Cargo.lock: {e}")),
            );
            return;
        }
    };
    let server_error_src = match std::fs::read_to_string("web/src/error/server.rs") {
        Ok(src) => src,
        Err(e) => {
            result.push(
                StepResult::fail("server-fn-wire-arg-error")
                    .detail(format!("cannot read web/src/error/server.rs: {e}")),
            );
            return;
        }
    };
    match validate_allowlist(
        &inputs,
        &index,
        &lockfile,
        &server_error_src,
        ALLOWED_EXTERNAL_DISPLAYS,
    ) {
        Ok(()) => result.push(StepResult::ok("server-fn-wire-arg-error")),
        Err(problems) => {
            result.push(StepResult::fail("server-fn-wire-arg-error").detail(problems.join("\n")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web(src: &str) -> Vec<(String, String)> {
        vec![("web/src/auth/api.rs".to_string(), src.to_string())]
    }

    fn common(src: &str) -> Vec<(String, String)> {
        vec![("common/src/dto.rs".to_string(), src.to_string())]
    }

    fn tys(inputs: &[WireInput]) -> Vec<String> {
        inputs.iter().map(|i| i.ty.clone()).collect()
    }

    #[test]
    fn direct_server_arg_is_a_wire_leaf() {
        let inputs = wire_inputs(
            &web("#[macros::server]\npub async fn update(email: Email) -> R { todo!() }"),
            &[],
        )
        .expect("wire inputs");

        assert_eq!(tys(&inputs), vec!["Email"]);
    }

    #[test]
    fn nested_request_struct_fields_are_wire_leaves() {
        let inputs = wire_inputs(
            &web(
                "pub struct LoginRequest { pub password: ProfferedPassword }\n\
                 #[macros::server]\npub async fn login(request: LoginRequest) -> R { todo!() }",
            ),
            &[],
        )
        .expect("wire inputs");

        assert_eq!(tys(&inputs), vec!["ProfferedPassword"]);
        assert_eq!(inputs[0].field_path, vec!["password"]);
    }

    #[test]
    fn aggregate_expansion_does_not_depend_on_request_suffix() {
        let inputs = wire_inputs(
            &web("pub struct PostInputs { pub title: PostTitle }\n\
                 #[macros::server]\npub async fn preview(post: PostInputs) -> R { todo!() }"),
            &[],
        )
        .expect("wire inputs");

        assert_eq!(tys(&inputs), vec!["PostTitle"]);
    }

    #[test]
    fn containers_are_unwrapped_to_wire_leaves() {
        let inputs = wire_inputs(
            &web(
                "pub struct Dto { pub destination: Option<DestinationPath>, pub emails: Vec<Email> }\n\
                 #[macros::server]\npub async fn save(dto: Dto) -> R { todo!() }",
            ),
            &[],
        )
        .expect("wire inputs");

        assert_eq!(tys(&inputs), vec!["DestinationPath", "Email"]);
    }

    #[test]
    fn tuple_newtypes_stay_wire_leaves() {
        let inputs = wire_inputs(
            &web("pub struct Email(String);\n\
                 #[macros::server]\npub async fn update(email: Email) -> R { todo!() }"),
            &[],
        )
        .expect("wire inputs");

        assert_eq!(tys(&inputs), vec!["Email"]);
    }

    #[test]
    fn enums_stay_wire_leaves() {
        let inputs = wire_inputs(
            &web("pub enum BackupMode { Manual, Scheduled }\n\
                 #[macros::server]\npub async fn update(mode: BackupMode) -> R { todo!() }"),
            &[],
        )
        .expect("wire inputs");

        assert_eq!(tys(&inputs), vec!["BackupMode"]);
    }

    fn class_for(src: &str, ty: &str) -> DisplayClass {
        let sources = common(src);
        let index = index_sources(&sources).expect("index");
        let error = from_str_error_for(ty, &index).expect("from_str error");
        display_classification(ty, &error, &index)
    }

    #[test]
    fn literal_error_display_is_telemetry_safe() {
        let class = class_for(
            r#"#[derive(thiserror::Error, Debug)]
               pub enum PostTitleError { #[error("post title must be non-empty")] Empty }
               pub struct PostTitle(String);
               impl std::str::FromStr for PostTitle {
                   type Err = PostTitleError;
                   fn from_str(s: &str) -> Result<Self, Self::Err> {
                       Err(PostTitleError::Empty)
                   }
               }"#,
            "PostTitle",
        );

        assert_eq!(class, DisplayClass::TelemetrySafe);
    }

    #[test]
    fn const_placeholder_error_display_is_telemetry_safe() {
        let class = class_for(
            r#"const MIN_LENGTH: usize = 8;
               #[derive(thiserror::Error, Debug)]
               pub enum PasswordError {
                   #[error("password must be at least {MIN_LENGTH} characters")]
                   TooShort,
               }
               pub struct ProfferedPassword(String);
               impl std::str::FromStr for ProfferedPassword {
                   type Err = PasswordError;
                   fn from_str(s: &str) -> Result<Self, Self::Err> {
                       Err(PasswordError::TooShort)
                   }
               }"#,
            "ProfferedPassword",
        );

        assert_eq!(class, DisplayClass::TelemetrySafe);
    }

    #[test]
    fn numeric_scalar_field_error_display_is_telemetry_safe() {
        let class = class_for(
            r#"#[derive(thiserror::Error, Debug)]
               pub enum FilenameError {
                   #[error("filename too long: {encoded} bytes")]
                   TooLong { encoded: usize },
               }
               pub struct Filename(String);
               impl std::str::FromStr for Filename {
                   type Err = FilenameError;
                   fn from_str(s: &str) -> Result<Self, Self::Err> {
                       Err(FilenameError::TooLong { encoded: s.len() })
                   }
               }"#,
            "Filename",
        );

        assert_eq!(class, DisplayClass::TelemetrySafe);
    }

    #[test]
    fn tuple_string_interpolation_is_unsafe() {
        let class = class_for(
            r#"#[derive(thiserror::Error, Debug)]
               #[error("bad {0}")]
               pub struct Bad(String);
               pub struct Email(String);
               impl std::str::FromStr for Email {
                   type Err = Bad;
                   fn from_str(s: &str) -> Result<Self, Self::Err> {
                       Err(Bad(s.to_string()))
                   }
               }"#,
            "Email",
        );

        assert!(matches!(class, DisplayClass::Unsafe(_)));
    }

    #[test]
    fn named_string_debug_interpolation_is_unsafe() {
        let class = class_for(
            r#"#[derive(thiserror::Error, Debug)]
               pub enum Bad {
                   #[error("bad {value:?}")]
                   Value { value: String },
               }
               pub struct Email(String);
               impl std::str::FromStr for Email {
                   type Err = Bad;
                   fn from_str(s: &str) -> Result<Self, Self::Err> {
                       Err(Bad::Value { value: s.to_string() })
                   }
               }"#,
            "Email",
        );

        assert!(matches!(class, DisplayClass::Unsafe(_)));
    }

    #[test]
    fn transparent_error_is_unsafe_without_proven_inner_type() {
        let class = class_for(
            r#"#[derive(thiserror::Error, Debug)]
               pub enum Bad {
                   #[error(transparent)]
                   Inner(InnerError),
               }
               #[derive(thiserror::Error, Debug)]
               #[error("inner")]
               pub struct InnerError;
               pub struct Email(String);
               impl std::str::FromStr for Email {
                   type Err = Bad;
                   fn from_str(s: &str) -> Result<Self, Self::Err> {
                       Err(Bad::Inner(InnerError))
                   }
               }"#,
            "Email",
        );

        assert!(matches!(class, DisplayClass::Unsafe(_)));
    }

    #[test]
    fn password_from_str_does_not_accept_unreachable_variants() {
        let class = class_for(
            r#"const MIN_LENGTH: usize = 8;
               const MAX_LENGTH: usize = 128;
               #[derive(thiserror::Error, Debug)]
               pub enum PasswordError {
                   #[error("password must be at least {MIN_LENGTH} characters")]
                   PasswordTooShort,
                   #[error("password must be at most {MAX_LENGTH} characters")]
                   PasswordTooLong,
                   #[error("verification failed: {0}")]
                   VerificationFailed(String),
                   #[error("hashing failed: {source}")]
                   HashingFailed { source: String },
               }
               pub struct ProfferedPassword(String);
               fn validate_password_shape(_: &str) -> Result<(), PasswordError> {
                   Err(PasswordError::PasswordTooShort)
               }
               impl std::str::FromStr for ProfferedPassword {
                   type Err = PasswordError;
                   fn from_str(s: &str) -> Result<Self, Self::Err> {
                       validate_password_shape(s)?;
                       Ok(ProfferedPassword(s.to_string()))
                   }
               }"#,
            "ProfferedPassword",
        );

        assert_eq!(class, DisplayClass::TelemetrySafe);
    }

    fn sanitized_server_error() -> &'static str {
        r#"fn emit_arg_decode_failure(value: &ServerFnErrorErr) {
               InternalError::validation("invalid request arguments").emit_boundary_failure();
           }"#
    }

    fn preserving_server_error() -> &'static str {
        r#"fn emit_arg_decode_failure(value: &ServerFnErrorErr) {
               InternalError::validation_source("invalid request arguments", value.clone())
                   .emit_boundary_failure();
           }"#
    }

    fn masked_server_error() -> &'static str {
        r#"fn emit_arg_decode_failure(value: &ServerFnErrorErr) {
               InternalError::validation("invalid request arguments").emit_boundary_failure();
               InternalError::masked(
                   ErrorKind::Validation,
                   ErrorClass::Client,
                   "invalid request arguments",
                   anyhow::Error::new(value.clone()),
               ).emit_boundary_failure();
           }"#
    }

    fn lock_with(name: &str, version: &str) -> String {
        format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n")
    }

    fn external_wrapper_sources() -> Vec<(String, String)> {
        common(
            r#"#[derive(thiserror::Error, Debug)]
               pub enum InvalidEmail {
                   #[error(transparent)]
                   Address(email_address::Error),
               }
               pub struct Email(String);
               impl std::str::FromStr for Email {
                   type Err = InvalidEmail;
                   fn from_str(_: &str) -> Result<Self, Self::Err> { Err(InvalidEmail::Address(todo!())) }
               }"#,
        )
    }

    fn backup_wrapper_sources() -> Vec<(String, String)> {
        common(
            r#"#[derive(thiserror::Error, Debug)]
               pub enum InvalidBackupSchedule {
                   #[error(transparent)]
                   Cron(croner::errors::CronError),
               }
               pub struct BackupSchedule(String);
               impl std::str::FromStr for BackupSchedule {
                   type Err = InvalidBackupSchedule;
                   fn from_str(_: &str) -> Result<Self, Self::Err> { Err(InvalidBackupSchedule::Cron(todo!())) }
               }"#,
        )
    }

    fn one_input(ty: &str) -> Vec<WireInput> {
        vec![WireInput {
            server_fn: "save".to_string(),
            root: ty.to_ascii_lowercase(),
            field_path: Vec::new(),
            ty: ty.to_string(),
        }]
    }

    #[test]
    fn external_wrapper_fails_without_allowlist_entry() {
        let sources = external_wrapper_sources();
        let index = index_sources(&sources).expect("index");
        let err = validate_allowlist(
            &one_input("Email"),
            &index,
            &lock_with("email_address", "0.2.9"),
            sanitized_server_error(),
            &[],
        )
        .unwrap_err();

        assert!(
            err.join("\n")
                .contains("without a matching external allowlist entry"),
            "{err:?}"
        );
    }

    #[test]
    fn telemetry_safe_allowlist_passes_with_matching_lockfile() {
        let sources = external_wrapper_sources();
        let index = index_sources(&sources).expect("index");
        let allowlist = [ALLOWED_EXTERNAL_DISPLAYS[0].clone()];

        validate_allowlist(
            &one_input("Email"),
            &index,
            &lock_with("email_address", "0.2.9"),
            sanitized_server_error(),
            &allowlist,
        )
        .expect("allowlist valid");
    }

    #[test]
    fn allowlist_does_not_match_same_last_segment_from_wrong_crate() {
        let sources = common(
            r#"#[derive(thiserror::Error, Debug)]
               pub enum InvalidEmail {
                   #[error(transparent)]
                   Address(other_crate::Error),
               }
               pub struct Email(String);
               impl std::str::FromStr for Email {
                   type Err = InvalidEmail;
                   fn from_str(_: &str) -> Result<Self, Self::Err> { todo!() }
               }"#,
        );
        let index = index_sources(&sources).expect("index");
        let allowlist = [ALLOWED_EXTERNAL_DISPLAYS[0].clone()];
        let err = validate_allowlist(
            &one_input("Email"),
            &index,
            &lock_with("email_address", "0.2.9"),
            sanitized_server_error(),
            &allowlist,
        )
        .unwrap_err()
        .join("\n");

        assert!(
            err.contains("without a matching external allowlist entry"),
            "{err}"
        );
    }

    #[test]
    fn allowlist_does_not_cover_unrelated_unsafe_variant_in_same_error_enum() {
        let sources = common(
            r#"#[derive(thiserror::Error, Debug)]
               pub enum InvalidEmail {
                   #[error(transparent)]
                   Address(email_address::Error),
                   #[error("bad {value}")]
                   BadValue { value: String },
               }
               pub struct Email(String);
               impl std::str::FromStr for Email {
                   type Err = InvalidEmail;
                   fn from_str(_: &str) -> Result<Self, Self::Err> {
                       Err(InvalidEmail::BadValue { value: String::new() })
                   }
               }"#,
        );
        let index = index_sources(&sources).expect("index");
        let allowlist = [ALLOWED_EXTERNAL_DISPLAYS[0].clone()];
        let err = validate_allowlist(
            &one_input("Email"),
            &index,
            &lock_with("email_address", "0.2.9"),
            sanitized_server_error(),
            &allowlist,
        )
        .unwrap_err()
        .join("\n");

        assert!(err.contains("InvalidEmail::BadValue"), "{err}");
    }

    #[test]
    fn allowlist_is_stale_when_wire_type_uses_different_wrapper() {
        let sources = common(
            r#"#[derive(thiserror::Error, Debug)]
               pub enum InvalidEmail {
                   #[error(transparent)]
                   Address(other_crate::Error),
               }
               pub struct Email(String);
               impl std::str::FromStr for Email {
                   type Err = InvalidEmail;
                   fn from_str(_: &str) -> Result<Self, Self::Err> { todo!() }
               }"#,
        );
        let index = index_sources(&sources).expect("index");
        let allowlist = [ALLOWED_EXTERNAL_DISPLAYS[0].clone()];
        let err = validate_allowlist(
            &one_input("Email"),
            &index,
            &lock_with("email_address", "0.2.9"),
            sanitized_server_error(),
            &allowlist,
        )
        .unwrap_err()
        .join("\n");

        assert!(err.contains("stale allowlist entry"), "{err}");
    }

    #[test]
    fn user_facing_only_allowlist_requires_sanitized_decode_telemetry() {
        let sources = backup_wrapper_sources();
        let index = index_sources(&sources).expect("index");
        let allowlist = [ALLOWED_EXTERNAL_DISPLAYS[1].clone()];

        validate_allowlist(
            &one_input("BackupSchedule"),
            &index,
            &lock_with("croner", "2.2.0"),
            sanitized_server_error(),
            &allowlist,
        )
        .expect("sanitized telemetry permits user-facing-only allowlist");

        let err = validate_allowlist(
            &one_input("BackupSchedule"),
            &index,
            &lock_with("croner", "2.2.0"),
            preserving_server_error(),
            &allowlist,
        )
        .unwrap_err();

        assert!(err.join("\n").contains("preserves source"), "{err:?}");

        let err = validate_allowlist(
            &one_input("BackupSchedule"),
            &index,
            &lock_with("croner", "2.2.0"),
            masked_server_error(),
            &allowlist,
        )
        .unwrap_err();

        assert!(err.join("\n").contains("preserves source"), "{err:?}");
    }
    #[test]
    fn allowlist_blank_reason_duplicate_version_drift_and_stale_entries_fail() {
        let sources = external_wrapper_sources();
        let index = index_sources(&sources).expect("index");
        let entry = AllowedExternalDisplay {
            reason: "",
            crate_version: "9.9.9",
            ..ALLOWED_EXTERNAL_DISPLAYS[0].clone()
        };
        let stale = AllowedExternalDisplay {
            wire_type: "Unreachable",
            ..ALLOWED_EXTERNAL_DISPLAYS[0].clone()
        };
        let err = validate_allowlist(
            &one_input("Email"),
            &index,
            &lock_with("email_address", "0.2.9"),
            sanitized_server_error(),
            &[entry.clone(), entry, stale],
        )
        .unwrap_err()
        .join("\n");

        assert!(err.contains("blank allowlist reason"), "{err}");
        assert!(err.contains("duplicate allowlist entry"), "{err}");
        assert!(err.contains("version drift"), "{err}");
        assert!(err.contains("stale allowlist entry"), "{err}");
    }

    #[test]
    fn backup_schedule_allowlist_fails_on_croner_version_drift() {
        let sources = backup_wrapper_sources();
        let index = index_sources(&sources).expect("index");
        let allowlist = [ALLOWED_EXTERNAL_DISPLAYS[1].clone()];
        let err = validate_allowlist(
            &one_input("BackupSchedule"),
            &index,
            &lock_with("croner", "2.3.0"),
            sanitized_server_error(),
            &allowlist,
        )
        .unwrap_err()
        .join("\n");

        assert!(err.contains("croner version drift"), "{err}");
    }

    #[test]
    fn return_type_is_ignored() {
        let inputs = wire_inputs(
            &web("#[macros::server]\npub async fn get() -> WebResult<LoginRequest> { todo!() }"),
            &common("pub struct LoginRequest { pub password: ProfferedPassword }"),
        )
        .expect("wire inputs");

        assert!(inputs.is_empty());
    }

    #[test]
    fn unparsable_source_is_an_error() {
        let err = wire_inputs(&web("pub struct Broken {"), &[]).unwrap_err();

        assert!(err.contains("cannot parse"), "{err}");
    }
}
