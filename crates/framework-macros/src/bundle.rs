use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, ItemMod, parse2};

use crate::declaration::{BundleArgs, bundle_metadata_records, parse_bundle_module};

pub fn expand(attribute: TokenStream2, item: TokenStream2) -> syn::Result<TokenStream2> {
    let args = parse2::<BundleArgs>(attribute)?;
    let module = parse2::<ItemMod>(item)?;
    let module_name = module.ident.clone();
    let parsed = parse_bundle_module(module)?;
    let macro_name: Ident = args.name;
    let records = bundle_metadata_records(&parsed.declarations, &module_name, &macro_name);
    let module = parsed.module;

    Ok(quote! {
        #module

        // Bundle records intentionally retain source `crate::` type paths; the
        // application compositor rewrites them to the imported crate prefix.
        #[allow(clippy::crate_in_macro_def)]
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #macro_name {
            (@__framework_forward $callback:ident; [$($acc:tt)*]) => {
                $callback! {
                    [$($acc)*]
                    [#(#records)*]
                }
            };
        }
    })
}
