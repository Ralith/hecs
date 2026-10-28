use hecs::{Bundle, Component, Query};

struct A(i32);
impl Component for A {}

struct B(bool);
impl Component for B {}

struct S(String);
impl Component for S {}

struct T(&'static str);
impl Component for T {}

#[derive(Bundle)]
struct Foo {
    foo: A,
}

#[derive(Bundle)]
struct Bar {
    foo: A,
    bar: S,
}

#[derive(Bundle)]
struct Baz {
    foo: A,
    bar: S,
    baz: T,
}

#[derive(Query)]
struct Quux<'a> {
    foo: &'a A,
    bar: &'a mut B,
}

fn main() {}
