use proc_macro2::Span;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    DataEnum, DataStruct, DeriveInput, Error, Generics, Ident, Lifetime, Result, Type, Visibility,
};

pub fn derive(input: DeriveInput) -> Result<TokenStream2> {
    let ident = input.ident;

    match input.data {
        syn::Data::Struct(_) | syn::Data::Enum(_) => {}
        _ => {
            return Err(Error::new_spanned(
                ident,
                "derive(Query) may only be applied to structs and enums",
            ))
        }
    }

    let vis = input.vis;
    let generics = PreparedGenerics::new(input.generics, &ident)?;

    match input.data {
        syn::Data::Struct(data_struct) => derive_struct(ident, vis, data_struct, generics),
        syn::Data::Enum(data_enum) => derive_enum(ident, vis, data_enum, generics),
        _ => unreachable!(),
    }
}

struct PreparedGenerics {
    lifetime: Lifetime,
    item_lifetime: Lifetime,
    generics: Generics,
    item_generics: Generics,
    fetch_generics: Generics,
    type_params: Vec<Ident>,
}

impl PreparedGenerics {
    fn new(mut generics: Generics, ident: &Ident) -> Result<Self> {
        let mut lifetimes = generics.lifetimes();
        let lifetime = match lifetimes.next() {
            Some(x) => x.lifetime.clone(),
            None => {
                return Err(Error::new_spanned(
                    generics,
                    "must have exactly one lifetime parameter",
                ))
            }
        };
        if lifetimes.next().is_some() {
            return Err(Error::new_spanned(
                ident,
                "must have exactly one lifetime parameter",
            ));
        }

        for tp in generics.type_params_mut() {
            tp.bounds.push(syn::parse_quote!(::hecs::Component));
        }
        let type_params = generics
            .type_params()
            .map(|tp| tp.ident.clone())
            .collect::<Vec<_>>();

        let item_lifetime = if lifetime.ident == "q" {
            Lifetime::new("'__hecs_q", Span::mixed_site())
        } else {
            Lifetime::new("'q", Span::mixed_site())
        };
        let mut item_generics = generics.clone();
        replace_lifetime_in_generics(&mut item_generics, &lifetime, &item_lifetime);

        let static_lifetime = Lifetime::new("'static", Span::mixed_site());
        let mut fetch_generics = generics.clone();
        fetch_generics.params = fetch_generics
            .params
            .into_iter()
            .filter(|param| !matches!(param, syn::GenericParam::Lifetime(_)))
            .collect();
        replace_lifetime_in_generics(&mut fetch_generics, &lifetime, &static_lifetime);

        Ok(Self {
            lifetime,
            item_lifetime,
            generics,
            item_generics,
            fetch_generics,
            type_params,
        })
    }
}

fn derive_struct(
    ident: Ident,
    vis: Visibility,
    data: DataStruct,
    prepared: PreparedGenerics,
) -> Result<TokenStream2> {
    let lifetime = &prepared.lifetime;
    let item_lifetime = &prepared.item_lifetime;
    let type_params = &prepared.type_params;
    let (impl_generics, ty_generics, where_clause) = prepared.generics.split_for_impl();
    let (_, item_ty_generics, _) = prepared.item_generics.split_for_impl();
    let (fetch_impl_generics, fetch_ty_generics, fetch_where_clause) =
        prepared.fetch_generics.split_for_impl();

    let (fields, queries): (Vec<_>, Vec<_>) = match data.fields {
        syn::Fields::Named(ref fields) => fields
            .named
            .iter()
            .map(|f| {
                (
                    syn::Member::Named(f.ident.clone().unwrap()),
                    query_ty(lifetime, &f.ty),
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
                    query_ty(lifetime, &f.ty),
                )
            })
            .unzip(),
        syn::Fields::Unit => (Vec::new(), Vec::new()),
    };
    let fetches = queries
        .iter()
        .map(|ty| quote! { <#ty as ::hecs::Query>::Fetch })
        .collect::<Vec<_>>();
    let tuple_indices = (0..fields.len())
        .map(|i| syn::Index {
            index: i as u32,
            span: Span::mixed_site(),
        })
        .collect::<Vec<_>>();

    let fetch_ident = Ident::new(&format!("{ident}Fetch"), Span::mixed_site());
    let state_ident = Ident::new(&format!("{ident}State"), Span::mixed_site());

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
            #vis struct #fetch_ident #fetch_impl_generics (
                #(#fetches,)*
                ::core::marker::PhantomData<fn() -> (#(#type_params,)*)>,
            ) #fetch_where_clause;

            impl #fetch_impl_generics ::core::clone::Clone for #fetch_ident #fetch_ty_generics #fetch_where_clause {
                fn clone(&self) -> Self {
                    Self(
                        #(::core::clone::Clone::clone(&self.#tuple_indices),)*
                        ::core::marker::PhantomData,
                    )
                }
            }

            impl #impl_generics ::hecs::Query for #ident #ty_generics #where_clause {
                type Item<#item_lifetime> = #ident #item_ty_generics;

                type Fetch = #fetch_ident #fetch_ty_generics;

                #[allow(unused_variables, clippy::redundant_field_names)]
                unsafe fn get<#item_lifetime>(meta: &[::hecs::EntityMeta], fetch: &Self::Fetch, n: usize) -> Self::Item<#item_lifetime> {
                    #(
                        let #intermediates: <#queries as ::hecs::Query>::Item<#item_lifetime> = <#queries as ::hecs::Query>::get(meta, &fetch.#tuple_indices, n);
                    )*
                    #ident {#(#fields: #intermediates,)*}
                }
            }

            #vis struct #state_ident #fetch_impl_generics (
                #(<#fetches as ::hecs::Fetch>::State,)*
                ::core::marker::PhantomData<fn() -> (#(#type_params,)*)>,
            ) #fetch_where_clause;

            impl #fetch_impl_generics ::core::marker::Copy for #state_ident #fetch_ty_generics #fetch_where_clause {}

            impl #fetch_impl_generics ::core::clone::Clone for #state_ident #fetch_ty_generics #fetch_where_clause {
                fn clone(&self) -> Self {
                    *self
                }
            }

            unsafe impl #fetch_impl_generics ::hecs::Fetch for #fetch_ident #fetch_ty_generics #fetch_where_clause {
                type State = #state_ident #fetch_ty_generics;

                fn dangling() -> Self {
                    Self(
                        #(#fetches::dangling(),)*
                        ::core::marker::PhantomData,
                    )
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
                    #(#fetches::borrow(archetype, state.#tuple_indices);)*
                }

                #[allow(unused_variables)]
                fn prepare(archetype: &::hecs::Archetype) -> ::core::option::Option<Self::State> {
                    ::core::option::Option::Some(#state_ident(
                        #(#fetches::prepare(archetype)?,)*
                        ::core::marker::PhantomData,
                    ))
                }

                #[allow(unused_variables)]
                fn execute(archetype: &::hecs::Archetype, state: Self::State) -> Self {
                    Self(
                        #(#fetches::execute(archetype, state.#tuple_indices),)*
                        ::core::marker::PhantomData,
                    )
                }

                #[allow(unused_variables)]
                fn release(archetype: &::hecs::Archetype, state: Self::State) {
                    #(#fetches::release(archetype, state.#tuple_indices);)*
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

fn derive_enum(
    enum_ident: Ident,
    vis: Visibility,
    data: DataEnum,
    prepared: PreparedGenerics,
) -> Result<TokenStream2> {
    let lifetime = &prepared.lifetime;
    let item_lifetime = &prepared.item_lifetime;
    let type_params = &prepared.type_params;
    let (impl_generics, ty_generics, where_clause) = prepared.generics.split_for_impl();
    let (_, item_ty_generics, _) = prepared.item_generics.split_for_impl();
    let (fetch_impl_generics, fetch_ty_generics, fetch_where_clause) =
        prepared.fetch_generics.split_for_impl();

    let mut dangling_constructor = None;
    let mut fetch_variants = TokenStream2::new();
    let mut fetch_clone_variants = TokenStream2::new();
    let mut state_variants = TokenStream2::new();
    let mut query_get_variants = TokenStream2::new();
    let mut fetch_access_variants = TokenStream2::new();
    let mut fetch_borrow_variants = TokenStream2::new();
    let mut fetch_prepare_variants = TokenStream2::new();
    let mut fetch_execute_variants = TokenStream2::new();
    let mut fetch_release_variants = TokenStream2::new();
    let mut fetch_for_each_borrow = TokenStream2::new();

    for variant in &data.variants {
        let (fields, queries): (Vec<_>, Vec<_>) = match variant.fields {
            syn::Fields::Named(ref fields) => fields
                .named
                .iter()
                .map(|f| {
                    (
                        syn::Member::Named(f.ident.clone().unwrap()),
                        query_ty(lifetime, &f.ty),
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
                        query_ty(lifetime, &f.ty),
                    )
                })
                .unzip(),
            syn::Fields::Unit => (Vec::new(), Vec::new()),
        };

        let ident = variant.ident.clone();

        if ident == "__HecsDanglingFetch__" || ident == "__HecsPhantom__" {
            return Err(Error::new_spanned(
                ident,
                "derive(Query) reserves this identifier for internal use",
            ));
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

        fetch_clone_variants.extend(quote! {
            Self::#ident { #(#named_fields),* } => Self::#ident {
                #(
                    #named_fields: ::core::clone::Clone::clone(#named_fields),
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

        query_get_variants.extend(quote! {
            Self::Fetch::#ident { #(#named_fields),* } => {
                #(
                    let #named_fields: <#queries as ::hecs::Query>::Item<#item_lifetime> = <#queries as ::hecs::Query>::get(meta, #named_fields, n);
                )*
                #enum_ident::#ident { #( #fields: #named_fields,)* }
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

        fetch_release_variants.extend(quote! {
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
        fetch_clone_variants.extend(quote! {
            Self::__HecsDanglingFetch__ => Self::__HecsDanglingFetch__,
        });
        query_get_variants.extend(quote! {
            Self::Fetch::__HecsDanglingFetch__ => panic!("Called get() with dangling fetch"),
        });
        quote! {
            Self::__HecsDanglingFetch__
        }
    };

    fetch_variants.extend(quote! {
        __HecsPhantom__(::core::convert::Infallible, ::core::marker::PhantomData<fn() -> (#(#type_params,)*)>),
    });
    fetch_clone_variants.extend(quote! {
        Self::__HecsPhantom__(infallible, _) => match *infallible {},
    });
    query_get_variants.extend(quote! {
        Self::Fetch::__HecsPhantom__(infallible, _) => match *infallible {},
    });

    state_variants.extend(quote! {
        __HecsPhantom__(::core::convert::Infallible, ::core::marker::PhantomData<fn() -> (#(#type_params,)*)>),
    });
    fetch_borrow_variants.extend(quote! {
        Self::State::__HecsPhantom__(infallible, _) => match infallible {},
    });
    fetch_execute_variants.extend(quote! {
        Self::State::__HecsPhantom__(infallible, _) => match infallible {},
    });
    fetch_release_variants.extend(quote! {
        Self::State::__HecsPhantom__(infallible, _) => match infallible {},
    });

    let fetch_ident = Ident::new(&format!("{enum_ident}Fetch"), Span::mixed_site());
    let fetch = quote! {
        #vis enum #fetch_ident #fetch_impl_generics #fetch_where_clause {
            #fetch_variants
        }
    };

    let state_ident = Ident::new(&format!("{enum_ident}State"), Span::mixed_site());
    let state = quote! {
        #vis enum #state_ident #fetch_impl_generics #fetch_where_clause {
            #state_variants
        }
    };

    Ok(quote! {
        const _: () = {
            #fetch

            impl #fetch_impl_generics ::core::clone::Clone for #fetch_ident #fetch_ty_generics #fetch_where_clause {
                fn clone(&self) -> Self {
                    match self {
                        #fetch_clone_variants
                    }
                }
            }

            impl #impl_generics ::hecs::Query for #enum_ident #ty_generics #where_clause {
                type Item<#item_lifetime> = #enum_ident #item_ty_generics;

                type Fetch = #fetch_ident #fetch_ty_generics;

                #[allow(unused_variables, clippy::redundant_field_names)]
                unsafe fn get<#item_lifetime>(meta: &[::hecs::EntityMeta], fetch: &Self::Fetch, n: usize) -> Self::Item<#item_lifetime> {
                    match fetch {
                        #query_get_variants
                    }
                }
            }

            #state

            impl #fetch_impl_generics ::core::marker::Copy for #state_ident #fetch_ty_generics #fetch_where_clause {}

            impl #fetch_impl_generics ::core::clone::Clone for #state_ident #fetch_ty_generics #fetch_where_clause {
                fn clone(&self) -> Self {
                    *self
                }
            }

            unsafe impl #fetch_impl_generics ::hecs::Fetch for #fetch_ident #fetch_ty_generics #fetch_where_clause {
                type State = #state_ident #fetch_ty_generics;

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

struct LifetimeReplacer<'a> {
    from: &'a Lifetime,
    to: &'a Lifetime,
}

impl syn::visit_mut::VisitMut for LifetimeReplacer<'_> {
    fn visit_lifetime_mut(&mut self, l: &mut Lifetime) {
        if l == self.from {
            *l = self.to.clone();
        }
    }
}

fn replace_lifetime_in_generics(generics: &mut Generics, from: &Lifetime, to: &Lifetime) {
    syn::visit_mut::visit_generics_mut(&mut LifetimeReplacer { from, to }, generics);
}

fn query_ty(lifetime: &Lifetime, ty: &Type) -> TokenStream2 {
    let static_lifetime = Lifetime::new("'static", Span::mixed_site());
    let mut ty = ty.clone();
    syn::visit_mut::visit_type_mut(
        &mut LifetimeReplacer {
            from: lifetime,
            to: &static_lifetime,
        },
        &mut ty,
    );
    quote! { #ty }
}
