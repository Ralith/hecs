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

#[derive(::hecs::Query)]
enum Corge<'a> {
    Foo(&'a i32),
    Bar { bar: &'a bool },
    Baz,
}

#[derive(::hecs::Query)]
struct Grault<'a, T> {
    foo: &'a T,
}

#[derive(::hecs::Query)]
enum Garply<'a, T> {
    Foo(&'a T),
    Bar { bar: &'a bool },
}

fn main() {}
