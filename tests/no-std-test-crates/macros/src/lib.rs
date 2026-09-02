//! This is a test crate to ensure that the macro crate and the macro-expanded code can work in a `no_std` environment.

#![no_std]
#![allow(clippy::disallowed_names)]

use hecs::{Bundle, Component, Query};

#[derive(Component)]
pub struct Comp;

#[derive(Component)]
pub struct Generic<T>(T);

#[derive(Bundle)]
pub struct Foo {
    pub foo: Comp,
}

#[derive(Query)]
pub struct Quux<'a> {
    pub foo: &'a Comp,
}
