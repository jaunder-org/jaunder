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
    owned_error_surfaces: BTreeSet<String>,
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

fn returns_str_reference(output: &syn::ReturnType, require_static: bool) -> bool {
    let syn::ReturnType::Type(_, ty) = output else {
        return false;
    };
    let syn::Type::Reference(reference) = ty.as_ref() else {
        return false;
    };
    if require_static
        && reference
            .lifetime
            .as_ref()
            .is_none_or(|lifetime| lifetime.ident != "static")
    {
        return false;
    }
    reference.mutability.is_none()
        && matches!(
            reference.elem.as_ref(),
            syn::Type::Path(path) if path.path.is_ident("str")
        )
}

fn has_immutable_self_receiver(signature: &syn::Signature) -> bool {
    signature.inputs.len() == 1
        && matches!(
            signature.inputs.first(),
            Some(syn::FnArg::Receiver(receiver))
                if receiver
                    .reference
                    .as_ref()
                    .is_some_and(|(_, lifetime)| lifetime.is_none())
                    && receiver.mutability.is_none()
                    && receiver.colon_token.is_none()
        )
}

fn has_conditional_compilation(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
}

fn is_plain_safe_method(signature: &syn::Signature) -> bool {
    signature.constness.is_none()
        && signature.asyncness.is_none()
        && signature.unsafety.is_none()
        && signature.abi.is_none()
        && signature.variadic.is_none()
        && signature.generics.params.is_empty()
        && signature.generics.where_clause.is_none()
}

fn record_owned_error_surface(index: &mut TypeIndex, item: &syn::ItemImpl) {
    if item.trait_.is_some() || has_conditional_compilation(&item.attrs) {
        return;
    }
    let Some(error_name) = type_name(&item.self_ty) else {
        return;
    };
    let public_method = |name: &str, require_static: bool| {
        item.items.iter().any(|item| {
            let syn::ImplItem::Fn(item) = item else {
                return false;
            };
            matches!(item.vis, syn::Visibility::Public(_))
                && item.sig.ident == name
                && has_immutable_self_receiver(&item.sig)
                && is_plain_safe_method(&item.sig)
                && !has_conditional_compilation(&item.attrs)
                && returns_str_reference(&item.sig.output, require_static)
        })
    };
    if public_method("user_message", false) && public_method("telemetry_code", true) {
        index.owned_error_surfaces.insert(error_name);
    }
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

#[cfg(test)]
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
                syn::Item::Impl(item) => {
                    record_owned_error_surface(&mut index, &item);
                    record_from_str_impl(&mut index, &item);
                }
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

const EXTERNAL_FORMAT_MARKER: &str = "// server-fn-wire-arg-error:allow";
#[derive(Debug, Clone, Copy)]
struct ExternalFormatCall {
    line: usize,
    literal_argument: bool,
}

#[derive(Default)]
struct ExternalFormatScan {
    calls: Vec<ExternalFormatCall>,
    indirect_references: Vec<usize>,
}

#[derive(Debug, Clone)]
enum FlatToken {
    Ident(String, usize),
    Literal,
    Punct(char),
    Open(proc_macro2::Delimiter),
    Close,
}

fn flatten_tokens(tokens: proc_macro2::TokenStream, out: &mut Vec<FlatToken>) {
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Ident(ident) => {
                out.push(FlatToken::Ident(
                    ident.to_string(),
                    ident.span().start().line,
                ));
            }
            proc_macro2::TokenTree::Literal(_) => out.push(FlatToken::Literal),
            proc_macro2::TokenTree::Punct(punct) => out.push(FlatToken::Punct(punct.as_char())),
            proc_macro2::TokenTree::Group(group) => {
                out.push(FlatToken::Open(group.delimiter()));
                flatten_tokens(group.stream(), out);
                out.push(FlatToken::Close);
            }
        }
    }
}

fn external_format_calls(file: &syn::File) -> ExternalFormatScan {
    let mut tokens = Vec::new();
    flatten_tokens(file.to_token_stream(), &mut tokens);
    let mut scan = ExternalFormatScan::default();

    for (index, token) in tokens.iter().enumerate() {
        let FlatToken::Ident(name, line) = token else {
            continue;
        };
        if name != "from_external" {
            continue;
        }
        if !matches!(
            (
                tokens.get(index.wrapping_sub(2)),
                tokens.get(index.wrapping_sub(1))
            ),
            (Some(FlatToken::Punct(':')), Some(FlatToken::Punct(':')))
        ) {
            continue;
        }
        let mut call_open = index + 1;
        while matches!(tokens.get(call_open), Some(FlatToken::Close)) {
            call_open += 1;
        }
        if !matches!(
            tokens.get(call_open),
            Some(FlatToken::Open(proc_macro2::Delimiter::Parenthesis))
        ) {
            scan.indirect_references.push(*line);
            continue;
        }
        let mut argument = call_open + 1;
        while matches!(
            tokens.get(argument),
            Some(FlatToken::Open(proc_macro2::Delimiter::Parenthesis))
        ) {
            argument += 1;
        }
        scan.calls.push(ExternalFormatCall {
            line: *line,
            literal_argument: matches!(tokens.get(argument), Some(FlatToken::Literal)),
        });
    }
    scan
}

fn external_format_marker_reason(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix(EXTERNAL_FORMAT_MARKER)?;
    if rest.is_empty() {
        return Some("");
    }
    rest.chars()
        .next()
        .is_some_and(char::is_whitespace)
        .then(|| rest.trim())
}

/// Census the reviewed doors that turn foreign `Display` into a user-only
/// message. Calls are found structurally; marker adjacency remains deliberately
/// line-local: one non-empty reason immediately before one non-literal call.
fn external_format_doors(sources: &[(String, String)]) -> Result<Vec<String>, Vec<String>> {
    let mut doors = Vec::new();
    let mut problems = Vec::new();

    for (path, source) in sources {
        let lines = source.lines().collect::<Vec<_>>();
        let file = match syn::parse_file(source) {
            Ok(file) => file,
            Err(error) => {
                problems.push(format!(
                    "{path}: cannot parse external formatter census: {error}"
                ));
                continue;
            }
        };
        let scan = external_format_calls(&file);
        for line in scan.indirect_references {
            problems.push(format!(
                "{path}:{line}: indirect reference to external formatter is forbidden; call it directly behind one marker"
            ));
        }
        let mut calls = BTreeMap::<usize, Vec<bool>>::new();
        for call in scan.calls {
            calls
                .entry(call.line)
                .or_default()
                .push(call.literal_argument);
        }

        for (line, literal_arguments) in &calls {
            let marker = line
                .checked_sub(2)
                .and_then(|index| lines.get(index))
                .and_then(|line| external_format_marker_reason(line));
            let Some(reason) = marker else {
                let shared = *line >= 3
                    && calls.contains_key(&(line - 1))
                    && lines[*line - 3].trim().starts_with(EXTERNAL_FORMAT_MARKER);
                problems.push(format!(
                    "{path}:{line}: {} external formatter call",
                    if shared {
                        "shared marker cannot cover"
                    } else {
                        "unmarked"
                    }
                ));
                continue;
            };
            if reason.trim().is_empty() {
                problems.push(format!(
                    "{path}:{}: bare external formatter marker",
                    line - 1
                ));
                continue;
            }
            if literal_arguments.len() != 1 {
                problems.push(format!(
                    "{path}:{line}: shared marker cannot cover multiple external formatter calls"
                ));
                continue;
            }
            if literal_arguments[0] {
                problems.push(format!(
                    "{path}:{}: stale external formatter marker wraps an owned literal",
                    line - 1
                ));
                continue;
            }
            doors.push(format!("{path}:{line}"));
        }

        for (index, line) in lines.iter().enumerate() {
            let Some(reason) = external_format_marker_reason(line) else {
                continue;
            };
            if reason.trim().is_empty() {
                continue;
            }
            let marker_line = index + 1;
            let next_call_count = calls.get(&(marker_line + 1)).map_or(0, Vec::len);
            if next_call_count == 0 {
                let kind = if calls.contains_key(&marker_line.saturating_sub(1)) {
                    "trailing"
                } else {
                    "orphan"
                };
                problems.push(format!(
                    "{path}:{marker_line}: {kind} external formatter marker"
                ));
            } else if next_call_count > 1 {
                problems.push(format!(
                    "{path}:{marker_line}: shared external formatter marker"
                ));
            }
        }
    }

    doors.sort();
    problems.sort();
    if problems.is_empty() {
        Ok(doors)
    } else {
        Err(problems)
    }
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
    let internal_error_constructors = body.matches("InternalError ::").count();
    internal_error_constructors == 1
        && body.contains("InternalError :: validation")
        && !body.contains("validation_source")
        && !body.contains("value . clone")
        && !body.contains("anyhow :: Error :: new")
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

fn validate_owned_surfaces(
    inputs: &[WireInput],
    index: &TypeIndex,
    server_error_src: &str,
    sources: &[(String, String)],
) -> Result<(), Vec<String>> {
    let mut problems = external_format_doors(sources).err().unwrap_or_default();
    if !decode_telemetry_is_sanitized(server_error_src) {
        problems.push(
            "web/src/error/server.rs emit_arg_decode_failure must keep decode telemetry source-free"
                .to_string(),
        );
    }

    for input in inputs {
        let Some(error_name) = index.from_str_errors.get(&input.ty) else {
            continue;
        };
        if index.owned_error_surfaces.contains(error_name) {
            continue;
        }
        let Some(error) = index.errors.get(error_name) else {
            problems.push(format!(
                "server_fn={} root={} field_path={} type={} parse error {} has no owned \
                 user_message/telemetry_code surfaces",
                input.server_fn,
                input.root,
                if input.field_path.is_empty() {
                    "<root>".to_string()
                } else {
                    input.field_path.join(".")
                },
                input.ty,
                error_name
            ));
            continue;
        };
        for variant in reachable_error_variants(&input.ty, error, index) {
            let mut singleton = BTreeSet::new();
            singleton.insert(variant.name.clone());
            if matches!(
                display_classification_for_variants(error, Some(&singleton)),
                DisplayClass::TelemetrySafe
            ) {
                continue;
            }
            problems.push(format!(
                "server_fn={} root={} field_path={} type={} uses unsafe display {}::{} \
                 without owned user_message/telemetry_code surfaces",
                input.server_fn,
                input.root,
                if input.field_path.is_empty() {
                    "<root>".to_string()
                } else {
                    input.field_path.join(".")
                },
                input.ty,
                error.name,
                variant.name
            ));
        }
    }

    problems.sort();

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

const WORKSPACE_SOURCE_ROOTS: &[&str] = &[
    "client/src",
    "common/src",
    "csr/src",
    "host/src",
    "macros/src",
    "server/src",
    "storage/src",
    "test-support/src",
    "web/src",
];

fn read_workspace_sources() -> Result<Vec<(String, String)>, String> {
    let mut sources = Vec::new();
    for root in WORKSPACE_SOURCE_ROOTS {
        sources.extend(read_sources(root)?);
    }
    Ok(sources)
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
    let workspace_sources = match read_workspace_sources() {
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
    match validate_owned_surfaces(&inputs, &index, &server_error_src, &workspace_sources) {
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

    fn server_constructor_error() -> &'static str {
        r#"fn emit_arg_decode_failure(value: &ServerFnErrorErr) {
               InternalError::validation("invalid request arguments").emit_boundary_failure();
               InternalError::server(ServerFnErrorErr::Args(value.to_string()))
                   .emit_boundary_failure();
           }"#
    }

    fn unsafe_error_sources() -> Vec<(String, String)> {
        common(
            r#"#[derive(thiserror::Error, Debug)]
               #[error("invalid value: {value}")]
               pub struct InvalidThing { value: String }
               pub struct Thing(String);
               impl std::str::FromStr for Thing {
                   type Err = InvalidThing;
                   fn from_str(_: &str) -> Result<Self, Self::Err> {
                       Err(InvalidThing { value: String::new() })
                   }
               }"#,
        )
    }

    fn owned_surface_sources() -> Vec<(String, String)> {
        common(
            r#"pub struct InvalidBackupSchedule(UserFacingMessage);
               impl InvalidBackupSchedule {
                   pub fn user_message(&self) -> &str { self.0.as_str() }
                   pub fn telemetry_code(&self) -> &'static str { "invalid_backup_schedule" }
               }
               pub struct BackupSchedule(String);
               impl std::str::FromStr for BackupSchedule {
                   type Err = InvalidBackupSchedule;
                   fn from_str(_: &str) -> Result<Self, Self::Err> { todo!() }
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

    fn nested_input(ty: &str) -> Vec<WireInput> {
        vec![WireInput {
            server_fn: "save_settings".to_string(),
            root: "config".to_string(),
            field_path: vec!["backup".to_string(), "schedule".to_string()],
            ty: ty.to_string(),
        }]
    }

    #[test]
    fn unsafe_display_without_owned_surfaces_names_the_wire_path() {
        let sources = unsafe_error_sources();
        let index = index_sources(&sources).expect("index");
        let error = validate_owned_surfaces(
            &nested_input("Thing"),
            &index,
            sanitized_server_error(),
            &sources,
        )
        .unwrap_err()
        .join("\n");

        assert!(error.contains("server_fn=save_settings"), "{error}");
        assert!(error.contains("root=config"), "{error}");
        assert!(error.contains("field_path=backup.schedule"), "{error}");
        assert!(error.contains("without owned user_message"), "{error}");
    }

    #[test]
    fn owned_user_and_telemetry_surfaces_replace_external_allowlists() {
        let sources = owned_surface_sources();
        let index = index_sources(&sources).expect("index");
        assert!(index.owned_error_surfaces.contains("InvalidBackupSchedule"));
        validate_owned_surfaces(
            &one_input("BackupSchedule"),
            &index,
            sanitized_server_error(),
            &sources,
        )
        .expect("owned sink surfaces are sufficient");
    }

    #[test]
    fn non_exact_methods_do_not_count_as_owned_error_surfaces() {
        let original = owned_surface_sources();
        let variants = [
            original
                .iter()
                .map(|(path, source)| (path.clone(), source.replace("(&self)", "()")))
                .collect::<Vec<_>>(),
            original
                .iter()
                .map(|(path, source)| {
                    (
                        path.clone(),
                        source.replace("pub fn user_message", "pub async fn user_message"),
                    )
                })
                .collect::<Vec<_>>(),
            original
                .iter()
                .map(|(path, source)| {
                    (
                        path.clone(),
                        source.replace("pub fn telemetry_code", "pub unsafe fn telemetry_code"),
                    )
                })
                .collect::<Vec<_>>(),
            original
                .iter()
                .map(|(path, source)| {
                    (
                        path.clone(),
                        source.replace(
                            "pub fn telemetry_code(&self) -> &'static str",
                            "pub fn telemetry_code(&self) -> &'static mut str",
                        ),
                    )
                })
                .collect::<Vec<_>>(),
            original
                .iter()
                .map(|(path, source)| {
                    (
                        path.clone(),
                        source.replace("user_message(&self)", "user_message<T>(&self)"),
                    )
                })
                .collect::<Vec<_>>(),
            original
                .iter()
                .map(|(path, source)| {
                    (
                        path.clone(),
                        source.replace(
                            "impl InvalidBackupSchedule",
                            "#[cfg(test)]\nimpl InvalidBackupSchedule",
                        ),
                    )
                })
                .collect::<Vec<_>>(),
            original
                .iter()
                .map(|(path, source)| {
                    (
                        path.clone(),
                        source.replace(
                            "pub fn user_message",
                            "#[cfg_attr(feature = \"hidden\", cfg(test))]\npub fn user_message",
                        ),
                    )
                })
                .collect::<Vec<_>>(),
        ];
        for sources in variants {
            let index = index_sources(&sources).expect("index");
            assert!(!index.owned_error_surfaces.contains("InvalidBackupSchedule"));
        }
    }

    #[test]
    fn owned_surfaces_still_require_source_free_decode_telemetry() {
        let sources = owned_surface_sources();
        let index = index_sources(&sources).expect("index");
        for server_error in [
            preserving_server_error(),
            masked_server_error(),
            server_constructor_error(),
        ] {
            let error = validate_owned_surfaces(
                &one_input("BackupSchedule"),
                &index,
                server_error,
                &sources,
            )
            .unwrap_err()
            .join("\n");
            assert!(error.contains("source-free"), "{error}");
        }
    }

    fn marker_errors(source: &str) -> String {
        let wrapped = format!("fn check() {{\n{source}\n}}");
        external_format_doors(&[("common/src/example.rs".to_string(), wrapped)])
            .unwrap_err()
            .join("\n")
    }
    #[test]
    fn marked_external_user_message_conversion_enters_the_census() {
        let sources = vec![(
            "common/src/backup.rs".to_string(),
            "fn check() {\n\
             // server-fn-wire-arg-error:allow detailed operator feedback\n\
             UserFacingMessage::from_external(format_args!(\"detail: {error}\"));\n\
             }"
            .to_string(),
        )];
        assert_eq!(
            external_format_doors(&sources).unwrap(),
            ["common/src/backup.rs:3"]
        );
    }

    #[test]
    fn external_user_message_markers_fail_closed() {
        let cases = [
            (
                "unmarked",
                "UserFacingMessage::from_external(format_args!(\"{error}\"));",
            ),
            (
                "unmarked",
                "<UserFacingMessage>::from_external(format_args!(\"{error}\"));",
            ),
            (
                "unmarked",
                "(UserFacingMessage::from_external)(format_args!(\"{error}\"));",
            ),
            (
                "unmarked",
                "let _ = vec![UserFacingMessage::from_external(format_args!(\"{error}\"))];",
            ),
            (
                "unmarked",
                "type Message = UserFacingMessage;\n\
                 Message::from_external(format_args!(\"{error}\"));",
            ),
            (
                "unmarked",
                "// server-fn-wire-arg-error:allowance typo\n\
                 UserFacingMessage::from_external(format_args!(\"{error}\"));",
            ),
            (
                "indirect",
                "let capture = UserFacingMessage::from_external;\n\
                 capture(format_args!(\"{error}\"));",
            ),
            (
                "bare",
                "// server-fn-wire-arg-error:allow\n\
                 UserFacingMessage::from_external(format_args!(\"{error}\"));",
            ),
            (
                "trailing",
                "UserFacingMessage::from_external(format_args!(\"{error}\"));\n\
                 // server-fn-wire-arg-error:allow too late",
            ),
            (
                "shared",
                "// server-fn-wire-arg-error:allow one call only\n\
                 UserFacingMessage::from_external(format_args!(\"{first}\"));\n\
                 UserFacingMessage::from_external(format_args!(\"{second}\"));",
            ),
            (
                "orphan",
                "// server-fn-wire-arg-error:allow no call follows\n\
                 let value = 1;",
            ),
            (
                "stale",
                "// server-fn-wire-arg-error:allow literals are already owned\n\
                 UserFacingMessage::from_external(\"fixed literal\");",
            ),
        ];
        for (expected, source) in cases {
            let errors = marker_errors(source);
            assert!(errors.contains(expected), "{expected}: {errors}");
        }
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
