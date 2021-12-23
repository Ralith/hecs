// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

extern crate proc_macro;

mod bundle;
mod bundle_clone;
mod query;

pub(crate) mod common;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

/// Implement `Bundle` for a struct
///
/// Bundles can be passed directly to `World::spawn` and `World::insert`, and obtained from
/// `World::remove`. Can be convenient when combined with other derives like `serde::Deserialize`.
///
/// # Example
/// ```ignore
/// #[derive(Bundle)]
/// struct Foo {
///     x: i32,
///     y: char,
/// }
///
/// let mut world = World::new();
/// let e = world.spawn(Foo { x: 42, y: 'a' });
/// assert_eq!(*world.get::<i32>(e).unwrap(), 42);
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

/// Implement `Query` for a struct
///
/// Queries structs can be passed to the type parameter of `World::query`. They must have exactly
/// one lifetime parameter, and all of their fields must be queries (e.g. references) using that
/// lifetime.
///
/// # Example
/// ```ignore
/// #[derive(Query, Debug, PartialEq)]
/// struct Foo<'a> {
///     x: &'a i32,
///     y: &'a mut bool,
/// }
///
/// let mut world = World::new();
/// let e = world.spawn((42, false));
/// assert_eq!(
///     world.query_one_mut::<Foo>(e).unwrap(),
///     Foo {
///         x: &42,
///         y: &mut false
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
