#![no_implicit_prelude]

struct A(i32);
impl ::hecs::Component for A {}

struct B(bool);
impl ::hecs::Component for B {}

#[derive(::hecs::Bundle)]
struct Foo {
    foo: A,
}

#[derive(::hecs::Bundle)]
struct Bar<T> {
    foo: T,
}

#[derive(::hecs::Bundle)]
struct Baz;

#[derive(::hecs::Query)]
struct Quux<'a> {
    foo: &'a A,
}

#[derive(::hecs::Query)]
enum Corge<'a> {
    Foo(&'a A),
    Bar { bar: &'a B },
    Baz,
}

fn main() {}
