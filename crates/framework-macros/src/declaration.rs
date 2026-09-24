use std::collections::HashSet;

use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{
    Attribute, Fields, FnArg, GenericArgument, Item, ItemFn, ItemMod, ItemStruct, LitInt, LitStr,
    Meta, PathArguments, ReturnType, Signature, Type, TypePath, Visibility, parse::Parse,
    parse::ParseStream, parse_quote, spanned::Spanned, visit::Visit,
};

pub struct BundleArgs {
    pub name: Ident,
}

impl Parse for BundleArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let keyword: Ident = input.parse()?;
        if keyword != "name" {
            return Err(syn::Error::new(
                keyword.span(),
                "expected `name = macro_name`",
            ));
        }
        input.parse::<syn::Token![=]>()?;
        let name: Ident = input.parse()?;
        if input.peek(syn::Token![,]) {
            input.parse::<syn::Token![,]>()?;
        }
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after bundle macro name"));
        }
        Ok(Self { name })
    }
}

pub struct ParsedBundle {
    pub module: ItemMod,
    pub declarations: Vec<Declaration>,
}

pub struct Declaration {
    pub source_order: usize,
    pub kind: DeclarationKind,
    pub output_type: Type,
    pub qualifier: Option<LitStr>,
    pub registrations: Vec<RegistrationCapability>,
    pub dependencies: Vec<Dependency>,
    pub is_async: bool,
    pub wrapper_name: Ident,
    pub wrapper: TokenStream2,
}

pub struct RegistrationCapability {
    pub target_type: Type,
    pub target_qualifier: Option<LitStr>,
    pub wrapper_name: Ident,
}

pub enum DeclarationKind {
    Provider,
    HookStart,
}

pub struct Dependency {
    pub type_syntax: Type,
    pub qualifier: Option<LitStr>,
}

pub fn parse_bundle_module(mut module: ItemMod) -> syn::Result<ParsedBundle> {
    let Some((_, items)) = module.content.as_mut() else {
        return Err(syn::Error::new_spanned(
            module,
            "framework bundle and application declarations require an inline module",
        ));
    };

    let mut declarations = Vec::new();
    let mut wrappers = Vec::new();
    let mut errors = None;

    let module_name = module.ident.clone();
    for item in items.iter_mut() {
        let source_order = declarations.len();
        match item {
            Item::Fn(function) => {
                let provider_attrs = take_attrs(&mut function.attrs, "provide");
                let hook_attrs = take_attrs(&mut function.attrs, "hook");

                if !provider_attrs.is_empty() && !hook_attrs.is_empty() {
                    combine_error(
                        &mut errors,
                        syn::Error::new_spanned(
                            &function.sig.ident,
                            "a function cannot be both a provider and a hook",
                        ),
                    );
                    continue;
                }

                if let Some(attr) = provider_attrs.first() {
                    if provider_attrs.len() > 1 {
                        combine_error(
                            &mut errors,
                            syn::Error::new_spanned(attr, "duplicate `#[provide]` attribute"),
                        );
                        continue;
                    }
                    match parse_provide_args(attr).and_then(|args| {
                        parse_function_dependencies(&mut function.sig).and_then(|dependencies| {
                            provider_function(
                                function,
                                args.qualifier,
                                args.registrations,
                                dependencies,
                                source_order,
                            )
                        })
                    }) {
                        Ok(declaration) => {
                            push_declaration(&mut declarations, &mut wrappers, declaration);
                        }
                        Err(error) => combine_error(&mut errors, error),
                    }
                } else if let Some(attr) = hook_attrs.first() {
                    if hook_attrs.len() > 1 {
                        combine_error(
                            &mut errors,
                            syn::Error::new_spanned(attr, "duplicate `#[hook]` attribute"),
                        );
                        continue;
                    }
                    match parse_hook_kind(attr).and_then(|()| {
                        parse_function_dependencies(&mut function.sig).and_then(|dependencies| {
                            hook_function(function, dependencies, source_order)
                        })
                    }) {
                        Ok(declaration) => {
                            push_declaration(&mut declarations, &mut wrappers, declaration);
                        }
                        Err(error) => combine_error(&mut errors, error),
                    }
                } else if let Some(attr) = function
                    .sig
                    .inputs
                    .iter()
                    .find_map(|arg| find_inject_attr(arg))
                {
                    combine_error(
                        &mut errors,
                        syn::Error::new_spanned(
                            attr,
                            "`#[inject]` is only supported on provider or hook parameters",
                        ),
                    );
                }
            }
            Item::Struct(structure) => {
                let provider_attrs = take_attrs(&mut structure.attrs, "provide");
                if let Some(attr) = provider_attrs.first() {
                    if provider_attrs.len() > 1 {
                        combine_error(
                            &mut errors,
                            syn::Error::new_spanned(attr, "duplicate `#[provide]` attribute"),
                        );
                        continue;
                    }
                    match parse_provide_args(attr).and_then(|args| {
                        provider_struct(
                            structure,
                            args.qualifier,
                            args.registrations,
                            &module_name,
                            source_order,
                        )
                    }) {
                        Ok(declaration) => {
                            push_declaration(&mut declarations, &mut wrappers, declaration);
                        }
                        Err(error) => combine_error(&mut errors, error),
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(error) = errors {
        return Err(error);
    }

    let mut marker_scan = RemainingMarkerFinder { error: None };
    marker_scan.visit_item_mod(&module);
    if let Some(error) = marker_scan.error {
        return Err(error);
    }

    let Some((_, items)) = module.content.as_mut() else {
        unreachable!("inline module was checked above")
    };
    for wrapper in wrappers {
        items.push(syn::parse2::<Item>(wrapper)?);
    }

    Ok(ParsedBundle {
        module,
        declarations,
    })
}

fn provider_function(
    function: &ItemFn,
    qualifier: Option<LitStr>,
    registration_args: Vec<RegistrationArgs>,
    dependencies: Vec<Dependency>,
    source_order: usize,
) -> syn::Result<Declaration> {
    reject_unsafe_or_extern(&function.sig, "provider")?;
    if function.sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            function.sig.asyncness,
            "provider functions must be synchronous",
        ));
    }
    if !function.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &function.sig.generics,
            "generic provider functions are not supported",
        ));
    }
    if function.sig.variadic.is_some() {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "variadic provider functions are not supported",
        ));
    }
    let ReturnType::Type(_, output_type) = &function.sig.output else {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "provider functions must declare an explicit return type",
        ));
    };
    if is_reference_type(output_type) {
        return Err(syn::Error::new_spanned(
            output_type,
            "provider functions cannot return references",
        ));
    }

    let wrapper_name = wrapper_name("provider", source_order);
    let wrapper = make_function_wrapper(function, &wrapper_name, false)?;
    let output_type = (**output_type).clone();
    let registrations = make_registration_capabilities(registration_args, source_order);
    Ok(Declaration {
        source_order,
        kind: DeclarationKind::Provider,
        output_type,
        qualifier,
        registrations,
        dependencies,
        is_async: false,
        wrapper_name,
        wrapper,
    })
}

fn hook_function(
    function: &ItemFn,
    dependencies: Vec<Dependency>,
    source_order: usize,
) -> syn::Result<Declaration> {
    reject_unsafe_or_extern(&function.sig, "hook")?;
    if !function.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &function.sig.generics,
            "generic hooks are not supported",
        ));
    }
    if function.sig.variadic.is_some() {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "variadic hooks are not supported",
        ));
    }
    if !returns_unit(&function.sig.output) {
        return Err(syn::Error::new_spanned(
            &function.sig.output,
            "start hooks must return unit",
        ));
    }

    let wrapper_name = wrapper_name("hook", source_order);
    let wrapper = make_function_wrapper(function, &wrapper_name, true)?;
    Ok(Declaration {
        source_order,
        kind: DeclarationKind::HookStart,
        output_type: parse_quote!(()),
        qualifier: None,
        registrations: Vec::new(),
        dependencies,
        is_async: function.sig.asyncness.is_some(),
        wrapper_name,
        wrapper,
    })
}

fn provider_struct(
    structure: &mut ItemStruct,
    qualifier: Option<LitStr>,
    registration_args: Vec<RegistrationArgs>,
    module: &Ident,
    source_order: usize,
) -> syn::Result<Declaration> {
    if !structure.generics.params.is_empty() || structure.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &structure.generics,
            "generic provider structs are not supported",
        ));
    }

    let wrapper_name = wrapper_name("provider", source_order);
    let mut dependencies = Vec::new();
    let mut params = Vec::new();
    let mut initializers = Vec::new();

    match &mut structure.fields {
        Fields::Named(fields) => {
            for (index, field) in fields.named.iter_mut().enumerate() {
                let field_name = field.ident.clone().expect("named field has an identifier");
                let field_qualifier = take_one_inject(&mut field.attrs)?;
                dependencies.push(Dependency {
                    type_syntax: field.ty.clone(),
                    qualifier: field_qualifier,
                });
                let param_name = format_ident!("__framework_arg_{index}", span = field.span());
                let field_ty = &field.ty;
                params.push(quote! { #param_name: #field_ty });
                initializers.push(quote! { #field_name: #param_name });
            }
            if fields.named.is_empty() {
                // An empty named-field struct has the same constructor shape as a unit provider.
            }
        }
        Fields::Unit => {}
        Fields::Unnamed(fields) => {
            return Err(syn::Error::new_spanned(
                fields,
                "tuple provider structs are not supported",
            ));
        }
    }

    let ident = &structure.ident;
    let output_type: Type = parse_quote!(crate::#module::#ident);
    let registrations = make_registration_capabilities(registration_args, source_order);
    let wrapper = match &structure.fields {
        Fields::Named(_) => quote! {
            #[doc(hidden)]
            pub fn #wrapper_name(#(#params),*) -> #output_type {
                #ident { #(#initializers),* }
            }
        },
        Fields::Unit => quote! {
            #[doc(hidden)]
            pub fn #wrapper_name() -> #output_type {
                #ident
            }
        },
        Fields::Unnamed(_) => unreachable!("tuple struct was rejected above"),
    };

    Ok(Declaration {
        source_order,
        kind: DeclarationKind::Provider,
        output_type,
        qualifier,
        registrations,
        dependencies,
        is_async: false,
        wrapper_name,
        wrapper,
    })
}

fn make_function_wrapper(
    function: &ItemFn,
    wrapper_name: &Ident,
    hook: bool,
) -> syn::Result<TokenStream2> {
    let mut signature = function.sig.clone();
    signature.ident = wrapper_name.clone();
    signature.generics.where_clause = function.sig.generics.where_clause.clone();
    let mut arg_names = Vec::new();

    for (index, input) in signature.inputs.iter_mut().enumerate() {
        match input {
            FnArg::Receiver(receiver) => {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "provider and hook functions cannot have a receiver",
                ));
            }
            FnArg::Typed(argument) => {
                let arg_name = format_ident!("__framework_arg_{index}", span = argument.pat.span());
                *argument.pat = parse_quote!(#arg_name);
                arg_names.push(arg_name);
            }
        }
    }

    let visibility: Visibility = parse_quote!(pub);
    let original_name = &function.sig.ident;
    let asyncness = function.sig.asyncness.is_some();
    let invocation = quote! { #original_name(#(#arg_names),*) };
    let body = match (hook, asyncness) {
        (true, true) => quote! {{ #invocation.await; }},
        (true, false) => quote! {{ #invocation; }},
        (false, _) => quote! {{ #invocation }},
    };

    Ok(quote! {
        #[doc(hidden)]
        #visibility #signature #body
    })
}

fn reject_unsafe_or_extern(signature: &Signature, declaration_kind: &str) -> syn::Result<()> {
    if let Some(unsafety) = &signature.unsafety {
        return Err(syn::Error::new_spanned(
            unsafety,
            format!("unsafe {declaration_kind} functions are not supported"),
        ));
    }
    if let Some(abi) = &signature.abi {
        return Err(syn::Error::new_spanned(
            abi,
            format!("extern ABI {declaration_kind} functions are not supported"),
        ));
    }
    Ok(())
}

fn parse_function_dependencies(signature: &mut Signature) -> syn::Result<Vec<Dependency>> {
    let mut dependencies = Vec::new();
    for input in &mut signature.inputs {
        match input {
            FnArg::Receiver(receiver) => {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "provider and hook functions cannot have a receiver",
                ));
            }
            FnArg::Typed(argument) => dependencies.push(Dependency {
                type_syntax: (*argument.ty).clone(),
                qualifier: take_one_inject(&mut argument.attrs)?,
            }),
        }
    }
    Ok(dependencies)
}

fn parse_provide_args(attribute: &Attribute) -> syn::Result<ProvideArgs> {
    let Meta::List(list) = &attribute.meta else {
        if matches!(attribute.meta, Meta::Path(_)) {
            return Ok(ProvideArgs::default());
        }
        return Err(syn::Error::new_spanned(
            attribute,
            "expected `#[provide(...)]`",
        ));
    };

    let parsed = syn::parse2::<ProvideArgs>(list.tokens.clone())?;
    if let Some(qualifier) = &parsed.qualifier {
        validate_nonempty_qualifier(qualifier)?;
    }
    let mut registration_keys = HashSet::new();
    for registration in &parsed.registrations {
        if let Some(qualifier) = &registration.target_qualifier {
            validate_nonempty_qualifier(qualifier)?;
        }
        let target_type = &registration.target_type;
        let type_key = quote! { #target_type }.to_string();
        let qualifier_key = registration.target_qualifier.as_ref().map(LitStr::value);
        if !registration_keys.insert((type_key, qualifier_key)) {
            return Err(syn::Error::new_spanned(
                &registration.target_type,
                "duplicate registration capability for the same target type and qualifier",
            ));
        }
    }
    Ok(parsed)
}

#[derive(Default)]
struct ProvideArgs {
    qualifier: Option<LitStr>,
    registrations: Vec<RegistrationArgs>,
}

impl Parse for ProvideArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut args = Self::default();
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            match key.to_string().as_str() {
                "qualifier" => {
                    if args.qualifier.is_some() {
                        return Err(syn::Error::new_spanned(key, "duplicate provider qualifier"));
                    }
                    input.parse::<syn::Token![=]>()?;
                    args.qualifier = Some(input.parse()?);
                }
                "register" => {
                    let content;
                    syn::parenthesized!(content in input);
                    args.registrations.push(content.parse()?);
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        "expected `qualifier = \"name\"` or `register(Type, qualifier = \"name\")`",
                    ));
                }
            }

            if input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error("expected `,` between provider attributes"));
            }
        }
        Ok(args)
    }
}

struct RegistrationArgs {
    target_type: Type,
    target_qualifier: Option<LitStr>,
}

impl Parse for RegistrationArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let target_type: Type = input.parse()?;
        let target_qualifier = if input.peek(syn::Token![,]) {
            input.parse::<syn::Token![,]>()?;
            let key: Ident = input.parse()?;
            if key != "qualifier" {
                return Err(syn::Error::new(
                    key.span(),
                    "expected `qualifier = \"name\"` after registration target type",
                ));
            }
            input.parse::<syn::Token![=]>()?;
            Some(input.parse()?)
        } else {
            None
        };

        if input.peek(syn::Token![,]) {
            input.parse::<syn::Token![,]>()?;
        }
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after registration target"));
        }

        Ok(Self {
            target_type,
            target_qualifier,
        })
    }
}

fn parse_hook_kind(attribute: &Attribute) -> syn::Result<()> {
    let Meta::List(list) = &attribute.meta else {
        return Err(syn::Error::new_spanned(
            attribute,
            "expected `#[hook(start)]`",
        ));
    };
    let kind = syn::parse2::<Ident>(list.tokens.clone())?;
    if kind != "start" {
        return Err(syn::Error::new(
            kind.span(),
            "only `#[hook(start)]` is supported",
        ));
    }
    Ok(())
}

fn take_one_inject(attributes: &mut Vec<Attribute>) -> syn::Result<Option<LitStr>> {
    let inject_attrs = take_attrs(attributes, "inject");
    if inject_attrs.len() > 1 {
        return Err(syn::Error::new_spanned(
            &inject_attrs[1],
            "duplicate `#[inject]` attribute",
        ));
    }
    let Some(attribute) = inject_attrs.first() else {
        return Ok(None);
    };
    let Meta::List(list) = &attribute.meta else {
        return Err(syn::Error::new_spanned(
            attribute,
            "expected `#[inject(\"qualifier\")]`",
        ));
    };
    let qualifier = syn::parse2::<LitStr>(list.tokens.clone())?;
    validate_nonempty_qualifier(&qualifier)?;
    Ok(Some(qualifier))
}

fn validate_nonempty_qualifier(qualifier: &LitStr) -> syn::Result<()> {
    if qualifier.value().is_empty() {
        return Err(syn::Error::new_spanned(
            qualifier,
            "qualifier must not be empty",
        ));
    }
    Ok(())
}

fn take_attrs(attributes: &mut Vec<Attribute>, name: &str) -> Vec<Attribute> {
    let mut selected = Vec::new();
    attributes.retain(|attribute| {
        if attribute.path().is_ident(name) {
            selected.push(attribute.clone());
            false
        } else {
            true
        }
    });
    selected
}

fn find_inject_attr(argument: &FnArg) -> Option<&Attribute> {
    match argument {
        FnArg::Receiver(receiver) => receiver
            .attrs
            .iter()
            .find(|attribute| attribute.path().is_ident("inject")),
        FnArg::Typed(argument) => argument
            .attrs
            .iter()
            .find(|attribute| attribute.path().is_ident("inject")),
    }
}

fn wrapper_name(kind: &str, source_order: usize) -> Ident {
    format_ident!("__framework_generated_{kind}_{source_order}")
}

fn is_reference_type(ty: &Type) -> bool {
    match ty {
        Type::Reference(_) => true,
        Type::Paren(paren) => is_reference_type(&paren.elem),
        Type::Group(group) => is_reference_type(&group.elem),
        _ => false,
    }
}

fn returns_unit(output: &ReturnType) -> bool {
    match output {
        ReturnType::Default => true,
        ReturnType::Type(_, ty) => is_unit_type(ty),
    }
}

fn is_unit_type(ty: &Type) -> bool {
    match ty {
        Type::Tuple(tuple) => tuple.elems.is_empty(),
        Type::Paren(paren) => is_unit_type(&paren.elem),
        Type::Group(group) => is_unit_type(&group.elem),
        _ => false,
    }
}

fn push_declaration(
    declarations: &mut Vec<Declaration>,
    wrappers: &mut Vec<TokenStream2>,
    declaration: Declaration,
) {
    wrappers.push(declaration.wrapper.clone());
    wrappers.extend(
        declaration
            .registrations
            .iter()
            .map(|registration| make_registration_wrapper(&declaration.output_type, registration)),
    );
    declarations.push(declaration);
}

fn make_registration_capabilities(
    registrations: Vec<RegistrationArgs>,
    source_order: usize,
) -> Vec<RegistrationCapability> {
    registrations
        .into_iter()
        .enumerate()
        .map(
            |(registration_order, registration)| RegistrationCapability {
                target_type: registration.target_type,
                target_qualifier: registration.target_qualifier,
                wrapper_name: format_ident!(
                    "__framework_generated_registration_{source_order}_{registration_order}"
                ),
            },
        )
        .collect()
}

fn make_registration_wrapper(
    output_type: &Type,
    registration: &RegistrationCapability,
) -> TokenStream2 {
    let wrapper_name = &registration.wrapper_name;
    let target_type = &registration.target_type;
    let component = if is_std_arc_or_rc(output_type) {
        quote! { &**value }
    } else {
        quote! { value }
    };

    quote! {
        #[doc(hidden)]
        pub fn #wrapper_name(value: &#output_type, target: &mut #target_type) {
            ::framework::Register::<#target_type>::register(#component, target);
        }
    }
}

fn is_std_arc_or_rc(ty: &Type) -> bool {
    let Type::Path(TypePath { qself: None, path }) = ty else {
        return false;
    };
    let segments: Vec<_> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let is_shared_pointer = matches!(segments.as_slice(), [std, sync, arc] if std == "std" && sync == "sync" && arc == "Arc")
        || matches!(segments.as_slice(), [std, rc, rc_type] if std == "std" && rc == "rc" && rc_type == "Rc");
    if !is_shared_pointer {
        return false;
    }

    matches!(
        path.segments.last().map(|segment| &segment.arguments),
        Some(PathArguments::AngleBracketed(arguments))
            if arguments.args.len() == 1
                && matches!(arguments.args.first(), Some(GenericArgument::Type(_)))
    )
}

fn combine_error(errors: &mut Option<syn::Error>, error: syn::Error) {
    if let Some(errors) = errors {
        errors.combine(error);
    } else {
        *errors = Some(error);
    }
}

struct RemainingMarkerFinder {
    error: Option<syn::Error>,
}

impl<'ast> Visit<'ast> for RemainingMarkerFinder {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        let path = attribute.path();
        if path.is_ident("provide") || path.is_ident("hook") || path.is_ident("inject") {
            combine_error(
                &mut self.error,
                syn::Error::new_spanned(
                    attribute,
                    "framework marker attributes are only supported on direct bundle declarations, provider/hook parameters, and provided struct fields",
                ),
            );
        }
        syn::visit::visit_attribute(self, attribute);
    }
}

pub fn bundle_metadata_records(
    declarations: &[Declaration],
    module: &Ident,
    bundle_name: &Ident,
) -> Vec<TokenStream2> {
    declarations
        .iter()
        .map(|declaration| {
            let source_order =
                LitInt::new(&declaration.source_order.to_string(), Span::call_site());
            let wrapper_name = &declaration.wrapper_name;
            let output_type = &declaration.output_type;
            let qualifier = qualifier_group(declaration.qualifier.as_ref());
            let dependencies =
                declaration
                    .dependencies
                    .iter()
                    .enumerate()
                    .map(|(slot, dependency)| {
                        let slot = LitInt::new(&slot.to_string(), Span::call_site());
                        let ty = &dependency.type_syntax;
                        let qualifier = qualifier_group(dependency.qualifier.as_ref());
                        quote! {
                            { dependency #slot { #ty } { #qualifier } }
                        }
                    });
            let kind = match declaration.kind {
                DeclarationKind::Provider => quote!(provider),
                DeclarationKind::HookStart => quote!(hook_start),
            };
            let execution = if declaration.is_async {
                quote!(asynchronous)
            } else {
                quote!(sync)
            };
            let registrations = declaration
                .registrations
                .iter()
                .enumerate()
                .map(|(registration_order, registration)| {
                    let registration_order =
                        LitInt::new(&registration_order.to_string(), Span::call_site());
                    let target_type = &registration.target_type;
                    let target_qualifier = qualifier_group(registration.target_qualifier.as_ref());
                    let wrapper_name = &registration.wrapper_name;
                    quote! {
                        {
                            registration #registration_order
                            { target_type { #target_type } }
                            { target_qualifier { #target_qualifier } }
                            { callable { $crate :: #module :: #wrapper_name } }
                        }
                    }
                })
                .collect::<Vec<_>>();
            let registrations_field = (!registrations.is_empty()).then(|| {
                quote! {
                    { registrations { #(#registrations)* } }
                }
            });
            quote! {
                {
                    framework_bundle_metadata_v1
                    { bundle_macro { #bundle_name } }
                    { source_module { #module } }
                    { source_order #source_order }
                    { kind #kind }
                    { execution #execution }
                    { output_type { #output_type } }
                    { qualifier { #qualifier } }
                    { dependencies { #(#dependencies)* } }
                    { callable { $crate :: #module :: #wrapper_name } }
                    #registrations_field
                }
            }
        })
        .collect()
}

fn qualifier_group(qualifier: Option<&LitStr>) -> TokenStream2 {
    match qualifier {
        Some(qualifier) => quote! { #qualifier },
        None => TokenStream2::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{bundle_metadata_records, parse_bundle_module};
    use proc_macro2::Ident;
    use quote::ToTokens;
    use syn::ItemMod;

    fn assert_parse_error(source: &str, expected: &str) {
        let module = syn::parse_str::<ItemMod>(source).expect("test module should parse");
        let error = match parse_bundle_module(module) {
            Ok(_) => panic!("expected parser error containing `{expected}`"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(expected),
            "expected `{expected}` in `{error}`"
        );
    }

    #[test]
    fn parses_registration_capabilities_on_provider_functions_and_structs() {
        let module = syn::parse_str::<ItemMod>(
            r#"
            mod test {
                #[provide(
                    qualifier = "special",
                    register(crate::Router, qualifier = "public"),
                    register(crate::Commands),
                )]
                fn routes() -> crate::Routes { crate::Routes }

                #[provide(register(crate::Router))]
                struct NamedRoutes { routes: crate::Routes }

                #[provide(register(crate::Router), register(crate::Commands))]
                struct UnitRoutes;
            }
            "#,
        )
        .expect("test module should parse");
        let parsed = parse_bundle_module(module).expect("registration declarations should parse");

        assert_eq!(parsed.declarations.len(), 3);
        let function = &parsed.declarations[0];
        assert_eq!(function.qualifier.as_ref().unwrap().value(), "special");
        assert_eq!(function.registrations.len(), 2);
        assert_eq!(
            function.registrations[0]
                .target_type
                .to_token_stream()
                .to_string(),
            "crate :: Router"
        );
        assert_eq!(
            function.registrations[0]
                .target_qualifier
                .as_ref()
                .unwrap()
                .value(),
            "public"
        );
        assert!(function.registrations[1].target_qualifier.is_none());
        assert_eq!(parsed.declarations[1].registrations.len(), 1);
        assert_eq!(parsed.declarations[2].registrations.len(), 2);

        let expanded_module = parsed.module.to_token_stream().to_string();
        assert!(expanded_module.contains("__framework_generated_registration_0_0"));
        assert!(expanded_module.contains("__framework_generated_registration_0_1"));
        assert!(expanded_module.contains("__framework_generated_registration_1_0"));
        assert!(expanded_module.contains("__framework_generated_registration_2_1"));
    }

    #[test]
    fn bundle_metadata_emits_registration_list_only_when_present() {
        let module = syn::parse_str::<ItemMod>(
            r#"mod test { #[provide] fn plain() -> crate::Plain { crate::Plain } }"#,
        )
        .expect("test module should parse");
        let parsed = parse_bundle_module(module).expect("plain provider should parse");
        let module_name = Ident::new("test", proc_macro2::Span::call_site());
        let bundle_name = Ident::new("test_bundle", proc_macro2::Span::call_site());
        let plain_record =
            bundle_metadata_records(&parsed.declarations, &module_name, &bundle_name);
        assert!(!plain_record[0].to_string().contains("registrations"));

        let module = syn::parse_str::<ItemMod>(
            r#"mod test { #[provide(register(crate::Router, qualifier = "public"))] fn routes() -> crate::Routes { crate::Routes } }"#,
        )
        .expect("test module should parse");
        let parsed = parse_bundle_module(module).expect("registered provider should parse");
        let record = bundle_metadata_records(&parsed.declarations, &module_name, &bundle_name);
        let record = record[0].to_string();
        assert!(record.contains("registrations"));
        assert!(record.contains("registration 0"));
        assert!(record.contains("target_type { crate :: Router }"));
        assert!(record.contains("target_qualifier { \"public\" }"));
        assert!(
            record.contains("$ crate :: test :: __framework_generated_registration_0_0"),
            "{record}"
        );
    }

    #[test]
    fn rejects_empty_registration_qualifier() {
        assert_parse_error(
            r#"mod test { #[provide(register(crate::Router, qualifier = ""))] fn provide() -> u8 { 1 } }"#,
            "qualifier must not be empty",
        );
    }

    #[test]
    fn rejects_duplicate_registration_target_keys_but_allows_distinct_qualifiers() {
        assert_parse_error(
            "mod test { #[provide(register(crate::Router), register(crate::Router))] fn provide() -> u8 { 1 } }",
            "duplicate registration capability for the same target type and qualifier",
        );

        let module = syn::parse_str::<ItemMod>(
            r#"mod test { #[provide(register(crate::Router, qualifier = "public"), register(crate::Router, qualifier = "internal"))] fn provide() -> u8 { 1 } }"#,
        )
        .expect("test module should parse");
        let parsed = parse_bundle_module(module)
            .expect("different registration qualifiers are distinct target keys");
        assert_eq!(parsed.declarations[0].registrations.len(), 2);
    }

    #[test]
    fn rejects_unsafe_provider_function() {
        assert_parse_error(
            "mod test { #[provide] unsafe fn provide() -> u8 { 1 } }",
            "unsafe provider functions are not supported",
        );
    }

    #[test]
    fn rejects_extern_provider_function() {
        assert_parse_error(
            "mod test { #[provide] extern \"C\" fn provide() -> u8 { 1 } }",
            "extern ABI provider functions are not supported",
        );
    }

    #[test]
    fn rejects_generic_hook_function() {
        assert_parse_error(
            "mod test { #[hook(start)] fn start<T>() {} }",
            "generic hooks are not supported",
        );
    }

    #[test]
    fn rejects_unsafe_hook_function() {
        assert_parse_error(
            "mod test { #[hook(start)] unsafe fn start() {} }",
            "unsafe hook functions are not supported",
        );
    }

    #[test]
    fn rejects_extern_hook_function() {
        assert_parse_error(
            "mod test { #[hook(start)] extern \"C\" fn start() {} }",
            "extern ABI hook functions are not supported",
        );
    }
}
