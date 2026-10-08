use hecs::{Component, Query};

struct A(i32);
impl Component for A {}

struct B(bool);
impl Component for B {}

#[derive(Query)]
struct Foo<'a> {
    foo: &'a A,
    bar: Bar<'a>,
}

#[derive(Query)]
struct Bar<'a> {
    baz: &'a mut B,
}

#[derive(Query)]
enum Baz<'a> {
    Foo(Foo<'a>),
    Bar(Bar<'a>),
}

fn main() {}
