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
    non_lifetime_params: Vec<Ident>,
}

impl PreparedGenerics {
    fn new(generics: Generics, ident: &Ident) -> Result<Self> {
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

        let item_lifetime = if lifetime.ident == "q" {
            Lifetime::new("'__hecs_q", Span::mixed_site())
        } else {
            Lifetime::new("'q", Span::mixed_site())
        };
        let mut item_generics = generics.clone();
        replace_lifetime_in_generics(&mut item_generics, &lifetime, &item_lifetime);

        let non_lifetime_params = generics
            .params
            .iter()
            .filter_map(|p| match p {
                syn::GenericParam::Type(t) => Some(t.ident.clone()),
                syn::GenericParam::Const(c) => Some(c.ident.clone()),
                syn::GenericParam::Lifetime(_) => None,
            })
            .collect();

        Ok(Self {
            lifetime,
            item_lifetime,
            generics,
            item_generics,
            non_lifetime_params,
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
    let (impl_generics, ty_generics, where_clause) = prepared.generics.split_for_impl();
    let (_, item_ty_generics, _) = prepared.item_generics.split_for_impl();

    let (fields, raw_tys): (Vec<_>, Vec<_>) = match data.fields {
        syn::Fields::Named(ref fields) => fields
            .named
            .iter()
            .map(|f| (syn::Member::Named(f.ident.clone().unwrap()), &f.ty))
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
                    &f.ty,
                )
            })
            .unzip(),
        syn::Fields::Unit => (Vec::new(), Vec::new()),
    };

    let queries = raw_tys
        .iter()
        .map(|ty| query_ty(lifetime, ty))
        .collect::<Vec<_>>();

    let mut fetch_params = Vec::new();
    let mut state_params = Vec::new();
    let mut query_fetch_args = Vec::new();
    let mut fetch_state_args = Vec::new();
    let mut field_fetch_tys = Vec::with_capacity(fields.len());
    let mut field_state_tys = Vec::with_capacity(fields.len());

    for (raw_ty, query) in raw_tys.iter().zip(&queries) {
        if type_uses_any_ident(raw_ty, &prepared.non_lifetime_params) {
            let idx = fetch_params.len();
            let fetch_param = Ident::new(&format!("__F{idx}"), Span::mixed_site());
            let state_param = Ident::new(&format!("__S{idx}"), Span::mixed_site());
            query_fetch_args.push(quote! { <#query as ::hecs::Query>::Fetch });
            fetch_state_args.push(quote! { <#fetch_param as ::hecs::Fetch>::State });
            field_fetch_tys.push(quote! { #fetch_param });
            field_state_tys.push(quote! { #state_param });
            fetch_params.push(fetch_param);
            state_params.push(state_param);
        } else {
            let concrete_fetch = quote! { <#query as ::hecs::Query>::Fetch };
            field_state_tys.push(quote! { <#concrete_fetch as ::hecs::Fetch>::State });
            field_fetch_tys.push(concrete_fetch);
        }
    }

    let tuple_indices = (0..fields.len())
        .map(|i| syn::Index {
            index: i as u32,
            span: Span::mixed_site(),
        })
        .collect::<Vec<_>>();

    let fetch_ty_params = if fetch_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#fetch_params),*> }
    };
    let fetch_clone_generics = if fetch_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#fetch_params: ::core::clone::Clone),*> }
    };
    let fetch_impl_generics = if fetch_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#fetch_params: ::hecs::Fetch),*> }
    };
    let state_ty_params = if state_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#state_params),*> }
    };
    let state_copy_generics = if state_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#state_params: ::core::marker::Copy),*> }
    };
    let query_fetch_arg_list = if query_fetch_args.is_empty() {
        quote! {}
    } else {
        quote! { <#(#query_fetch_args),*> }
    };
    let fetch_state_arg_list = if fetch_state_args.is_empty() {
        quote! {}
    } else {
        quote! { <#(#fetch_state_args),*> }
    };

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
            #vis struct #fetch_ident #fetch_ty_params (
                #(#field_fetch_tys,)*
            );

            impl #fetch_clone_generics ::core::clone::Clone for #fetch_ident #fetch_ty_params {
                fn clone(&self) -> Self {
                    Self(
                        #(::core::clone::Clone::clone(&self.#tuple_indices),)*
                    )
                }
            }

            impl #impl_generics ::hecs::Query for #ident #ty_generics #where_clause {
                type Item<#item_lifetime> = #ident #item_ty_generics;

                type Fetch = #fetch_ident #query_fetch_arg_list;

                #[allow(unused_variables, clippy::redundant_field_names)]
                unsafe fn get<#item_lifetime>(meta: &[::hecs::EntityMeta], fetch: &Self::Fetch, n: usize) -> Self::Item<#item_lifetime> {
                    #(
                        let #intermediates: <#queries as ::hecs::Query>::Item<#item_lifetime> = <#queries as ::hecs::Query>::get(meta, &fetch.#tuple_indices, n);
                    )*
                    #ident {#(#fields: #intermediates,)*}
                }
            }

            #vis struct #state_ident #state_ty_params (
                #(#field_state_tys,)*
            );

            impl #state_copy_generics ::core::marker::Copy for #state_ident #state_ty_params {}

            impl #state_copy_generics ::core::clone::Clone for #state_ident #state_ty_params {
                fn clone(&self) -> Self {
                    *self
                }
            }

            unsafe impl #fetch_impl_generics ::hecs::Fetch for #fetch_ident #fetch_ty_params {
                type State = #state_ident #fetch_state_arg_list;

                fn dangling() -> Self {
                    Self(
                        #(<#field_fetch_tys as ::hecs::Fetch>::dangling(),)*
                    )
                }

                #[allow(unused_variables, unused_mut)]
                fn access(archetype: &::hecs::Archetype) -> ::core::option::Option<::hecs::Access> {
                    let mut access = ::hecs::Access::Iterate;
                    #(
                        access = ::core::cmp::max(access, <#field_fetch_tys as ::hecs::Fetch>::access(archetype)?);
                    )*
                    ::core::option::Option::Some(access)
                }

                #[allow(unused_variables)]
                fn borrow(archetype: &::hecs::Archetype, state: Self::State) {
                    #(<#field_fetch_tys as ::hecs::Fetch>::borrow(archetype, state.#tuple_indices);)*
                }

                #[allow(unused_variables)]
                fn prepare(archetype: &::hecs::Archetype) -> ::core::option::Option<Self::State> {
                    ::core::option::Option::Some(#state_ident(
                        #(<#field_fetch_tys as ::hecs::Fetch>::prepare(archetype)?,)*
                    ))
                }

                #[allow(unused_variables)]
                fn execute(archetype: &::hecs::Archetype, state: Self::State) -> Self {
                    Self(
                        #(<#field_fetch_tys as ::hecs::Fetch>::execute(archetype, state.#tuple_indices),)*
                    )
                }

                #[allow(unused_variables)]
                fn release(archetype: &::hecs::Archetype, state: Self::State) {
                    #(<#field_fetch_tys as ::hecs::Fetch>::release(archetype, state.#tuple_indices);)*
                }

                #[allow(unused_variables, unused_mut)]
                fn for_each_borrow(mut f: impl ::core::ops::FnMut(::core::any::TypeId, bool)) {
                    #(
                        <#field_fetch_tys as ::hecs::Fetch>::for_each_borrow(&mut f);
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
    let (impl_generics, ty_generics, where_clause) = prepared.generics.split_for_impl();
    let (_, item_ty_generics, _) = prepared.item_generics.split_for_impl();

    let mut dangling_constructor = None;
    let mut query_fetch_args = Vec::new();
    let mut fetch_params = Vec::new();
    let mut state_params = Vec::new();
    let mut fetch_state_args = Vec::new();
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
        let (fields, raw_tys): (Vec<_>, Vec<_>) = match variant.fields {
            syn::Fields::Named(ref fields) => fields
                .named
                .iter()
                .map(|f| (syn::Member::Named(f.ident.clone().unwrap()), &f.ty))
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
                        &f.ty,
                    )
                })
                .unzip(),
            syn::Fields::Unit => (Vec::new(), Vec::new()),
        };

        let ident = variant.ident.clone();

        if ident == "__HecsDanglingFetch__" {
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

        let queries = raw_tys
            .iter()
            .map(|ty| query_ty(lifetime, ty))
            .collect::<Vec<_>>();

        let mut variant_fetch_tys = Vec::with_capacity(queries.len());
        let mut variant_state_tys = Vec::with_capacity(queries.len());
        for (raw_ty, query) in raw_tys.iter().zip(&queries) {
            if type_uses_any_ident(raw_ty, &prepared.non_lifetime_params) {
                let idx = fetch_params.len();
                let fetch_param = Ident::new(&format!("__F{idx}"), Span::mixed_site());
                let state_param = Ident::new(&format!("__S{idx}"), Span::mixed_site());
                query_fetch_args.push(quote! { <#query as ::hecs::Query>::Fetch });
                fetch_state_args.push(quote! { <#fetch_param as ::hecs::Fetch>::State });
                variant_fetch_tys.push(quote! { #fetch_param });
                variant_state_tys.push(quote! { #state_param });
                fetch_params.push(fetch_param);
                state_params.push(state_param);
            } else {
                let concrete_fetch = quote! { <#query as ::hecs::Query>::Fetch };
                variant_state_tys.push(quote! { <#concrete_fetch as ::hecs::Fetch>::State });
                variant_fetch_tys.push(concrete_fetch);
            }
        }

        if dangling_constructor.is_none() && fields.is_empty() {
            dangling_constructor = Some(quote! {
                Self::#ident {}
            });
        }

        fetch_variants.extend(quote! {
            #ident {
                #(
                    #named_fields: #variant_fetch_tys,
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
                    #named_fields: #variant_state_tys,
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
                    if let ::core::option::Option::Some(new_access) = <#variant_fetch_tys as ::hecs::Fetch>::access(archetype) {
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
                    <#variant_fetch_tys as ::hecs::Fetch>::borrow(archetype, #named_fields);
                )*
            },
        });

        fetch_prepare_variants.extend(quote! {
            'block: {
                #(
                    let ::core::option::Option::Some(#named_fields) = <#variant_fetch_tys as ::hecs::Fetch>::prepare(archetype) else {
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
                        #named_fields: <#variant_fetch_tys as ::hecs::Fetch>::execute(archetype, #named_fields),
                    )*
                };
            },
        });

        fetch_release_variants.extend(quote! {
            Self::State::#ident { #(#named_fields),* } => {
                #(
                    <#variant_fetch_tys as ::hecs::Fetch>::release(archetype, #named_fields);
                )*
            },
        });

        fetch_for_each_borrow.extend(quote! {
            #(
                <#variant_fetch_tys as ::hecs::Fetch>::for_each_borrow(&mut f);
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
            Self::Fetch::__HecsDanglingFetch__ => ::core::panic!("Called get() with dangling fetch"),
        });
        quote! {
            Self::__HecsDanglingFetch__
        }
    };

    let fetch_ty_params = if fetch_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#fetch_params),*> }
    };
    let fetch_clone_generics = if fetch_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#fetch_params: ::core::clone::Clone),*> }
    };
    let fetch_impl_generics = if fetch_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#fetch_params: ::hecs::Fetch),*> }
    };
    let state_ty_params = if state_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#state_params),*> }
    };
    let state_copy_generics = if state_params.is_empty() {
        quote! {}
    } else {
        quote! { <#(#state_params: ::core::marker::Copy),*> }
    };
    let query_fetch_arg_list = if query_fetch_args.is_empty() {
        quote! {}
    } else {
        quote! { <#(#query_fetch_args),*> }
    };
    let fetch_state_arg_list = if fetch_state_args.is_empty() {
        quote! {}
    } else {
        quote! { <#(#fetch_state_args),*> }
    };

    let fetch_ident = Ident::new(&format!("{enum_ident}Fetch"), Span::mixed_site());
    let fetch = quote! {
        #vis enum #fetch_ident #fetch_ty_params {
            #fetch_variants
        }
    };

    let state_ident = Ident::new(&format!("{enum_ident}State"), Span::mixed_site());
    let state = quote! {
        #vis enum #state_ident #state_ty_params {
            #state_variants
        }
    };

    Ok(quote! {
        const _: () = {
            #fetch

            impl #fetch_clone_generics ::core::clone::Clone for #fetch_ident #fetch_ty_params {
                fn clone(&self) -> Self {
                    match self {
                        #fetch_clone_variants
                    }
                }
            }

            impl #impl_generics ::hecs::Query for #enum_ident #ty_generics #where_clause {
                type Item<#item_lifetime> = #enum_ident #item_ty_generics;

                type Fetch = #fetch_ident #query_fetch_arg_list;

                #[allow(unused_variables, clippy::redundant_field_names)]
                unsafe fn get<#item_lifetime>(meta: &[::hecs::EntityMeta], fetch: &Self::Fetch, n: usize) -> Self::Item<#item_lifetime> {
                    match fetch {
                        #query_get_variants
                    }
                }
            }

            #state

            impl #state_copy_generics ::core::marker::Copy for #state_ident #state_ty_params {}

            impl #state_copy_generics ::core::clone::Clone for #state_ident #state_ty_params {
                fn clone(&self) -> Self {
                    *self
                }
            }

            unsafe impl #fetch_impl_generics ::hecs::Fetch for #fetch_ident #fetch_ty_params {
                type State = #state_ident #fetch_state_arg_list;

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

struct IdentFinder<'a> {
    idents: &'a [Ident],
    found: bool,
}

impl syn::visit_mut::VisitMut for IdentFinder<'_> {
    fn visit_path_mut(&mut self, path: &mut syn::Path) {
        if !self.found && path.leading_colon.is_none() {
            if let Some(first) = path.segments.first() {
                if self.idents.contains(&first.ident) {
                    self.found = true;
                    return;
                }
            }
        }
        syn::visit_mut::visit_path_mut(self, path);
    }
}

fn type_uses_any_ident(ty: &Type, idents: &[Ident]) -> bool {
    if idents.is_empty() {
        return false;
    }
    let mut finder = IdentFinder {
        idents,
        found: false,
    };
    let mut ty = ty.clone();
    syn::visit_mut::visit_type_mut(&mut finder, &mut ty);
    finder.found
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
