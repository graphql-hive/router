//! Generic "sum a `usize` method over every field" derive engine.
//!
//! The field-walking logic (structs, enums, empty enums, unions rejected) does not care
//! which trait it implements. A concrete derive only supplies a [`SumConfig`]:
//! which trait path to implement, which method to sum, which helper attribute carries
//! a custom `bound`, and the derive name used in error messages.
//!
//! `HeapSize` in `heap_bytes` is one instantiation; future traits with the same shape
//! reuse [`expand`] without copying this logic.

use quote::quote;
use syn::{
    punctuated::Punctuated, Data, DeriveInput, Fields, Index, Meta, WhereClause, WherePredicate,
};

/// Everything a concrete derive must specify to reuse [`expand`].
pub(crate) struct SumConfig {
    /// e.g. `crate::heap_size::HeapSize`.
    pub(crate) trait_path: syn::Path,
    /// e.g. `heap_size`.
    pub(crate) method: syn::Ident,
    /// Helper attribute carrying `bound = "..."`, e.g. `"heap_size"`.
    pub(crate) helper_attr: &'static str,
    /// Derive name used in error messages, e.g. `"HeapSize"`.
    pub(crate) derive_name: &'static str,
}

/// Pure expansion over an already-parsed input, so it stays testable and reusable
/// outside a `proc_macro::TokenStream` context. The caller maps `Err` to
/// `to_compile_error()`.
pub(crate) fn expand(
    input: &DeriveInput,
    config: &SumConfig,
) -> syn::Result<proc_macro2::TokenStream> {
    let SumConfig {
        trait_path,
        method,
        helper_attr,
        derive_name,
    } = config;

    let body = match &input.data {
        Data::Struct(data) => {
            let terms = sum_fields(&data.fields, config, |member| quote!(self.#member));
            quote!(#terms)
        }
        Data::Enum(data) => {
            if data.variants.is_empty() {
                // an empty enum has no values, so nothing can call this
                quote!(0)
            } else {
                let arms = data.variants.iter().map(|variant| {
                    let name = &variant.ident;
                    let bindings = binding_names(&variant.fields);
                    let terms = sum_fields(&variant.fields, config, |member| {
                        let binding = binding_ident(&member);
                        quote!((*#binding))
                    });
                    match &variant.fields {
                        Fields::Named(_) => quote!(Self::#name { #(#bindings),* } => #terms),
                        Fields::Unnamed(_) => quote!(Self::#name(#(#bindings),*) => #terms),
                        Fields::Unit => quote!(Self::#name => 0),
                    }
                });
                quote!(match self { #(#arms),* })
            }
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                format!(
                    "{derive_name} cannot be derived for a union: its fields overlap, so there \
                     is no single answer for what it owns"
                ),
            ));
        }
    };

    let name = &input.ident;
    let (impl_generics, type_generics, inferred_where) = input.generics.split_for_impl();

    let where_clause = match custom_bound(input, helper_attr)? {
        Some(bound) => quote!(where #bound),
        None => {
            // every type parameter has to carry the trait, the way `#[derive(Clone)]` does
            let params = input
                .generics
                .type_params()
                .map(|param| {
                    let ident = &param.ident;
                    quote!(#ident: #trait_path)
                });
            match inferred_where {
                Some(WhereClause { predicates, .. }) => quote!(where #predicates, #(#params),*),
                None => quote!(where #(#params),*),
            }
        }
    };

    Ok(quote! {
        impl #impl_generics #trait_path for #name #type_generics #where_clause {
            fn #method(&self) -> usize {
                #body
            }
        }
    })
}

/// `#[helper_attr(bound = "...")]`, parsed into where-predicates.
fn custom_bound(
    input: &DeriveInput,
    helper_attr: &str,
) -> syn::Result<Option<Punctuated<WherePredicate, syn::Token![,]>>> {
    for attr in &input.attrs {
        if !attr.path().is_ident(helper_attr) {
            continue;
        }
        let meta = attr.parse_args::<Meta>()?;
        let Meta::NameValue(name_value) = meta else {
            return Err(syn::Error::new_spanned(
                attr,
                format!("expected #[{helper_attr}(bound = \"...\")]"),
            ));
        };
        if !name_value.path.is_ident("bound") {
            return Err(syn::Error::new_spanned(
                &name_value.path,
                "the only supported option is `bound`",
            ));
        }
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(text),
            ..
        }) = &name_value.value
        else {
            return Err(syn::Error::new_spanned(
                &name_value.value,
                "`bound` takes a string of where-predicates",
            ));
        };
        return text
            .parse_with(Punctuated::<WherePredicate, syn::Token![,]>::parse_terminated)
            .map(Some);
    }
    Ok(None)
}

/// `Trait::method(&a) + Trait::method(&b) + ...`, or `0` when there are no fields.
fn sum_fields(
    fields: &Fields,
    config: &SumConfig,
    access: impl Fn(syn::Member) -> proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let SumConfig {
        trait_path,
        method,
        ..
    } = config;
    let terms: Vec<_> = members(fields)
        .map(|member| {
            let value = access(member);
            quote!(#trait_path::#method(&#value))
        })
        .collect();

    if terms.is_empty() {
        quote!(0)
    } else {
        quote!(#(#terms)+*)
    }
}

pub(crate) fn members(fields: &Fields) -> impl Iterator<Item = syn::Member> + '_ {
    fields
        .iter()
        .enumerate()
        .map(|(index, field)| match &field.ident {
            Some(ident) => syn::Member::Named(ident.clone()),
            None => syn::Member::Unnamed(Index::from(index)),
        })
}

/// Names every field of a variant, so a field added to it cannot go unbound.
pub(crate) fn binding_names(fields: &Fields) -> Vec<proc_macro2::TokenStream> {
    members(fields)
        .map(|member| {
            let binding = binding_ident(&member);
            match member {
                syn::Member::Named(name) => quote!(#name: #binding),
                syn::Member::Unnamed(_) => quote!(#binding),
            }
        })
        .collect()
}

pub(crate) fn binding_ident(member: &syn::Member) -> proc_macro2::Ident {
    match member {
        syn::Member::Named(name) => proc_macro2::Ident::new(&format!("field_{name}"), name.span()),
        syn::Member::Unnamed(index) => {
            proc_macro2::Ident::new(&format!("field_{}", index.index), index.span)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn other_config() -> SumConfig {
        SumConfig {
            trait_path: syn::parse_quote!(crate::bytes::HeapBytes),
            method: quote::format_ident!("heap_bytes"),
            helper_attr: "heap_bytes",
            derive_name: "HeapBytes",
        }
    }

    #[test]
    fn struct_sums_fields_through_configured_trait() {
        let input: DeriveInput = syn::parse_quote! {
            struct Foo { a: String, b: u32 }
        };
        let tokens = expand(&input, &other_config())
            .expect("struct expands")
            .to_string();
        assert!(
            tokens.contains("crate :: bytes :: HeapBytes"),
            "trait path comes from config, not hardcoded: {tokens}"
        );
        assert!(
            tokens.contains("fn heap_bytes"),
            "method name comes from config: {tokens}"
        );
        assert!(
            !tokens.contains("heap_size"),
            "no HeapSize leakage into other derives: {tokens}"
        );
    }

    #[test]
    fn custom_bound_replaces_inferred_bounds() {
        let input: DeriveInput = syn::parse_quote! {
            #[heap_bytes(bound = "T: Clone")]
            struct Foo<T> { inner: T }
        };
        let tokens = expand(&input, &other_config())
            .expect("custom bound expands")
            .to_string();
        assert!(
            tokens.contains("T : Clone"),
            "custom bound is honored: {tokens}"
        );
        assert!(
            !tokens.contains("T : crate"),
            "custom bound replaces inference: {tokens}"
        );
    }

    #[test]
    fn union_error_names_the_configured_derive() {
        let input: DeriveInput = syn::parse_quote! {
            union Foo { a: u32, b: f32 }
        };
        let err = expand(&input, &other_config()).expect_err("union is rejected");
        assert!(
            err.to_string().contains("HeapBytes"),
            "error names the derive from config: {err}"
        );
    }
}
