//! The `server-fn-wire-arg-error` static check (#846): server-fn argument
//! decode telemetry must not export raw newtype parse errors.
//!
//! The check starts from the same `#[macros::server]` inventory as the registrar
//! and tracing gates, then expands local request aggregates to the leaf wire
//! types whose `FromStr::Err` displays can reach decode errors.

use std::collections::{BTreeMap, BTreeSet};

use quote::ToTokens;

use crate::web_server_fns;

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
}

#[derive(Clone)]
struct Field {
    name: String,
    ty: syn::Type,
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

fn index_sources(sources: &[(String, String)]) -> Result<TypeIndex, String> {
    let mut index = TypeIndex::default();
    for (path, src) in sources {
        let file = syn::parse_file(src).map_err(|e| format!("{path}: cannot parse: {e}"))?;
        for item in file.items {
            if let syn::Item::Struct(item) = item
                && let Some(fields) = named_fields(&item.fields)
            {
                index.aggregates.insert(item.ident.to_string(), fields);
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
