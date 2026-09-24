//! Procedural macros for the framework.

mod application;
mod bundle;
mod declaration;

use proc_macro::TokenStream;

mod graph;

#[proc_macro_attribute]
pub fn bundle(attr: TokenStream, item: TokenStream) -> TokenStream {
    match bundle::expand(attr.into(), item.into()) {
        Ok(output) => output.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn application(_attr: TokenStream, _item: TokenStream) -> TokenStream {
    match application::expand(_attr.into(), _item.into()) {
        Ok(output) => output.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro]
pub fn __compose(_input: TokenStream) -> TokenStream {
    match application::compose(_input.into()) {
        Ok(output) => output.into(),
        Err(error) => error.to_compile_error().into(),
    }
}
