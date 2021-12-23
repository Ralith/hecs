use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DeriveInput, Error, Result};

use crate::common::struct_fields;

pub fn derive(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident;
    let data = match input.data {
        syn::Data::Struct(s) => s,
        _ => {
            return Err(Error::new_spanned(
                ident,
                "derive(DynamicBundleClone) does not support enums or unions",
            ))
        }
    };
    let (tys, field_members) = struct_fields(&data.fields);
    let generics = add_additional_bounds_to_generic_params(input.generics);

    let dyn_bundle_code = gen_dynamic_bundle_impl(&ident, &generics, &field_members, &tys);
    Ok(dyn_bundle_code)
}

fn gen_dynamic_bundle_impl(
    ident: &syn::Ident,
    generics: &syn::Generics,
    field_members: &[syn::Member],
    tys: &[&syn::Type],
) -> TokenStream2 {
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        unsafe impl #impl_generics ::hecs::DynamicBundleClone for #ident #ty_generics #where_clause {
            #[allow(clippy::forget_copy)]
            unsafe fn put_with_clone(mut self, mut f: impl ::std::ops::FnMut(*mut u8, ::hecs::TypeInfo, DynamicClone)) {
                #(
                    f(
                        (&mut self.#field_members as *mut #tys).cast::<u8>(),
                        ::hecs::TypeInfo::of::<#tys>(),
                        ::hecs::DynamicClone::new::<#tys>()
                    );
                    ::std::mem::forget(self.#field_members);
                )*
            }
        }
    }
}

fn make_component_trait_bound() -> syn::TraitBound {
    syn::TraitBound {
        paren_token: None,
        modifier: syn::TraitBoundModifier::None,
        lifetimes: None,
        path: syn::parse_quote!(::std::clone::Clone),
    }
}

fn add_additional_bounds_to_generic_params(mut generics: syn::Generics) -> syn::Generics {
    for tp in generics.type_params_mut() {
        tp.bounds
            .push(syn::TypeParamBound::Trait(make_component_trait_bound()))
    }
    generics
}
