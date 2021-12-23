use std::borrow::Cow;

use proc_macro2::Span;

pub fn struct_fields(fields: &syn::Fields) -> (Vec<&syn::Type>, Vec<syn::Member>) {
    match fields {
        syn::Fields::Named(ref fields) => fields
            .named
            .iter()
            .map(|f| (&f.ty, syn::Member::Named(f.ident.clone().unwrap())))
            .unzip(),
        syn::Fields::Unnamed(ref fields) => fields
            .unnamed
            .iter()
            .enumerate()
            .map(|(i, f)| {
                (
                    &f.ty,
                    syn::Member::Unnamed(syn::Index {
                        index: i as u32,
                        span: Span::call_site(),
                    }),
                )
            })
            .unzip(),
        syn::Fields::Unit => (Vec::new(), Vec::new()),
    }
}

pub fn member_as_idents(members: &[syn::Member]) -> Vec<Cow<'_, syn::Ident>> {
    members
        .iter()
        .map(|member| match member {
            syn::Member::Named(ident) => Cow::Borrowed(ident),
            &syn::Member::Unnamed(syn::Index { index, span }) => {
                Cow::Owned(syn::Ident::new(&format!("tuple_field_{}", index), span))
            }
        })
        .collect()
}
