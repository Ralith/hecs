extern crate proc_macro;

mod bundle;
mod bundle_clone;
mod query;

pub(crate) mod common;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

/// Implement `Bundle` for a struct
///
/// Bundles can be passed directly to `World::spawn` and `World::insert`, and obtained from
/// `World::remove`. Can be convenient when combined with other derives like `serde::Deserialize`.
///
/// # Example
/// ```
/// # use hecs::*;
/// #[derive(Debug, PartialEq)]
/// struct X(i32);
/// impl Component for X {}
///
/// #[derive(Debug, PartialEq)]
/// struct Y(char);
/// impl Component for Y {}
///
/// #[derive(Bundle)]
/// struct Foo {
///     x: X,
///     y: Y,
/// }
///
/// let mut world = World::new();
/// let e = world.spawn(Foo { x: X(42), y: Y('a') });
/// assert_eq!(*world.get::<&X>(e).unwrap(), X(42));
/// ```
#[proc_macro_derive(Bundle)]
pub fn derive_bundle(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match bundle::derive(input) {
        Ok(ts) => ts,
        Err(e) => e.to_compile_error(),
    }
    .into()
}

/// Implement `DynamicBundleClone` for a struct.
///
/// This is an extension macro for bundles which allow them to be cloned, and
/// subsequently used in `EntityBuilderClone::add_bundle`.
///
/// Requires that all fields of the struct implement [`Clone`].
///
/// The trait Bundle must also be implemented to be able to be used in
/// entity builder.
#[proc_macro_derive(DynamicBundleClone)]
pub fn derive_dynamic_bundle_clone(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match bundle_clone::derive(input) {
        Ok(ts) => ts,
        Err(e) => e.to_compile_error(),
    }
    .into()
}

/// Implement `Query` for a struct or enum.
///
/// Queries can be passed to the type parameter of `World::query`. They must have exactly
/// one lifetime parameter, and all of their fields must be queries (e.g. references) using that
/// lifetime.
///
/// For enum queries, the result will always be the first variant that matches the entity.
/// Unit variants and variants without any fields will always match an entity.
///
/// # Example
/// ```
/// # use hecs::*;
/// #[derive(Debug, PartialEq)]
/// struct X(i32);
/// impl Component for X {}
///
/// #[derive(Debug, PartialEq)]
/// struct Y(bool);
/// impl Component for Y {}
///
/// #[derive(Query, Debug, PartialEq)]
/// struct Foo<'a> {
///     x: &'a X,
///     y: &'a mut Y,
/// }
///
/// let mut world = World::new();
/// let e = world.spawn((X(42), Y(false)));
/// assert_eq!(
///     world.query_one_mut::<Foo>(e).unwrap(),
///     Foo {
///         x: &X(42),
///         y: &mut Y(false)
///     }
/// );
/// ```
#[proc_macro_derive(Query)]
pub fn derive_query(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match query::derive(input) {
        Ok(ts) => ts,
        Err(e) => e.to_compile_error(),
    }
    .into()
}

/// Implement `Component` for some type.
///
/// Convenience short-hand for `impl Component for T {}`.
///
/// Generic type parameters automatically receive the `Send + Sync + 'static` bounds required by the
/// trait's supertraits. When this is inappropriate, use a manual implementation instead.
#[proc_macro_derive(Component)]
pub fn derive_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ident = input.ident;
    let mut generics = input.generics;
    for param in generics.type_params_mut() {
        param.bounds.push(syn::parse_quote!(::core::marker::Send));
        param.bounds.push(syn::parse_quote!(::core::marker::Sync));
        param.bounds.push(syn::parse_quote!('static));
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    quote! { impl #impl_generics ::hecs::Component for #ident #ty_generics #where_clause {} }.into()
}
