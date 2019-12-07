extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

/// Implement `Bundle` for a monomorphic struct
///
/// Using derived `Bundle` impls improves spawn performance and can be convenient when combined with
/// other derives like `serde::Deserialize`.
///
/// ```
/// # use hecs::*;
/// # struct MeshId(&'static str);
/// # #[derive(Copy, Clone, PartialEq, Debug)]
/// # struct Position([f32; 3]);
/// #[derive(Bundle)]
/// struct StaticMesh {
///     mesh: MeshId,
///     position: Position,
/// }
///
/// let mut world = World::new();
/// let position = Position([1.0, 2.0, 3.0]);
/// let e = world.spawn(StaticMesh { position, mesh: MeshId("example.gltf") });
/// assert_eq!(*world.get::<Position>(e).unwrap(), position);
/// ```
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
    let tys = match data.fields {
        syn::Fields::Named(ref fields) => fields.named.iter().map(|f| &f.ty).collect(),
        syn::Fields::Unnamed(ref fields) => fields.unnamed.iter().map(|f| &f.ty).collect(),
        syn::Fields::Unit => Vec::new(),
    };
    let fields = match data.fields {
        syn::Fields::Named(ref fields) => fields
            .named
            .iter()
            .map(|f| f.ident.clone().unwrap())
            .collect(),
        syn::Fields::Unnamed(ref fields) => (0..fields.unnamed.len())
            .map(|i| syn::Ident::new(&i.to_string(), Span::call_site()))
            .collect(),
        syn::Fields::Unit => Vec::new(),
    };

    let n = tys.len();
    let code = quote! {
        impl ::hecs::Bundle for #ident {
            fn elements() -> &'static [std::any::TypeId] {
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
                &*ELEMENTS
            }
        }

        impl ::hecs::DynamicBundle for #ident {
            fn get_archetype(&self, table: &mut ::hecs::ArchetypeTable) -> u32 {
                table
                    .get_id(Self::elements())
                    .unwrap_or_else(|| {
                        let mut info = vec![#(::hecs::TypeInfo::of::<#tys>()),*];
                        info.sort_unstable();
                        table.alloc(info)
                    })
            }

            unsafe fn store(self, archetype: &mut ::hecs::Archetype, index: u32) {
                #(
                    archetype.put(self.#fields, index);
                )*
            }
        }
    };
    TokenStream::from(code)
}
