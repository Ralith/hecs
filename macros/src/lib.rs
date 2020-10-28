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
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            input.generics,
            "derive(Bundle) does not support generics",
        ));
    }
    let data = match input.data {
        syn::Data::Struct(s) => s,
        _ => {
            return Err(Error::new_spanned(
                ident,
                "derive(Bundle) does not support enums or unions",
            ))
        }
    };
    let (tys, fields) = struct_fields(&data.fields);

    let dyn_bundle_code = gen_dynamic_bundle_impl(&ident, &fields, &tys);
    let num_tys = tys.len();
    let bundle_code = if num_tys == 0 {
        gen_unit_struct_bundle_impl(ident)
    } else {
        gen_bundle_impl(&ident, &fields, &tys)
    };
    let mut ts = dyn_bundle_code;
    ts.extend(bundle_code);
    Ok(ts)
}

fn gen_dynamic_bundle_impl(
    ident: &syn::Ident,
    fields: &[syn::Ident],
    tys: &[&syn::Type],
) -> TokenStream2 {
    quote! {
        impl ::hecs::DynamicBundle for #ident {
            fn with_ids<T>(&self, f: impl FnOnce(&[std::any::TypeId]) -> T) -> T {
                Self::with_static_ids(f)
            }

            fn type_info(&self) -> Vec<::hecs::TypeInfo> {
                Self::static_type_info()
            }

            #[allow(clippy::forget_copy)]
            unsafe fn put(mut self, mut f: impl FnMut(*mut u8, ::hecs::TypeInfo)) {
                #(
                    f((&mut self.#fields as *mut #tys).cast::<u8>(), ::hecs::TypeInfo::of::<#tys>());
                    std::mem::forget(self.#fields);
                )*
            }
        }
    }
}

fn gen_bundle_impl(ident: &syn::Ident, fields: &[syn::Ident], tys: &[&syn::Type]) -> TokenStream2 {
    let num_tys = tys.len();
    quote! {
        impl ::hecs::Bundle for #ident {
            fn with_static_ids<T>(f: impl FnOnce(&[std::any::TypeId]) -> T) -> T {
                use std::any::TypeId;
                use std::mem;

                ::hecs::lazy_static::lazy_static! {
                    static ref ELEMENTS: [TypeId; #num_tys] = {
                        let mut dedup = std::collections::HashSet::new();
                        for &(ty, name) in [#((std::any::TypeId::of::<#tys>(), std::any::type_name::<#tys>())),*].iter() {
                            if !dedup.insert(ty) {
                                panic!("{} has multiple {} fields; each type must occur at most once!", stringify!(#ident), name);
                            }
                        }

                        let mut tys = [#((mem::align_of::<#tys>(), TypeId::of::<#tys>())),*];
                        tys.sort_unstable_by(|x, y| x.0.cmp(&y.0).reverse().then(x.1.cmp(&y.1)));
                        let mut ids = [TypeId::of::<()>(); #num_tys];
                        for (id, info) in ids.iter_mut().zip(tys.iter()) {
                            *id = info.1;
                        }
                        ids
                    };
                }

                f(&*ELEMENTS)
            }

            fn static_type_info() -> Vec<::hecs::TypeInfo> {
                let mut info = vec![#(::hecs::TypeInfo::of::<#tys>()),*];
                info.sort_unstable();
                info
            }

            unsafe fn get(
                mut f: impl FnMut(::hecs::TypeInfo) -> Option<std::ptr::NonNull<u8>>,
            ) -> Result<Self, ::hecs::MissingComponent> {
                #(
                    let #fields = f(::hecs::TypeInfo::of::<#tys>())
                            .ok_or_else(::hecs::MissingComponent::new::<#tys>)?
                            .cast::<#tys>()
                        .as_ptr();
                )*
                Ok(Self { #( #fields: #fields.read(), )* })
            }
        }
    }
}

fn gen_unit_struct_bundle_impl(ident: syn::Ident) -> TokenStream2 {
    quote! {
        impl ::hecs::Bundle for #ident {
            fn with_static_ids<T>(f: impl FnOnce(&[std::any::TypeId]) -> T) -> T { f(&[]) }
            fn static_type_info() -> Vec<::hecs::TypeInfo> { Vec::new() }

            unsafe fn get(
                mut f: impl FnMut(::hecs::TypeInfo) -> Option<std::ptr::NonNull<u8>>,
            ) -> Result<Self, ::hecs::MissingComponent> {
                Ok(Self {/* for some reason this works for all unit struct variations */})
            }
        }
    }
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
