#![no_implicit_prelude]

#[derive(::hecs::Bundle)]
struct Foo {
    foo: (),
}

#[derive(::hecs::Query)]
struct Quux<'a> {
    foo: &'a (),
}

fn main() {}
