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

use std::borrow::Cow;

use proc_macro::TokenStream;
use proc_macro2::Span;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Error, Result};

/// Implement `Bundle` for a monomorphic struct
///
/// Bundles can be passed directly to `World::spawn` and `World::insert`, and obtained from
/// `World::remove`. Monomorphic `Bundle` implementations are slightly more efficient than the
/// polymorphic implementations for tuples, and can be convenient when combined with other derives
/// like `serde::Deserialize`.
#[allow(clippy::cognitive_complexity)]
#[proc_macro_derive(Bundle)]
pub fn derive_bundle(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive_bundle_(input) {
        Ok(ts) => ts,
        Err(e) => e.to_compile_error(),
    }
    .into()
}

fn derive_bundle_(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident;
    let data = match input.data {
        syn::Data::Struct(s) => s,
        _ => {
            return Err(Error::new_spanned(
                ident,
                "derive(Bundle) does not support enums or unions",
            ))
        }
    };
    let (tys, field_members) = struct_fields(&data.fields);
    let field_idents = member_as_idents(&field_members);
    let generics = add_additional_bounds_to_generic_params(input.generics);

    let dyn_bundle_code = gen_dynamic_bundle_impl(&ident, &generics, &field_members, &tys);
    let bundle_code = if tys.is_empty() {
        gen_unit_struct_bundle_impl(ident, &generics)
    } else {
        gen_bundle_impl(&ident, &generics, &field_members, &field_idents, &tys)
    };
    let mut ts = dyn_bundle_code;
    ts.extend(bundle_code);
    Ok(ts)
}

fn gen_dynamic_bundle_impl(
    ident: &syn::Ident,
    generics: &syn::Generics,
    field_members: &[syn::Member],
    tys: &[&syn::Type],
) -> TokenStream2 {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        impl #impl_generics ::hecs::DynamicBundle for #ident #ty_generics #where_clause {
            fn with_ids<__hecs__T>(&self, f: impl ::std::ops::FnOnce(&[::std::any::TypeId]) -> __hecs__T) -> __hecs__T {
                <Self as ::hecs::Bundle>::with_static_ids(f)
            }

            fn type_info(&self) -> ::std::vec::Vec<::hecs::TypeInfo> {
                <Self as ::hecs::Bundle>::static_type_info()
            }

            #[allow(clippy::forget_copy)]
            unsafe fn put(mut self, mut f: impl ::std::ops::FnMut(*mut u8, ::hecs::TypeInfo)) {
                #(
                    f((&mut self.#field_members as *mut #tys).cast::<u8>(), ::hecs::TypeInfo::of::<#tys>());
                    ::std::mem::forget(self.#field_members);
                )*
            }
        }
    }
}

fn gen_bundle_impl(
    ident: &syn::Ident,
    generics: &syn::Generics,
    field_members: &[syn::Member],
    field_idents: &[Cow<syn::Ident>],
    tys: &[&syn::Type],
) -> TokenStream2 {
    let num_tys = tys.len();
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let with_static_ids_inner = quote! {
        {
            let mut dedup = ::std::collections::HashSet::new();
            for &(ty, name) in [#((::std::any::TypeId::of::<#tys>(), ::std::any::type_name::<#tys>())),*].iter() {
                if !dedup.insert(ty) {
                    ::std::panic!("{} has multiple {} fields; each type must occur at most once!", stringify!(#ident), name);
                }
            }

            let mut tys = [#((::std::mem::align_of::<#tys>(), ::std::any::TypeId::of::<#tys>())),*];
            tys.sort_unstable_by(|x, y| {
                ::std::cmp::Ord::cmp(&x.0, &y.0)
                    .reverse()
                    .then(::std::cmp::Ord::cmp(&x.1, &y.1))
            });
            let mut ids = [::std::any::TypeId::of::<()>(); #num_tys];
            for (id, info) in ::std::iter::Iterator::zip(ids.iter_mut(), tys.iter()) {
                *id = info.1;
            }
            ids
        }
    };
    let with_static_ids = if generics.params.is_empty() {
        quote! {
            #[allow(non_camel_case_types)]
            fn with_static_ids<__hecs__T>(f: impl ::std::ops::FnOnce(&[::std::any::TypeId]) -> __hecs__T) -> __hecs__T {
                ::hecs::lazy_static::lazy_static! {
                    static ref ELEMENTS: [::std::any::TypeId; #num_tys] = {
                        #with_static_ids_inner
                    };
                }

                f(&*ELEMENTS)
            }
        }
    } else {
        quote! {
            #[allow(non_camel_case_types)]
            fn with_static_ids<__hecs__T>(f: impl ::std::ops::FnOnce(&[::std::any::TypeId]) -> __hecs__T) -> __hecs__T {
                f(&#with_static_ids_inner)
            }
        }
    };
    quote! {
        impl #impl_generics ::hecs::Bundle for #ident #ty_generics #where_clause {
            #with_static_ids

            fn static_type_info() -> ::std::vec::Vec<::hecs::TypeInfo> {
                let mut info = ::std::vec![#(::hecs::TypeInfo::of::<#tys>()),*];
                info.sort_unstable();
                info
            }

            unsafe fn get(
                mut f: impl ::std::ops::FnMut(::hecs::TypeInfo) -> ::std::option::Option<::std::ptr::NonNull<u8>>,
            ) -> ::std::result::Result<Self, ::hecs::MissingComponent> {
                #(
                    let #field_idents = f(::hecs::TypeInfo::of::<#tys>())
                            .ok_or_else(::hecs::MissingComponent::new::<#tys>)?
                            .cast::<#tys>()
                            .as_ptr();
                )*
                ::std::result::Result::Ok(Self { #( #field_members: #field_idents.read(), )* })
            }
        }
    }
}

// no reason to generate a static for unit structs
fn gen_unit_struct_bundle_impl(ident: syn::Ident, generics: &syn::Generics) -> TokenStream2 {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        impl #impl_generics ::hecs::Bundle for #ident #ty_generics #where_clause {
            #[allow(non_camel_case_types)]
            fn with_static_ids<__hecs__T>(f: impl ::std::ops::FnOnce(&[::std::any::TypeId]) -> __hecs__T) -> __hecs__T { f(&[]) }
            fn static_type_info() -> ::std::vec::Vec<::hecs::TypeInfo> { ::std::vec::Vec::new() }

            unsafe fn get(
                mut f: impl ::std::ops::FnMut(::hecs::TypeInfo) -> Option<::std::ptr::NonNull<u8>>,
            ) -> Result<Self, ::hecs::MissingComponent> {
                Ok(Self {/* for some reason this works for all unit struct variations */})
            }
        }
    }
}

fn make_component_trait_bound() -> syn::TraitBound {
    syn::TraitBound {
        paren_token: None,
        modifier: syn::TraitBoundModifier::None,
        lifetimes: None,
        path: syn::parse_quote!(::hecs::Component),
    }
}

fn add_additional_bounds_to_generic_params(mut generics: syn::Generics) -> syn::Generics {
    generics.type_params_mut().for_each(|tp| {
        tp.bounds
            .push(syn::TypeParamBound::Trait(make_component_trait_bound()))
    });
    generics
}

fn struct_fields(fields: &syn::Fields) -> (Vec<&syn::Type>, Vec<syn::Member>) {
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

fn member_as_idents<'a>(members: &'a [syn::Member]) -> Vec<Cow<'a, syn::Ident>> {
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
