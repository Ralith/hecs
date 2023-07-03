use proc_macro2::Span;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DeriveInput, Error, Ident, Lifetime, Result, Type};

pub fn derive(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident;
    let vis = input.vis;
    let data = match input.data {
        syn::Data::Struct(s) => s,
        _ => {
            return Err(Error::new_spanned(
                ident,
                "derive(Query) may only be applied to structs",
            ))
        }
    };
    let lifetime = input
        .generics
        .lifetimes()
        .next()
        .map(|x| x.lifetime.clone());
    let lifetime = match lifetime {
        Some(x) => x,
        None => {
            return Err(Error::new_spanned(
                input.generics,
                "must have exactly one lifetime parameter",
            ))
        }
    };
    if input.generics.params.len() != 1 {
        return Err(Error::new_spanned(
            ident,
            "must have exactly one lifetime parameter and no type parameters",
        ));
    }

    let (fields, queries) = match data.fields {
        syn::Fields::Named(ref fields) => fields
            .named
            .iter()
            .map(|f| {
                (
                    syn::Member::Named(f.ident.clone().unwrap()),
                    query_ty(&lifetime, &f.ty),
                )
            })
            .unzip(),
        syn::Fields::Unnamed(ref fields) => fields
            .unnamed
            .iter()
            .enumerate()
            .map(|(i, f)| {
                (
                    syn::Member::Unnamed(syn::Index {
                        index: i as u32,
                        span: Span::call_site(),
                    }),
                    query_ty(&lifetime, &f.ty),
                )
            })
            .unzip(),
        syn::Fields::Unit => (Vec::new(), Vec::new()),
    };
    let fetches = queries
        .iter()
        .map(|ty| quote! { <#ty as ::hecs::Query>::Fetch })
        .collect::<Vec<_>>();
    let fetch_ident = Ident::new(&format!("{}Fetch", ident), Span::call_site());
    let fetch = match data.fields {
        syn::Fields::Named(_) => quote! {
            #vis struct #fetch_ident {
                #(
                    #fields: #fetches,
                )*
            }
        },
        syn::Fields::Unnamed(_) => quote! {
            #vis struct #fetch_ident(#(#fetches),*);
        },
        syn::Fields::Unit => quote! {
            #vis struct #fetch_ident;
        },
    };
    let state_ident = Ident::new(&format!("{}State", ident), Span::call_site());
    let state = match data.fields {
        syn::Fields::Named(_) => quote! {
            #[derive(Clone, Copy)]
            #vis struct #state_ident {
                #(
                    #fields: <#fetches as ::hecs::Fetch>::State,
                )*
            }
        },
        syn::Fields::Unnamed(_) => quote! {
            #[derive(Clone, Copy)]
            #vis struct #state_ident(#(<#fetches as ::hecs::Fetch>::State),*);
        },
        syn::Fields::Unit => quote! {
            #[derive(Clone, Copy)]
            #vis struct #state_ident;
        },
    };

    let intermediates = fields
        .iter()
        .map(|x| match x {
            syn::Member::Named(ref ident) => ident.clone(),
            syn::Member::Unnamed(ref index) => {
                Ident::new(&format!("field_{}", index.index), Span::call_site())
            }
        })
        .collect::<Vec<_>>();

    Ok(quote! {
        const _: () = {
            #[derive(Clone)]
            #fetch

            impl<'a> ::hecs::Query for #ident<'a> {
                type Item<'q> = #ident<'q>;

                type Fetch = #fetch_ident;

                #[allow(unused_variables)]
                unsafe fn get<'q>(fetch: &Self::Fetch, n: usize) -> Self::Item<'q> {
                    #(
                        let #intermediates: <#queries as ::hecs::Query>::Item<'q> = <#queries as ::hecs::Query>::get(&fetch.#fields, n);
                    )*
                    #ident {#(#fields: #intermediates,)*}
                }
            }

            #state

            unsafe impl ::hecs::Fetch for #fetch_ident {
                type State = #state_ident;

                fn dangling() -> Self {
                    Self {
                        #(
                            #fields: #fetches::dangling(),
                        )*
                    }
                }

                #[allow(unused_variables, unused_mut)]
                fn access(archetype: &::hecs::Archetype) -> ::std::option::Option<::hecs::Access> {
                    let mut access = ::hecs::Access::Iterate;
                    #(
                        access = ::core::cmp::max(access, #fetches::access(archetype)?);
                    )*
                    ::std::option::Option::Some(access)
                }

                #[allow(unused_variables)]
                fn borrow(archetype: &::hecs::Archetype, state: Self::State) {
                    #(#fetches::borrow(archetype, state.#fields);)*
                }

                #[allow(unused_variables)]
                fn prepare(archetype: &::hecs::Archetype) -> ::std::option::Option<Self::State> {
                    ::std::option::Option::Some(#state_ident {
                        #(
                            #fields: #fetches::prepare(archetype)?,
                        )*
                    })
                }

                #[allow(unused_variables)]
                fn execute(archetype: &::hecs::Archetype, state: Self::State) -> Self {
                    Self {
                        #(
                            #fields: #fetches::execute(archetype, state.#fields),
                        )*
                    }
                }

                #[allow(unused_variables)]
                fn release(archetype: &::hecs::Archetype, state: Self::State) {
                    #(#fetches::release(archetype, state.#fields);)*
                }

                #[allow(unused_variables, unused_mut)]
                fn for_each_borrow(mut f: impl ::core::ops::FnMut(::core::any::TypeId, bool)) {
                    #(
                        <#fetches as ::hecs::Fetch>::for_each_borrow(&mut f);
                    )*
                }
            }
        };
    })
}

fn query_ty(lifetime: &Lifetime, ty: &Type) -> TokenStream2 {
    struct Visitor<'a> {
        replace: &'a Lifetime,
    }
    impl syn::visit_mut::VisitMut for Visitor<'_> {
        fn visit_lifetime_mut(&mut self, l: &mut Lifetime) {
            if l == self.replace {
                *l = Lifetime::new("'static", Span::call_site());
            }
        }
    }

    let mut ty = ty.clone();
    syn::visit_mut::visit_type_mut(&mut Visitor { replace: lifetime }, &mut ty);
    quote! { #ty }
}
