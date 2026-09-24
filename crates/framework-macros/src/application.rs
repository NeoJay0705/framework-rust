use std::collections::{HashMap, HashSet};

use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::ToTokens;
use quote::{format_ident, quote};
use syn::{
    GenericArgument, Item, ItemMod, LitBool, LitInt, LitStr, Path, PathArguments, PathSegment,
    Type, TypePath, UseTree, braced, bracketed, parse::Parse, parse::ParseStream, parse_quote,
    parse2, spanned::Spanned, visit::Visit, visit_mut::VisitMut,
};

use crate::{
    declaration::{
        Declaration, DeclarationKind, Dependency, ParsedBundle, RegistrationCapability,
        parse_bundle_module,
    },
    graph::{
        GraphError, GraphPlan, Key, ProviderDescriptor, ProviderId, ProviderIndex, QualifierKey,
        Request, RequestSlot, RootDescriptor, RootId, TypeKey,
    },
};

struct ApplicationArgs {
    imports: Vec<Path>,
}

impl Parse for ApplicationArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self {
                imports: Vec::new(),
            });
        }

        let key: Ident = input.parse()?;
        if key != "imports" {
            return Err(syn::Error::new(key.span(), "expected `imports(path, ...)`"));
        }
        let content;
        syn::parenthesized!(content in input);
        let mut imports = Vec::new();
        while !content.is_empty() {
            imports.push(content.parse::<Path>()?);
            if content.peek(syn::Token![,]) {
                content.parse::<syn::Token![,]>()?;
            } else if !content.is_empty() {
                return Err(content.error("expected `,` between imported bundle paths"));
            }
        }
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after `imports(...)`"));
        }
        for path in &imports {
            if path.segments.len() < 2 {
                return Err(syn::Error::new_spanned(
                    path,
                    "an imported bundle path must include a crate and macro name",
                ));
            }
        }
        Ok(Self { imports })
    }
}

pub fn expand(attribute: TokenStream2, item: TokenStream2) -> syn::Result<TokenStream2> {
    let args = parse2::<ApplicationArgs>(attribute)?;
    let module = parse2::<ItemMod>(item)?;
    let initial_accumulator = quote! { framework_application_module { #module } };

    if args.imports.is_empty() {
        return Ok(quote! {
            ::framework::__compose! { [#initial_accumulator] }
        });
    }

    let module_name = &module.ident;
    let callback_names: Vec<_> = (0..args.imports.len())
        .map(|index| format_ident!("__framework_{}_forward_{index}", module_name))
        .collect();
    let mut callback_definitions = Vec::new();

    for (index, current_path) in args.imports.iter().enumerate() {
        let callback = &callback_names[index];
        let crate_prefix = &current_path.segments[0].ident;
        let forward_segment = quote! {
            framework_bundle_import {
                import_path { #current_path }
                crate_prefix { #crate_prefix }
                records { $($records)* }
            }
        };

        if let Some(next_path) = args.imports.get(index + 1) {
            let next_callback = &callback_names[index + 1];
            callback_definitions.push(quote! {
                #[doc(hidden)]
                macro_rules! #callback {
                    ([$($acc:tt)*] [$($records:tt)*]) => {
                        #next_path! {
                            @__framework_forward #next_callback;
                            [$($acc)* #forward_segment]
                        }
                    };
                }
            });
        } else {
            callback_definitions.push(quote! {
                #[doc(hidden)]
                macro_rules! #callback {
                    ([$($acc:tt)*] [$($records:tt)*]) => {
                        ::framework::__compose! {
                            [$($acc)* #forward_segment]
                        }
                    };
                }
            });
        }
    }

    let first_path = &args.imports[0];
    let first_callback = &callback_names[0];
    Ok(quote! {
        #(#callback_definitions)*
        #first_path! {
            @__framework_forward #first_callback;
            [#initial_accumulator]
        }
    })
}

pub fn compose(input: TokenStream2) -> syn::Result<TokenStream2> {
    let input = parse2::<ComposeInput>(input)?;
    let ParsedBundle {
        module,
        declarations,
    } = parse_bundle_module(input.module)?;
    reject_run_collision(&module)?;

    let mut entries = Vec::new();
    for import in input.imports {
        let origin = import.import_path.to_token_stream().to_string();
        for record in import.records {
            entries.push(CompositionEntry::from_import(
                record,
                import.crate_prefix.clone(),
                origin.clone(),
            ));
        }
    }
    entries.extend(CompositionEntry::from_local(&module.ident, declarations));

    let generated = compose_entries(&entries)?;
    let mut module = module;
    let Some((_, items)) = module.content.as_mut() else {
        unreachable!("parse_bundle_module validates inline modules")
    };
    items.extend(parse2::<syn::File>(generated)?.items);
    Ok(quote! { #module })
}

struct ComposeInput {
    module: ItemMod,
    imports: Vec<ImportedBundle>,
}

impl Parse for ComposeInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let entries;
        bracketed!(entries in input);
        let mut module = None;
        let mut imports = Vec::new();
        while !entries.is_empty() {
            let kind: Ident = entries.parse()?;
            match kind.to_string().as_str() {
                "framework_application_module" => {
                    if module.is_some() {
                        return Err(syn::Error::new(kind.span(), "duplicate application module"));
                    }
                    let content;
                    braced!(content in entries);
                    module = Some(content.parse::<ItemMod>()?);
                    if !content.is_empty() {
                        return Err(content.error("unexpected tokens after application module"));
                    }
                }
                "framework_bundle_import" => imports.push(parse_imported_bundle(&entries)?),
                _ => {
                    return Err(syn::Error::new(
                        kind.span(),
                        "unexpected token in framework composition metadata",
                    ));
                }
            }
        }
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after composition metadata"));
        }
        let module = module.ok_or_else(|| input.error("missing application module metadata"))?;
        Ok(Self { module, imports })
    }
}

struct ImportedBundle {
    import_path: Path,
    crate_prefix: Ident,
    records: Vec<BundleRecord>,
}

fn parse_imported_bundle(input: ParseStream<'_>) -> syn::Result<ImportedBundle> {
    let content;
    braced!(content in input);
    let mut import_path = None;
    let mut crate_prefix = None;
    let mut records = None;
    while !content.is_empty() {
        let key: Ident = content.parse()?;
        let group;
        braced!(group in content);
        match key.to_string().as_str() {
            "import_path" => import_path = Some(group.parse::<Path>()?),
            "crate_prefix" => crate_prefix = Some(group.parse::<Ident>()?),
            "records" => {
                let mut parsed = Vec::new();
                while !group.is_empty() {
                    parsed.push(group.parse::<BundleRecord>()?);
                }
                records = Some(parsed);
            }
            _ => {
                return Err(syn::Error::new(
                    key.span(),
                    "unknown imported bundle metadata field",
                ));
            }
        }
        if !group.is_empty() {
            return Err(group.error("unexpected tokens in imported bundle metadata"));
        }
    }
    Ok(ImportedBundle {
        import_path: import_path.ok_or_else(|| content.error("missing bundle import path"))?,
        crate_prefix: crate_prefix.ok_or_else(|| content.error("missing bundle crate prefix"))?,
        records: records.ok_or_else(|| content.error("missing bundle records"))?,
    })
}

struct BundleRecord {
    source_module: Ident,
    declaration: Declaration,
    callable: TokenStream2,
    registrations: Vec<BundleRegistration>,
    lifecycle: Option<LifecycleCallables>,
}

struct BundleRegistration {
    target_type: Type,
    target_qualifier: Option<LitStr>,
    callable: TokenStream2,
}

#[derive(Clone)]
struct LifecycleCallables {
    start: TokenStream2,
    shutdown: TokenStream2,
}

impl Parse for BundleRecord {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let content;
        braced!(content in input);
        let version: Ident = content.parse()?;
        if version != "framework_bundle_metadata_v1" {
            return Err(syn::Error::new(
                version.span(),
                "unsupported framework bundle metadata",
            ));
        }

        let mut source_module = None;
        let mut source_order = None;
        let mut kind = None;
        let mut execution = None;
        let mut output_type = None;
        let mut qualifier = None;
        let mut dependencies = None;
        let mut callable = None;
        let mut registrations = None;
        let mut lifecycle = None;

        while !content.is_empty() {
            let field_content;
            braced!(field_content in content);
            let field: Ident = field_content.parse()?;
            match field.to_string().as_str() {
                "source_order" => {
                    source_order = Some(field_content.parse::<LitInt>()?.base10_parse::<usize>()?);
                }
                "kind" => kind = Some(field_content.parse::<Ident>()?),
                "execution" => execution = Some(field_content.parse::<Ident>()?),
                "bundle_macro" | "source_module" | "output_type" | "qualifier" | "dependencies"
                | "callable" | "registrations" | "lifecycle" => {
                    let value;
                    braced!(value in field_content);
                    match field.to_string().as_str() {
                        "bundle_macro" => {
                            let _: Ident = value.parse()?;
                        }
                        "source_module" => source_module = Some(value.parse::<Ident>()?),
                        "output_type" => output_type = Some(value.parse::<Type>()?),
                        "qualifier" => {
                            qualifier = if value.is_empty() {
                                Some(None)
                            } else {
                                Some(Some(value.parse::<LitStr>()?))
                            }
                        }
                        "dependencies" => {
                            let mut parsed = Vec::new();
                            while !value.is_empty() {
                                parsed.push(parse_record_dependency(&value)?);
                            }
                            dependencies = Some(parsed);
                        }
                        "callable" => callable = Some(value.parse::<TokenStream2>()?),
                        "registrations" => {
                            let mut parsed = Vec::new();
                            while !value.is_empty() {
                                parsed.push(parse_record_registration(&value)?);
                            }
                            registrations = Some(parsed);
                        }
                        "lifecycle" => lifecycle = Some(parse_record_lifecycle(&value)?),
                        _ => unreachable!("the field was matched above"),
                    }
                    if !value.is_empty() {
                        return Err(value.error("unexpected tokens in framework bundle metadata"));
                    }
                }
                _ => {
                    return Err(syn::Error::new(
                        field.span(),
                        "unknown framework bundle metadata field",
                    ));
                }
            }
            if !field_content.is_empty() {
                return Err(
                    field_content.error("unexpected tokens after framework bundle metadata field")
                );
            }
        }

        let source_module = source_module.ok_or_else(|| content.error("missing source module"))?;
        let source_order = source_order.ok_or_else(|| content.error("missing source order"))?;
        let kind = match kind
            .ok_or_else(|| content.error("missing declaration kind"))?
            .to_string()
            .as_str()
        {
            "provider" => DeclarationKind::Provider,
            "hook_start" => DeclarationKind::HookStart,
            _ => {
                return Err(syn::Error::new(
                    source_module.span(),
                    "unsupported bundle declaration kind",
                ));
            }
        };
        if lifecycle.is_some() && matches!(&kind, DeclarationKind::HookStart) {
            return Err(syn::Error::new(
                source_module.span(),
                "lifecycle capabilities are only supported on providers",
            ));
        }
        let is_async = match execution
            .ok_or_else(|| content.error("missing declaration execution kind"))?
            .to_string()
            .as_str()
        {
            "sync" => false,
            "asynchronous" => true,
            _ => {
                return Err(syn::Error::new(
                    source_module.span(),
                    "unsupported bundle execution kind",
                ));
            }
        };
        let wrapper_name = format_ident!("__framework_imported_{}", source_order);
        Ok(Self {
            source_module,
            declaration: Declaration {
                source_order,
                kind,
                output_type: output_type.ok_or_else(|| content.error("missing output type"))?,
                qualifier: qualifier.ok_or_else(|| content.error("missing qualifier field"))?,
                registrations: Vec::new(),
                lifecycle: None,
                dependencies: dependencies
                    .ok_or_else(|| content.error("missing dependency list"))?,
                is_async,
                wrapper_name,
                wrapper: TokenStream2::new(),
            },
            callable: callable.ok_or_else(|| content.error("missing callable path"))?,
            registrations: registrations.unwrap_or_default(),
            lifecycle,
        })
    }
}

fn parse_record_lifecycle(input: ParseStream<'_>) -> syn::Result<LifecycleCallables> {
    let mut enabled = None;
    let mut start = None;
    let mut shutdown = None;
    while !input.is_empty() {
        let field_content;
        braced!(field_content in input);
        let field: Ident = field_content.parse()?;
        let value;
        braced!(value in field_content);
        match field.to_string().as_str() {
            "enabled" => enabled = Some(value.parse::<LitBool>()?.value),
            "start" => start = Some(parse_record_lifecycle_callable(&value)?),
            "shutdown" => shutdown = Some(parse_record_lifecycle_callable(&value)?),
            _ => {
                return Err(syn::Error::new(
                    field.span(),
                    "unknown lifecycle metadata field",
                ));
            }
        }
        if !value.is_empty() || !field_content.is_empty() {
            return Err(field_content.error("unexpected tokens in lifecycle metadata field"));
        }
    }

    if !enabled.ok_or_else(|| input.error("missing lifecycle enabled flag"))? {
        return Err(input.error("disabled lifecycle metadata must be omitted"));
    }
    Ok(LifecycleCallables {
        start: start.ok_or_else(|| input.error("missing lifecycle start callable"))?,
        shutdown: shutdown.ok_or_else(|| input.error("missing lifecycle shutdown callable"))?,
    })
}

fn parse_record_lifecycle_callable(input: ParseStream<'_>) -> syn::Result<TokenStream2> {
    let field: Ident = input.parse()?;
    if field != "callable" {
        return Err(syn::Error::new(
            field.span(),
            "expected lifecycle callable metadata",
        ));
    }
    let path_content;
    braced!(path_content in input);
    let callable = path_content.parse::<TokenStream2>()?;
    if callable.is_empty() {
        return Err(path_content.error("missing lifecycle callable path"));
    }
    if !path_content.is_empty() || !input.is_empty() {
        return Err(input.error("unexpected tokens after lifecycle callable"));
    }
    Ok(callable)
}

fn parse_record_registration(input: ParseStream<'_>) -> syn::Result<BundleRegistration> {
    let registration;
    braced!(registration in input);
    let keyword: Ident = registration.parse()?;
    if keyword != "registration" {
        return Err(syn::Error::new(
            keyword.span(),
            "expected registration metadata",
        ));
    }
    let _: LitInt = registration.parse()?;
    let mut target_type = None;
    let mut target_qualifier = None;
    let mut callable = None;
    while !registration.is_empty() {
        let field;
        braced!(field in registration);
        let name: Ident = field.parse()?;
        let value;
        braced!(value in field);
        match name.to_string().as_str() {
            "target_type" => target_type = Some(value.parse::<Type>()?),
            "target_qualifier" => {
                target_qualifier = Some(if value.is_empty() {
                    None
                } else {
                    Some(value.parse::<LitStr>()?)
                })
            }
            "callable" => callable = Some(value.parse::<TokenStream2>()?),
            _ => {
                return Err(syn::Error::new(
                    name.span(),
                    "unknown registration metadata field",
                ));
            }
        }
        if !value.is_empty() || !field.is_empty() {
            return Err(field.error("unexpected tokens in registration metadata field"));
        }
    }
    Ok(BundleRegistration {
        target_type: target_type
            .ok_or_else(|| registration.error("missing registration target type"))?,
        target_qualifier: target_qualifier
            .ok_or_else(|| registration.error("missing registration target qualifier field"))?,
        callable: callable
            .ok_or_else(|| registration.error("missing registration callable path"))?,
    })
}

fn parse_record_dependency(input: ParseStream<'_>) -> syn::Result<Dependency> {
    let dependency;
    braced!(dependency in input);
    let keyword: Ident = dependency.parse()?;
    if keyword != "dependency" {
        return Err(syn::Error::new(
            keyword.span(),
            "expected dependency metadata",
        ));
    }
    let _: LitInt = dependency.parse()?;
    let type_content;
    braced!(type_content in dependency);
    let type_syntax = type_content.parse::<Type>()?;
    if !type_content.is_empty() {
        return Err(type_content.error("unexpected tokens after dependency type"));
    }
    let qualifier_content;
    braced!(qualifier_content in dependency);
    let qualifier = if qualifier_content.is_empty() {
        None
    } else {
        Some(qualifier_content.parse::<LitStr>()?)
    };
    if !qualifier_content.is_empty() || !dependency.is_empty() {
        return Err(dependency.error("unexpected tokens in dependency metadata"));
    }
    Ok(Dependency {
        type_syntax,
        qualifier,
    })
}

struct CompositionEntry {
    declaration: Declaration,
    callable: TokenStream2,
    import_prefix: Option<Ident>,
    origin: String,
    registrations: Vec<CompositionRegistration>,
    lifecycle: Option<LifecycleCallables>,
}

struct CompositionRegistration {
    target_type: Type,
    target_qualifier: Option<LitStr>,
    callable: TokenStream2,
}

fn local_registration(registration: &RegistrationCapability) -> CompositionRegistration {
    let wrapper_name = &registration.wrapper_name;
    CompositionRegistration {
        target_type: registration.target_type.clone(),
        target_qualifier: registration.target_qualifier.clone(),
        callable: quote! { #wrapper_name },
    }
}

impl CompositionEntry {
    fn from_import(record: BundleRecord, prefix: Ident, import_path: String) -> Self {
        let lifecycle = record.lifecycle;
        let registrations = record
            .registrations
            .into_iter()
            .map(|registration| CompositionRegistration {
                target_type: registration.target_type,
                target_qualifier: registration.target_qualifier,
                callable: registration.callable,
            })
            .collect();
        Self {
            origin: format!(
                "bundle import `{import_path}` (module `{}`)",
                record.source_module
            ),
            declaration: record.declaration,
            callable: record.callable,
            import_prefix: Some(prefix),
            registrations,
            lifecycle,
        }
    }

    fn from_local(module_name: &Ident, declarations: Vec<Declaration>) -> Vec<Self> {
        let module_name = module_name.to_string();
        declarations
            .into_iter()
            .map(|declaration| {
                let wrapper_name = &declaration.wrapper_name;
                let lifecycle = declaration.lifecycle.as_ref().map(|capability| {
                    let start = &capability.start_wrapper_name;
                    let shutdown = &capability.shutdown_wrapper_name;
                    LifecycleCallables {
                        start: quote! { #start },
                        shutdown: quote! { #shutdown },
                    }
                });
                let registrations = declaration
                    .registrations
                    .iter()
                    .map(local_registration)
                    .collect();
                Self {
                    callable: quote! { #wrapper_name },
                    declaration,
                    import_prefix: None,
                    origin: format!("application module `{module_name}`"),
                    registrations,
                    lifecycle,
                }
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
enum ArgumentMode {
    Move,
    Borrow,
    Clone,
}

struct PreparedDependency {
    mode: ArgumentMode,
    span: Span,
}

struct PreparedEntry {
    key: Option<Key>,
    dependencies: Vec<PreparedDependency>,
    registrations: Vec<PreparedRegistration>,
    lifecycle: Option<LifecycleCallables>,
    callable: TokenStream2,
    is_async: bool,
    output_span: Span,
    component_identifier: Option<String>,
    origin: String,
}

struct PreparedRegistration {
    request: Request,
    callable: TokenStream2,
    span: Span,
}

struct BoundRegistration {
    registrar: ProviderId,
    target: ProviderId,
    callable: TokenStream2,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum EntryKind {
    Provider,
    Hook,
}

fn compose_entries(entries: &[CompositionEntry]) -> syn::Result<TokenStream2> {
    let mut prepared = Vec::with_capacity(entries.len());
    let mut provider_entry_indices = Vec::new();
    let mut root_entry_indices = Vec::new();
    let mut providers = Vec::new();
    let mut roots = Vec::new();
    let mut requester_spans = HashMap::new();
    let mut requester_origins = HashMap::new();

    for (entry_index, entry) in entries.iter().enumerate() {
        let prefix_path = entry
            .import_prefix
            .as_ref()
            .map(|prefix| parse_quote!(#prefix));
        let kind = match entry.declaration.kind {
            DeclarationKind::Provider => EntryKind::Provider,
            DeclarationKind::HookStart => EntryKind::Hook,
        };
        let mut output_type = entry.declaration.output_type.clone();
        if let Some(prefix) = &prefix_path {
            rewrite_crate_prefix(&mut output_type, prefix);
        }
        validate_type(&output_type)?;
        let output_span = output_type.span();
        let key = if kind == EntryKind::Provider {
            Some(make_key(
                &output_type,
                entry.declaration.qualifier.as_ref(),
            )?)
        } else {
            None
        };

        let mut dependencies = Vec::with_capacity(entry.declaration.dependencies.len());
        let mut graph_dependencies = Vec::with_capacity(entry.declaration.dependencies.len());
        for dependency in &entry.declaration.dependencies {
            let mut syntax = dependency.type_syntax.clone();
            if let Some(prefix) = &prefix_path {
                rewrite_crate_prefix(&mut syntax, prefix);
            }
            let (type_key, mode) = make_request_type(&mut syntax)?;
            validate_type(&syntax)?;
            let request = Request {
                key: Key {
                    type_key,
                    qualifier: dependency
                        .qualifier
                        .as_ref()
                        .map(|qualifier| QualifierKey(qualifier.value())),
                },
            };
            let span = syntax.span();
            dependencies.push(PreparedDependency { mode, span });
            graph_dependencies.push(request);
        }

        if kind == EntryKind::Hook && !entry.registrations.is_empty() {
            return Err(syn::Error::new(
                entry.registrations[0].target_type.span(),
                "registration capabilities are only supported on providers",
            ));
        }
        let mut registrations = Vec::with_capacity(entry.registrations.len());
        for registration in &entry.registrations {
            let mut target_type = registration.target_type.clone();
            if let Some(prefix) = &prefix_path {
                rewrite_crate_prefix(&mut target_type, prefix);
            }
            validate_type(&target_type)?;
            let span = target_type.span();
            registrations.push(PreparedRegistration {
                request: Request {
                    key: make_key(&target_type, registration.target_qualifier.as_ref())?,
                },
                callable: registration.callable.clone(),
                span,
            });
        }

        let prepared_entry = PreparedEntry {
            key: key.clone(),
            dependencies,
            registrations,
            lifecycle: entry.lifecycle.clone(),
            callable: entry.callable.clone(),
            is_async: entry.declaration.is_async,
            output_span,
            component_identifier: key.as_ref().map(|key| {
                format!(
                    "{} provider declaration {} ({})",
                    entry.origin,
                    entry.declaration.source_order,
                    display_key(key)
                )
            }),
            origin: entry.origin.clone(),
        };
        match kind {
            EntryKind::Provider => {
                let provider_id = ProviderId(providers.len());
                provider_entry_indices.push(entry_index);
                providers.push(ProviderDescriptor {
                    key: key.expect("providers have keys"),
                    dependencies: graph_dependencies,
                });
                for (dependency_index, dependency) in prepared_entry.dependencies.iter().enumerate()
                {
                    requester_spans.insert(
                        RequestSlot::Provider {
                            provider: provider_id,
                            dependency_index,
                        },
                        dependency.span,
                    );
                    requester_origins.insert(
                        RequestSlot::Provider {
                            provider: provider_id,
                            dependency_index,
                        },
                        prepared_entry.origin.clone(),
                    );
                }
                for (registration_index, registration) in
                    prepared_entry.registrations.iter().enumerate()
                {
                    let slot = RequestSlot::Registration {
                        provider: provider_id,
                        registration_index,
                    };
                    requester_spans.insert(slot, registration.span);
                    requester_origins.insert(slot, prepared_entry.origin.clone());
                }
            }
            EntryKind::Hook => {
                let root_id = RootId(roots.len());
                root_entry_indices.push(entry_index);
                roots.push(RootDescriptor {
                    dependencies: graph_dependencies,
                });
                for (dependency_index, dependency) in prepared_entry.dependencies.iter().enumerate()
                {
                    requester_spans.insert(
                        RequestSlot::Root {
                            root: root_id,
                            dependency_index,
                        },
                        dependency.span,
                    );
                    requester_origins.insert(
                        RequestSlot::Root {
                            root: root_id,
                            dependency_index,
                        },
                        prepared_entry.origin.clone(),
                    );
                }
            }
        }
        prepared.push(prepared_entry);
    }

    let plan = crate::graph::resolve(&providers, &roots).map_err(|error| {
        graph_diagnostic(
            error,
            &prepared,
            &provider_entry_indices,
            &requester_spans,
            &requester_origins,
        )
    })?;
    let provider_index = ProviderIndex::new(&providers).map_err(|error| {
        graph_diagnostic(
            error,
            &prepared,
            &provider_entry_indices,
            &requester_spans,
            &requester_origins,
        )
    })?;
    let reachable: HashSet<_> = plan.construction_order.iter().copied().collect();
    let mut registrations = Vec::new();
    for registrar in &plan.construction_order {
        let entry_index = provider_entry_indices[registrar.0];
        let entry = &prepared[entry_index];
        for (registration_index, registration) in entry.registrations.iter().enumerate() {
            let slot = RequestSlot::Registration {
                provider: *registrar,
                registration_index,
            };
            let target = provider_index
                .resolve_request(&registration.request, slot)
                .map_err(|error| {
                    graph_diagnostic(
                        error,
                        &prepared,
                        &provider_entry_indices,
                        &requester_spans,
                        &requester_origins,
                    )
                })?;
            let target_key = &registration.request.key;
            if target == *registrar {
                return Err(syn::Error::new(
                    registration.span,
                    format!(
                        "registration target {} resolves to its registering provider from {}; a provider cannot register itself",
                        display_key(target_key),
                        entry.origin
                    ),
                ));
            }
            if !reachable.contains(&target) {
                return Err(syn::Error::new(
                    registration.span,
                    format!(
                        "registration target {} from {} is not reachable from any hook",
                        display_key(target_key),
                        entry.origin
                    ),
                ));
            }
            registrations.push(BoundRegistration {
                registrar: *registrar,
                target,
                callable: registration.callable.clone(),
            });
        }
    }
    codegen(
        &prepared,
        &provider_entry_indices,
        &root_entry_indices,
        &plan,
        &registrations,
    )
}

fn make_key(ty: &Type, qualifier: Option<&LitStr>) -> syn::Result<Key> {
    Ok(Key {
        type_key: TypeKey(type_key_string(ty)?),
        qualifier: qualifier.map(|value| QualifierKey(value.value())),
    })
}

fn make_request_type(ty: &mut Type) -> syn::Result<(TypeKey, ArgumentMode)> {
    let mode = match ty {
        Type::Reference(reference) => {
            if reference.mutability.is_some() {
                return Err(syn::Error::new_spanned(
                    reference,
                    "mutable reference dependencies are not supported",
                ));
            }
            let element = (*reference.elem).clone();
            *ty = element;
            ArgumentMode::Borrow
        }
        _ if is_arc_or_rc(ty) => ArgumentMode::Clone,
        _ => ArgumentMode::Move,
    };
    Ok((TypeKey(type_key_string(ty)?), mode))
}

fn type_key_string(ty: &Type) -> syn::Result<String> {
    validate_type(ty)?;
    Ok(quote! { #ty }.to_string())
}

fn is_arc_or_rc(ty: &Type) -> bool {
    let Type::Path(TypePath { qself: None, path }) = ty else {
        return false;
    };
    let segments: Vec<_> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    matches!(segments.as_slice(), [std, sync, arc] if std == "std" && sync == "sync" && arc == "Arc")
        || matches!(segments.as_slice(), [std, namespace, rc_name] if std == "std" && namespace == "rc" && rc_name == "Rc")
}

struct CratePrefixRewriter<'a> {
    prefix: &'a Path,
}

impl VisitMut for CratePrefixRewriter<'_> {
    fn visit_type_path_mut(&mut self, type_path: &mut TypePath) {
        if type_path.qself.is_none()
            && type_path
                .path
                .segments
                .first()
                .is_some_and(|segment| segment.ident == "crate")
        {
            let tail: Vec<PathSegment> = type_path.path.segments.iter().skip(1).cloned().collect();
            type_path.path.segments = self.prefix.segments.clone();
            type_path.path.segments.extend(tail);
            type_path.path.leading_colon = None;
        }
        syn::visit_mut::visit_type_path_mut(self, type_path);
    }
}

fn rewrite_crate_prefix(ty: &mut Type, prefix: &Path) {
    CratePrefixRewriter { prefix }.visit_type_mut(ty);
}

fn validate_type(ty: &Type) -> syn::Result<()> {
    let mut validator = TypeValidator { error: None };
    validator.visit_type(ty);
    if let Some(error) = validator.error {
        Err(error)
    } else {
        Ok(())
    }
}

struct TypeValidator {
    error: Option<syn::Error>,
}

impl<'ast> Visit<'ast> for TypeValidator {
    fn visit_type_path(&mut self, ty: &'ast TypePath) {
        if self.error.is_some() {
            return;
        }
        if ty.qself.is_some() {
            self.error = Some(syn::Error::new_spanned(
                ty,
                "qualified or associated type paths are not supported in framework declarations",
            ));
            return;
        }
        if ty.path.segments.iter().any(|segment| {
            segment.ident == "self" || segment.ident == "super" || segment.ident == "Self"
        }) {
            self.error = Some(syn::Error::new_spanned(
                ty,
                "`self`, `super`, and `Self` type paths are not supported; use an explicit crate or external path",
            ));
            return;
        }
        let first = ty
            .path
            .segments
            .first()
            .map(|segment| segment.ident.to_string());
        let single = ty.path.segments.len() == 1;
        if single && !first.as_deref().is_some_and(is_primitive_type) {
            self.error = Some(syn::Error::new_spanned(
                ty,
                "bare custom type names are not supported; use an explicit crate or external path",
            ));
            return;
        }
        for segment in &ty.path.segments {
            if let PathArguments::AngleBracketed(arguments) = &segment.arguments
                && arguments.args.iter().any(|argument| {
                    matches!(
                        argument,
                        GenericArgument::AssocType(_)
                            | GenericArgument::AssocConst(_)
                            | GenericArgument::Constraint(_)
                    )
                })
            {
                self.error = Some(syn::Error::new_spanned(
                    segment,
                    "associated type bindings are not supported in framework declarations",
                ));
                return;
            }
        }
        syn::visit::visit_type_path(self, ty);
    }

    fn visit_type_macro(&mut self, ty: &'ast syn::TypeMacro) {
        if self.error.is_none() {
            self.error = Some(syn::Error::new_spanned(
                ty,
                "type macros are not supported in framework declarations",
            ));
        }
    }
}

fn is_primitive_type(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "str"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
    )
}

fn codegen(
    entries: &[PreparedEntry],
    provider_entry_indices: &[usize],
    root_entry_indices: &[usize],
    plan: &GraphPlan,
    registrations: &[BoundRegistration],
) -> syn::Result<TokenStream2> {
    let bindings: HashMap<_, _> = plan
        .bindings
        .iter()
        .map(|binding| (binding.slot, binding.provider))
        .collect();
    let mutable_targets: HashSet<_> = registrations.iter().map(|entry| entry.target).collect();
    let mut constructions = Vec::new();
    for provider_id in &plan.construction_order {
        let entry_index = provider_entry_indices[provider_id.0];
        let entry = &entries[entry_index];
        let args = entry
            .dependencies
            .iter()
            .enumerate()
            .map(|(dependency_index, dependency)| {
                let slot = RequestSlot::Provider {
                    provider: *provider_id,
                    dependency_index,
                };
                let provider = bindings
                    .get(&slot)
                    .expect("resolved provider dependency has a binding");
                argument_expression(dependency.mode, *provider)
            })
            .collect::<Vec<_>>();
        let callable = &entry.callable;
        let value = provider_value(*provider_id);
        if mutable_targets.contains(provider_id) {
            constructions.push(quote! {
                let mut #value = #callable(#(#args),*);
            });
        } else {
            constructions.push(quote! {
                let #value = #callable(#(#args),*);
            });
        }
    }

    let registration_calls = registrations.iter().map(|registration| {
        let registrar = provider_value(registration.registrar);
        let target = provider_value(registration.target);
        let callable = &registration.callable;
        quote! { #callable(&#registrar, &mut #target); }
    });

    let mut lifecycle_starts = Vec::new();
    let mut lifecycle_shutdowns = Vec::new();
    for provider_id in &plan.construction_order {
        let entry_index = provider_entry_indices[provider_id.0];
        let entry = &entries[entry_index];
        let Some(lifecycle) = &entry.lifecycle else {
            continue;
        };
        let value = provider_value(*provider_id);
        let provider_index = LitInt::new(&provider_id.0.to_string(), Span::call_site());
        let component = LitStr::new(
            entry
                .component_identifier
                .as_deref()
                .expect("lifecycle capability belongs to a provider"),
            Span::call_site(),
        );
        let start = &lifecycle.start;
        let shutdown = &lifecycle.shutdown;
        lifecycle_starts.push(quote! {
            if !__framework_stopped && !__framework_start_failed {
                match #start(&#value).await {
                    ::std::result::Result::Ok(()) => {
                        __framework_started.push(#provider_index);
                        if !__framework_one_shot
                            && __framework_poll_stop_once(__framework_stop.as_mut()).await
                        {
                            __framework_stopped = true;
                        }
                    }
                    ::std::result::Result::Err(error) => {
                        __framework_errors.push(::framework::LifecycleFailure::new(
                            ::framework::LifecyclePhase::Start,
                            #component,
                            error,
                        ));
                        __framework_start_failed = true;
                    }
                }
            }
        });
        lifecycle_shutdowns.push(quote! {
            if __framework_started.last() == ::std::option::Option::Some(&#provider_index) {
                let _ = __framework_started.pop();
                if let ::std::result::Result::Err(error) = #shutdown(&#value).await {
                    __framework_errors.push(::framework::LifecycleFailure::new(
                        ::framework::LifecyclePhase::Shutdown,
                        #component,
                        error,
                    ));
                }
            }
        });
    }

    let mut hooks = Vec::new();
    for (root_index, entry_index) in root_entry_indices.iter().enumerate() {
        let entry = &entries[*entry_index];
        let args = entry
            .dependencies
            .iter()
            .enumerate()
            .map(|(dependency_index, dependency)| {
                let slot = RequestSlot::Root {
                    root: RootId(root_index),
                    dependency_index,
                };
                let provider = bindings
                    .get(&slot)
                    .expect("resolved hook dependency has a binding");
                argument_expression(dependency.mode, *provider)
            })
            .collect::<Vec<_>>();
        let callable = &entry.callable;
        if entry.is_async {
            hooks.push(quote! {
                if !__framework_one_shot && !__framework_stopped && !__framework_start_failed
                    && __framework_poll_stop_once(__framework_stop.as_mut()).await
                {
                    __framework_stopped = true;
                }
                if !__framework_stopped && !__framework_start_failed {
                    #callable(#(#args),*).await;
                }
            });
        } else {
            hooks.push(quote! {
                if !__framework_one_shot && !__framework_stopped && !__framework_start_failed
                    && __framework_poll_stop_once(__framework_stop.as_mut()).await
                {
                    __framework_stopped = true;
                }
                if !__framework_stopped && !__framework_start_failed {
                    #callable(#(#args),*);
                }
            });
        }
    }

    let lifecycle_shutdowns = lifecycle_shutdowns.into_iter().rev().collect::<Vec<_>>();

    Ok(quote! {
        #[doc(hidden)]
        async fn __framework_run_internal<__FrameworkStop>(
            __framework_stop_future: __FrameworkStop,
            __framework_one_shot: bool,
        ) -> ::std::result::Result<(), ::framework::RunError>
        where
            __FrameworkStop: ::std::future::Future<Output = ()>,
        {
            async fn __framework_poll_stop_once<__FrameworkStop>(
                mut __framework_stop: ::std::pin::Pin<&mut __FrameworkStop>,
            ) -> bool
            where
                __FrameworkStop: ::std::future::Future<Output = ()>,
            {
                ::std::future::poll_fn(|__framework_context| {
                    ::std::task::Poll::Ready(
                        ::std::future::Future::poll(
                            __framework_stop.as_mut(),
                            __framework_context,
                        )
                        .is_ready(),
                    )
                })
                .await
            }

            #(#constructions)*
            #(#registration_calls)*

            let mut __framework_stop = ::std::pin::pin!(__framework_stop_future);
            let mut __framework_errors = ::framework::RunError::new();
            let mut __framework_started = ::std::vec::Vec::<usize>::new();
            let mut __framework_start_failed = false;
            let mut __framework_stopped = if __framework_one_shot {
                false
            } else {
                __framework_poll_stop_once(__framework_stop.as_mut()).await
            };

            #(#lifecycle_starts)*
            #(#hooks)*

            if !__framework_one_shot
                && !__framework_stopped
                && !__framework_start_failed
            {
                __framework_stop.as_mut().await;
            }

            #(#lifecycle_shutdowns)*

            if __framework_errors.failures().is_empty() {
                ::std::result::Result::Ok(())
            } else {
                ::std::result::Result::Err(__framework_errors)
            }
        }

        #[doc = "Runs a one-shot application: starts reachable lifecycle components, runs hooks, then awaits shutdown in reverse start order."]
        pub async fn run() -> ::std::result::Result<(), ::framework::RunError> {
            __framework_run_internal(::std::future::pending::<()>(), true).await
        }

        #[doc = "Runs until the supplied stop future completes. The future is polled during startup and hooks, then awaited after hooks if still pending. Successful lifecycle components are shut down and awaited in reverse start order."]
        pub async fn run_until<__FrameworkStop>(
            __framework_stop: __FrameworkStop,
        ) -> ::std::result::Result<(), ::framework::RunError>
        where
            __FrameworkStop: ::std::future::Future<Output = ()>,
        {
            __framework_run_internal(__framework_stop, false).await
        }
    })
}

fn argument_expression(mode: ArgumentMode, provider: ProviderId) -> TokenStream2 {
    let value = provider_value(provider);
    match mode {
        ArgumentMode::Move => quote! { #value },
        ArgumentMode::Borrow => quote! { &#value },
        ArgumentMode::Clone => quote! { #value.clone() },
    }
}

fn provider_value(provider: ProviderId) -> Ident {
    format_ident!("__framework_value_{}", provider.0)
}

fn reject_run_collision(module: &ItemMod) -> syn::Result<()> {
    let Some((_, items)) = &module.content else {
        return Ok(());
    };
    for (name, generated_signature) in [
        ("run", "pub async fn run()"),
        (
            "run_until",
            "pub async fn run_until(stop: impl Future<Output = ()>)",
        ),
        (
            "__framework_run_internal",
            "the generated framework runner helper",
        ),
    ] {
        if let Some(item) = items.iter().find(|item| item_binds_value_name(item, name)) {
            return Err(syn::Error::new_spanned(
                item,
                format!(
                    "application module already defines `{name}`, which conflicts with generated `{generated_signature}`"
                ),
            ));
        }
    }
    Ok(())
}

fn item_binds_value_name(item: &Item, name: &str) -> bool {
    match item {
        Item::Fn(function) => function.sig.ident == name,
        Item::Const(item) => item.ident == name,
        Item::Static(item) => item.ident == name,
        Item::Use(item) => use_tree_binds_name(&item.tree, name),
        _ => false,
    }
}

fn use_tree_binds_name(tree: &UseTree, name: &str) -> bool {
    match tree {
        UseTree::Path(path) => use_tree_binds_name(&path.tree, name),
        UseTree::Name(import) => import.ident == name,
        UseTree::Rename(rename) => rename.rename == name,
        UseTree::Group(group) => group
            .items
            .iter()
            .any(|item| use_tree_binds_name(item, name)),
        UseTree::Glob(_) => false,
    }
}

fn graph_diagnostic(
    error: GraphError,
    entries: &[PreparedEntry],
    provider_entry_indices: &[usize],
    requester_spans: &HashMap<RequestSlot, Span>,
    requester_origins: &HashMap<RequestSlot, String>,
) -> syn::Error {
    let provider_label = |provider: ProviderId| -> String {
        let entry = &entries[provider_entry_indices[provider.0]];
        let key = entry.key.as_ref().expect("provider entry has key");
        format!("{} from {}", display_key(key), entry.origin)
    };
    match error {
        GraphError::DuplicateProvider { key, provider_ids } => {
            let labels = provider_ids
                .iter()
                .map(|id| provider_label(*id))
                .collect::<Vec<_>>()
                .join("; ");
            let span = provider_ids
                .first()
                .map(|id| entries[provider_entry_indices[id.0]].output_span)
                .unwrap_or_else(Span::call_site);
            syn::Error::new(
                span,
                format!(
                    "duplicate provider for {}; declarations: {labels}",
                    display_key(&key)
                ),
            )
        }
        GraphError::Missing { request, requester } => {
            let span = requester_spans
                .get(&requester)
                .copied()
                .unwrap_or_else(Span::call_site);
            let origin = requester_origins
                .get(&requester)
                .map(|origin| format!(" requested by {origin}"))
                .unwrap_or_default();
            syn::Error::new(
                span,
                format!("missing provider for {}{origin}", display_key(&request.key)),
            )
        }
        GraphError::Ambiguous {
            request,
            candidate_ids,
            requester,
        } => {
            let span = requester_spans
                .get(&requester)
                .copied()
                .unwrap_or_else(Span::call_site);
            let candidates = candidate_ids
                .iter()
                .map(|id| provider_label(*id))
                .collect::<Vec<_>>()
                .join("; ");
            let origin = requester_origins
                .get(&requester)
                .map(|origin| format!(" requested by {origin}"))
                .unwrap_or_default();
            syn::Error::new(
                span,
                format!(
                    "ambiguous provider for {}{origin}; candidates: {candidates}",
                    display_key(&request.key)
                ),
            )
        }
        GraphError::InvalidQualifier {
            request,
            available_candidates,
            requester,
        } => {
            let span = requester_spans
                .get(&requester)
                .copied()
                .unwrap_or_else(Span::call_site);
            let candidates = available_candidates
                .iter()
                .map(|id| provider_label(*id))
                .collect::<Vec<_>>()
                .join("; ");
            let origin = requester_origins
                .get(&requester)
                .map(|origin| format!(" requested by {origin}"))
                .unwrap_or_default();
            syn::Error::new(
                span,
                format!(
                    "invalid qualifier for {}{origin}; available providers: {candidates}",
                    display_key(&request.key)
                ),
            )
        }
        GraphError::Cycle { closed_path } => {
            let span = closed_path
                .first()
                .map(|id| entries[provider_entry_indices[id.0]].output_span)
                .unwrap_or_else(Span::call_site);
            let path = closed_path
                .iter()
                .map(|id| provider_label(*id))
                .collect::<Vec<_>>()
                .join(" -> ");
            syn::Error::new(span, format!("provider dependency cycle: {path}"))
        }
    }
}

fn display_key(key: &Key) -> String {
    match &key.qualifier {
        Some(qualifier) => format!("type `{}` with qualifier `{}`", key.type_key.0, qualifier.0),
        None => format!("type `{}`", key.type_key.0),
    }
}

#[cfg(test)]
mod tests {
    use super::reject_run_collision;
    use syn::ItemMod;

    #[test]
    fn rejects_imports_that_bind_the_generated_run_name() {
        for source in [
            "mod app { use crate::tools::run; }",
            "mod app { use crate::tools::entry as run; }",
            "mod app { use crate::tools::{entry as other, run}; }",
            "mod app { use crate::tools::{nested::{entry as run}}; }",
        ] {
            let module = syn::parse_str::<ItemMod>(source).expect("test module should parse");
            let error = reject_run_collision(&module).expect_err("run import should conflict");
            assert!(error.to_string().contains("already defines `run`"));
        }
    }

    #[test]
    fn rejects_run_until_and_internal_runner_collisions() {
        for (source, name) in [
            ("mod app { async fn run_until() {} }", "run_until"),
            (
                "mod app { use crate::tools::stop as run_until; }",
                "run_until",
            ),
            (
                "mod app { async fn __framework_run_internal() {} }",
                "__framework_run_internal",
            ),
        ] {
            let module = syn::parse_str::<ItemMod>(source).expect("test module should parse");
            let error = reject_run_collision(&module).expect_err("generated name should conflict");
            assert!(error.to_string().contains(name));
        }
    }

    #[test]
    fn allows_imports_renamed_away_from_generated_run_name() {
        for source in [
            "mod app { use crate::tools::run as start; }",
            "mod app { use crate::tools::{entry as other, nested::*}; }",
        ] {
            let module = syn::parse_str::<ItemMod>(source).expect("test module should parse");
            reject_run_collision(&module).expect("non-conflicting import should pass");
        }
    }
}
