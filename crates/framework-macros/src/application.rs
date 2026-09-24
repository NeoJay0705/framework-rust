use std::collections::HashMap;

use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::ToTokens;
use quote::{format_ident, quote};
use syn::{
    GenericArgument, Item, ItemMod, LitInt, LitStr, Path, PathArguments, PathSegment, Type,
    TypePath, UseTree, braced, bracketed, parse::Parse, parse::ParseStream, parse_quote, parse2,
    spanned::Spanned, visit::Visit, visit_mut::VisitMut,
};

use crate::{
    declaration::{Declaration, DeclarationKind, Dependency, ParsedBundle, parse_bundle_module},
    graph::{
        GraphError, GraphPlan, Key, ProviderDescriptor, ProviderId, QualifierKey, Request,
        RequestSlot, RootDescriptor, RootId, TypeKey,
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
    items.push(parse2::<Item>(generated)?);
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
                | "callable" => {
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
                dependencies: dependencies
                    .ok_or_else(|| content.error("missing dependency list"))?,
                is_async,
                wrapper_name,
                wrapper: TokenStream2::new(),
            },
            callable: callable.ok_or_else(|| content.error("missing callable path"))?,
        })
    }
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
}

impl CompositionEntry {
    fn from_import(record: BundleRecord, prefix: Ident, import_path: String) -> Self {
        Self {
            origin: format!(
                "bundle import `{import_path}` (module `{}`)",
                record.source_module
            ),
            declaration: record.declaration,
            callable: record.callable,
            import_prefix: Some(prefix),
        }
    }

    fn from_local(module_name: &Ident, declarations: Vec<Declaration>) -> Vec<Self> {
        let module_name = module_name.to_string();
        declarations
            .into_iter()
            .map(|declaration| {
                let wrapper_name = &declaration.wrapper_name;
                Self {
                    callable: quote! { #wrapper_name },
                    declaration,
                    import_prefix: None,
                    origin: format!("application module `{module_name}`"),
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
    callable: TokenStream2,
    is_async: bool,
    output_span: Span,
    origin: String,
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

        let prepared_entry = PreparedEntry {
            key: key.clone(),
            dependencies,
            callable: entry.callable.clone(),
            is_async: entry.declaration.is_async,
            output_span,
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
    codegen(
        &prepared,
        &provider_entry_indices,
        &root_entry_indices,
        &plan,
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
) -> syn::Result<TokenStream2> {
    let bindings: HashMap<_, _> = plan
        .bindings
        .iter()
        .map(|binding| (binding.slot, binding.provider))
        .collect();
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
        constructions.push(quote! {
            let #value = #callable(#(#args),*);
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
            hooks.push(quote! { #callable(#(#args),*).await; });
        } else {
            hooks.push(quote! { #callable(#(#args),*); });
        }
    }

    Ok(quote! {
        pub async fn run() {
            #(#constructions)*
            #(#hooks)*
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
    if let Some(item) = items.iter().find(|item| match item {
        Item::Fn(function) => function.sig.ident == "run",
        Item::Const(item) => item.ident == "run",
        Item::Static(item) => item.ident == "run",
        Item::Use(item) => use_tree_binds_run(&item.tree),
        _ => false,
    }) {
        return Err(syn::Error::new_spanned(
            item,
            "application module already defines `run`, which conflicts with generated `pub async fn run()`",
        ));
    }
    Ok(())
}

fn use_tree_binds_run(tree: &UseTree) -> bool {
    match tree {
        UseTree::Path(path) => use_tree_binds_run(&path.tree),
        UseTree::Name(name) => name.ident == "run",
        UseTree::Rename(rename) => rename.rename == "run",
        UseTree::Group(group) => group.items.iter().any(use_tree_binds_run),
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
