use hecs::{Component, Query};

struct A(i32);
impl Component for A {}

struct B(bool);
impl Component for B {}

#[derive(Query)]
struct Foo<'a> {
    foo: &'a A,
    bar: &'static mut B,
}

#[derive(Query)]
enum Bar<'a> {
    Foo(&'a A),
    Bar(&'static mut B),
}

fn main() {}
