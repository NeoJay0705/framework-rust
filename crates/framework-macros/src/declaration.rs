use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{
    Attribute, Fields, FnArg, Item, ItemFn, ItemMod, ItemStruct, LitInt, LitStr, Meta, ReturnType,
    Signature, Type, Visibility, parse::Parse, parse::ParseStream, parse_quote, spanned::Spanned,
    visit::Visit,
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
    pub dependencies: Vec<Dependency>,
    pub is_async: bool,
    pub wrapper_name: Ident,
    pub wrapper: TokenStream2,
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
                    match parse_qualifier(attr).and_then(|qualifier| {
                        parse_function_dependencies(&mut function.sig).and_then(|dependencies| {
                            provider_function(function, qualifier, dependencies, source_order)
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
                    match parse_qualifier(attr).and_then(|qualifier| {
                        provider_struct(structure, qualifier, &module_name, source_order)
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
    Ok(Declaration {
        source_order,
        kind: DeclarationKind::Provider,
        output_type: (**output_type).clone(),
        qualifier,
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
        dependencies,
        is_async: function.sig.asyncness.is_some(),
        wrapper_name,
        wrapper,
    })
}

fn provider_struct(
    structure: &mut ItemStruct,
    qualifier: Option<LitStr>,
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

fn parse_qualifier(attribute: &Attribute) -> syn::Result<Option<LitStr>> {
    let Meta::List(list) = &attribute.meta else {
        if matches!(attribute.meta, Meta::Path(_)) {
            return Ok(None);
        }
        return Err(syn::Error::new_spanned(
            attribute,
            "expected `#[provide(qualifier = \"name\")]`",
        ));
    };

    let parsed = syn::parse2::<ProvideArgs>(list.tokens.clone())?;
    if let Some(qualifier) = &parsed.qualifier {
        validate_nonempty_qualifier(qualifier)?;
    }
    Ok(parsed.qualifier)
}

struct ProvideArgs {
    qualifier: Option<LitStr>,
}

impl Parse for ProvideArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        if key != "qualifier" {
            return Err(syn::Error::new(
                key.span(),
                "expected `qualifier = \"name\"`",
            ));
        }
        input.parse::<syn::Token![=]>()?;
        let qualifier: LitStr = input.parse()?;
        if input.peek(syn::Token![,]) {
            input.parse::<syn::Token![,]>()?;
        }
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after qualifier"));
        }
        Ok(Self {
            qualifier: Some(qualifier),
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
    declarations.push(declaration);
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
    use super::parse_bundle_module;
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
