use hecs::{Component, Query};

struct A(i32);
impl Component for A {}

struct B(bool);
impl Component for B {}

#[derive(Query)]
enum Foo<'a> {
    Foo(&'a A),
}

#[derive(Query)]
enum Bar<'a> {
    Bar { bar: &'a B },
}

#[derive(Query)]
enum All<'a> {
    Foo(&'a A),
    Bar { bar: &'a B },
    Baz,
}

fn main() {}
