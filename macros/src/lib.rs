// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

/// Implement `Bundle` for a monomorphic struct
///
/// Using derived `Bundle` impls improves spawn performance and can be convenient when combined with
/// other derives like `serde::Deserialize`.
#[proc_macro_derive(Bundle)]
pub fn derive_bundle(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    if !input.generics.params.is_empty() {
        return TokenStream::from(
            quote! { compile_error!("derive(Bundle) does not support generics"); },
        );
    }
    let data = match input.data {
        syn::Data::Struct(s) => s,
        _ => {
            return TokenStream::from(
                quote! { compile_error!("derive(Bundle) only supports structs"); },
            )
        }
    };
    let ident = input.ident;
    let (tys, fields) = struct_fields(&data.fields);

    let n = tys.len();
    let code = quote! {
        impl ::hecs::DynamicBundle for #ident {
            fn with_ids<T>(&self, f: impl FnOnce(&[std::any::TypeId]) -> T) -> T {
                Self::with_static_ids(f)
            }

            fn type_info(&self) -> Vec<::hecs::TypeInfo> {
                let mut info = vec![#(::hecs::TypeInfo::of::<#tys>()),*];
                info.sort_unstable();
                info
            }

            fn put(mut self, dest: &mut impl ::hecs::ComponentSink) {
                #(dest.put(self.#fields);)*
            }
        }

        impl ::hecs::Bundle for #ident {
            fn with_static_ids<T>(f: impl FnOnce(&[std::any::TypeId]) -> T) -> T {
                use std::any::TypeId;
                use std::mem;

                use ::hecs::once_cell::sync::Lazy;

                static ELEMENTS: Lazy<[TypeId; #n]> = Lazy::new(|| {
                    let mut dedup = std::collections::HashSet::new();
                    for &(ty, name) in [#((std::any::TypeId::of::<#tys>(), std::any::type_name::<#tys>())),*].iter() {
                        if !dedup.insert(ty) {
                            panic!("{} has multiple {} fields; each type must occur at most once!", stringify!(#ident), name);
                        }
                    }

                    let mut tys = [#((mem::align_of::<#tys>(), TypeId::of::<#tys>())),*];
                    tys.sort_unstable_by(|x, y| x.0.cmp(&y.0).reverse().then(x.1.cmp(&y.1)));
                    let mut ids = [TypeId::of::<()>(); #n];
                    for (id, info) in ids.iter_mut().zip(tys.iter()) {
                        *id = info.1;
                    }
                    ids
                });

                f(&*ELEMENTS)
            }

            fn get(
                src: &mut impl ::hecs::ComponentSource
            ) -> Result<Self, MissingComponent> {
                unsafe {
                    Ok(Self {
                        #(#fields: src.get()?,)*
                    })
                }
            }
        }
    };
    TokenStream::from(code)
}

/// Implement `Query` for a struct whose fields are queries
///
/// The struct must have exactly one lifetime parameter, used for the queries.
#[proc_macro_derive(Query)]
pub fn derive_query(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let lifetimes = input.generics.lifetimes().collect::<Vec<_>>();

    let lifetime = match lifetimes[..] {
        [x] => x.lifetime.clone(),
        _ => {
            return TokenStream::from(
                quote! { compile_error!("derive(Query) must be applied to structs with exactly one unbounded lifetime parameter"); },
            );
        }
    };
    if input.generics.where_clause.is_some() {
        return TokenStream::from(
            quote! { compile_error!("derive(Query) does not support where clauses"); },
        );
    }
    let data = match input.data {
        syn::Data::Struct(s) => s,
        _ => {
            return TokenStream::from(
                quote! { compile_error!("derive(Query) only supports structs"); },
            )
        }
    };
    let ident = input.ident;
    let vis = input.vis;
    let fetch = syn::Ident::new(&format!("{}Fetch", ident), Span::call_site());

    let (tys, fields) = struct_fields(&data.fields);

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let fetch_def = match data.fields {
        syn::Fields::Named(_) => quote! {
            #[doc(hidden)]
            #vis struct #fetch #ty_generics #where_clause {
                #(
                    #fields: <#tys as Query<#lifetime>>::Fetch,
                )*
            }
        },
        syn::Fields::Unnamed(_) => quote! {
            #[doc(hidden)]
            #vis struct #fetch #ty_generics (
                #(
                    #fields: <#tys as Query<#lifetime>>::Fetch,
                )*
            ) #where_clause;
        },
        syn::Fields::Unit => quote! { struct #fetch #ty_generics #where_clause {} },
    };

    let code = quote! {
        #fetch_def

        impl #impl_generics ::hecs::Fetch<#lifetime> for #fetch #ty_generics #where_clause {
            type Item = #ident #ty_generics;

            fn get(archetype: & #lifetime Archetype) -> Option<Self> {
                Some(Self {
                    #(
                        #fields: <#tys as Query<#lifetime>>::Fetch::get(archetype)?,
                    )*
                })
            }

            unsafe fn next(&mut self) -> Self::Item {
                #ident {
                    #(
                        #fields: self.#fields.next(),
                    )*
                }
            }
        }

        impl #impl_generics ::hecs::Query<#lifetime> for #ident #ty_generics #where_clause {
            type Fetch = #fetch #ty_generics;

            fn borrow(state: &BorrowState) {
                #(
                    <#tys as Query>::borrow(state);
                )*
            }

            fn release(state: &BorrowState) {
                #(
                    <#tys as Query>::release(state);
                )*
            }
        }
    };
    TokenStream::from(code)
}

fn struct_fields(fields: &syn::Fields) -> (Vec<&syn::Type>, Vec<syn::Ident>) {
    match fields {
        syn::Fields::Named(ref fields) => fields
            .named
            .iter()
            .map(|f| (&f.ty, f.ident.clone().unwrap()))
            .unzip(),
        syn::Fields::Unnamed(ref fields) => fields
            .unnamed
            .iter()
            .enumerate()
            .map(|(i, f)| (&f.ty, syn::Ident::new(&i.to_string(), Span::call_site())))
            .unzip(),
        syn::Fields::Unit => (Vec::new(), Vec::new()),
    }
}
