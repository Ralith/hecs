//! This is a test crate to ensure that the macro crate and the macro-expanded code can work in a `no_std` environment.

#![no_std]
#![allow(clippy::disallowed_names)]

use hecs::{Bundle, Component, Query};

#[derive(Bundle)]
pub struct Foo {
    pub foo: i32,
}

#[derive(Query)]
pub struct Baz<'a> {
    pub a: &'a usize,
}

#[derive(Query)]
pub struct GenericBaz<'a, T: Component = usize> {
    pub a: &'a T,
}

#[derive(Query)]
pub enum GenericEnum<'a, T: Component = usize> {
    Val(&'a T),
}

#[derive(Query)]
pub struct Quux<'a> {
    pub foo: &'a Foo,
}
