#![no_implicit_prelude]

#[derive(::hecs::Bundle)]
struct Foo {
    foo: (),
}

#[derive(::hecs::Bundle)]
struct Bar<T> {
    foo: T,
}

#[derive(::hecs::Bundle)]
struct Baz;

#[derive(::hecs::Query)]
struct Quux<'a> {
    foo: &'a (),
}

fn main() {}
