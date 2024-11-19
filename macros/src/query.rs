use proc_macro2::Span;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DataEnum, DataStruct, DeriveInput, Error, Ident, Lifetime, Result, Type, Visibility};

pub fn derive(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident;

    match input.data {
        syn::Data::Struct(_) | syn::Data::Enum(_) => {},
        _ => {
            return Err(Error::new_spanned(
                ident,
                "derive(Query) may only be applied to structs and enums",
            ))
        }
    }

    let vis = input.vis;
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

    match input.data {
        syn::Data::Struct(data_struct) => derive_struct(ident, vis, data_struct, lifetime),
        syn::Data::Enum(data_enum) => derive_enum(ident, vis, data_enum, lifetime),
        _ => unreachable!()
    }
}

fn derive_struct(ident: Ident, vis: Visibility, data: DataStruct, lifetime: Lifetime) -> Result<TokenStream2> {
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
                        span: Span::mixed_site(),
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
    let fetch_ident = Ident::new(&format!("{}Fetch", ident), Span::mixed_site());
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
    let state_ident = Ident::new(&format!("{}State", ident), Span::mixed_site());
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
                Ident::new(&format!("field_{}", index.index), Span::mixed_site())
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
                fn access(archetype: &::hecs::Archetype) -> ::core::option::Option<::hecs::Access> {
                    let mut access = ::hecs::Access::Iterate;
                    #(
                        access = ::core::cmp::max(access, #fetches::access(archetype)?);
                    )*
                    ::core::option::Option::Some(access)
                }

                #[allow(unused_variables)]
                fn borrow(archetype: &::hecs::Archetype, state: Self::State) {
                    #(#fetches::borrow(archetype, state.#fields);)*
                }

                #[allow(unused_variables)]
                fn prepare(archetype: &::hecs::Archetype) -> ::core::option::Option<Self::State> {
                    ::core::option::Option::Some(#state_ident {
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

fn derive_enum(enum_ident: Ident, vis: Visibility, data: DataEnum, lifetime: Lifetime) -> Result<TokenStream2> {
    let mut dangling_constructor = None;
    let mut fetch_variants = TokenStream2::new();
    let mut state_variants = TokenStream2::new();
    let mut query_get_variants = TokenStream2::new();
    let mut fetch_access_variants = TokenStream2::new();
    let mut fetch_borrow_variants = TokenStream2::new();
    let mut fetch_prepare_variants = TokenStream2::new();
    let mut fetch_execute_variants = TokenStream2::new();
    let mut fetch_release_variants = TokenStream2::new();
    let mut fetch_for_each_borrow = TokenStream2::new();

    for variant in &data.variants {
        let (fields, queries) = match variant.fields {
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
                            span: Span::mixed_site(),
                        }),
                        query_ty(&lifetime, &f.ty),
                    )
                })
                .unzip(),
            syn::Fields::Unit => (Vec::new(), Vec::new()),
        };

        let ident = variant.ident.clone();

        if ident.to_string() == "__HecsDanglingFetch__" {
            return Err(Error::new_spanned(
                ident,
                "derive(Query) reserves this identifier for internal use",
            ))
        }

        let named_fields = fields
            .iter()
            .map(|x| match x {
                syn::Member::Named(ref ident) => ident.clone(),
                syn::Member::Unnamed(ref index) => {
                    Ident::new(&format!("field_{}", index.index), Span::mixed_site())
                }
            })
            .collect::<Vec<_>>();

        let fetches = queries
            .iter()
            .map(|ty| quote! { <#ty as ::hecs::Query>::Fetch })
            .collect::<Vec<_>>();

        if dangling_constructor.is_none() && fields.is_empty() {
            dangling_constructor = Some(quote! {
                Self::#ident {}
            });
        }

        fetch_variants.extend(quote! {
            #ident {
                #(
                    #named_fields: #fetches,
                )*
            },
        });

        state_variants.extend(quote! {
            #ident {
                #(
                    #named_fields: <#fetches as ::hecs::Fetch>::State,
                )*
            },
        });

        query_get_variants.extend(match variant.fields {
            syn::Fields::Named(_) | syn::Fields::Unnamed(_) => quote! {
                Self::Fetch::#ident { #(#named_fields),* } => {
                    #(
                        let #named_fields: <#queries as ::hecs::Query>::Item<'q> = <#queries as ::hecs::Query>::get(#named_fields, n);
                    )*
                    Self::Item::#ident { #( #fields: #named_fields,)* }
                },
            },
            syn::Fields::Unit => quote! {
                Self::Fetch::#ident {} => Self::Item::#ident,
            },
        });

        fetch_access_variants.extend(quote! {
            'block: {
                let mut access = ::hecs::Access::Iterate;
                #(
                    if let ::core::option::Option::Some(new_access) = #fetches::access(archetype) {
                        access = ::core::cmp::max(access, new_access);
                    } else {
                        break 'block;
                    }
                )*
                return ::core::option::Option::Some(access)
            }
        });

        fetch_borrow_variants.extend(quote! {
            Self::State::#ident { #(#named_fields),* } => {
                #(
                    #fetches::borrow(archetype, #named_fields);
                )*
            },
        });

        fetch_prepare_variants.extend(quote! {
            'block: {
                #(
                    let ::core::option::Option::Some(#named_fields) = #fetches::prepare(archetype) else {
                        break 'block;
                    };
                )*
                return ::core::option::Option::Some(Self::State::#ident { #(#named_fields,)* });
            }
        });

        fetch_execute_variants.extend(quote! {
            Self::State::#ident { #(#named_fields),* } => {
                return Self::#ident {
                    #(
                        #named_fields: #fetches::execute(archetype, #named_fields),
                    )*
                };
            },
        });

        fetch_release_variants.extend( quote! {
            Self::State::#ident { #(#named_fields),* } => {
                #(
                    #fetches::release(archetype, #named_fields);
                )*
            },
        });

        fetch_for_each_borrow.extend(quote! {
            #(
                <#fetches as ::hecs::Fetch>::for_each_borrow(&mut f);
            )*
        });
    }

    let dangling_constructor = if let Some(dangling_constructor) = dangling_constructor {
        dangling_constructor
    } else {
        fetch_variants.extend(quote! {
            __HecsDanglingFetch__,
        });
        query_get_variants.extend(quote! {
            Self::Fetch::__HecsDanglingFetch__ => panic!("Called get() with dangling fetch"),
        });
        quote! {
            Self::__HecsDanglingFetch__
        }
    };

    let fetch_ident = Ident::new(&format!("{}Fetch", enum_ident), Span::mixed_site());
    let fetch = quote! {
        #vis enum #fetch_ident {
            #fetch_variants
        }
    };

    let state_ident = Ident::new(&format!("{}State", enum_ident), Span::mixed_site());
    let state = quote! {
        #vis enum #state_ident {
            #state_variants
        }
    };

    Ok(quote! {
        const _: () = {
            #[derive(Clone)]
            #fetch

            impl<'a> ::hecs::Query for #enum_ident<'a> {
                type Item<'q> = #enum_ident<'q>;

                type Fetch = #fetch_ident;

                #[allow(unused_variables)]
                unsafe fn get<'q>(fetch: &Self::Fetch, n: usize) -> Self::Item<'q> {
                    match fetch {
                        #query_get_variants
                    }
                }
            }

            #[derive(Clone, Copy)]
            #state

            unsafe impl ::hecs::Fetch for #fetch_ident {
                type State = #state_ident;

                fn dangling() -> Self {
                    #dangling_constructor
                }

                #[allow(unused_variables, unused_mut, unreachable_code)]
                fn access(archetype: &::hecs::Archetype) -> ::core::option::Option<::hecs::Access> {
                    #fetch_access_variants
                    ::core::option::Option::None
                }

                #[allow(unused_variables)]
                fn borrow(archetype: &::hecs::Archetype, state: Self::State) {
                    match state {
                        #fetch_borrow_variants
                    }
                }

                #[allow(unused_variables, unreachable_code)]
                fn prepare(archetype: &::hecs::Archetype) -> ::core::option::Option<Self::State> {
                    #fetch_prepare_variants
                    ::core::option::Option::None
                }

                #[allow(unused_variables)]
                fn execute(archetype: &::hecs::Archetype, state: Self::State) -> Self {
                    match state {
                        #fetch_execute_variants
                    }
                }

                #[allow(unused_variables)]
                fn release(archetype: &::hecs::Archetype, state: Self::State) {
                    match state {
                        #fetch_release_variants
                    }
                }

                #[allow(unused_variables, unused_mut)]
                fn for_each_borrow(mut f: impl ::core::ops::FnMut(::core::any::TypeId, bool)) {
                    #fetch_for_each_borrow
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
                *l = Lifetime::new("'static", Span::mixed_site());
            }
        }
    }

    let mut ty = ty.clone();
    syn::visit_mut::visit_type_mut(&mut Visitor { replace: lifetime }, &mut ty);
    quote! { #ty }
}
