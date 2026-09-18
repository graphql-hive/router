use proc_macro::TokenStream;

use crate::sum::{expand, SumConfig};

fn config() -> SumConfig {
    SumConfig {
        trait_path: syn::parse_quote!(crate::heap_size::HeapSize),
        method: quote::format_ident!("heap_size"),
        helper_attr: "heap_size",
        derive_name: "HeapSize",
    }
}

pub(crate) fn derive_heap_size(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    let config = config();
    match expand(&input, &config) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
