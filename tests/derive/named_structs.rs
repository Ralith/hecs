use hecs::{Bundle, Query};

#[derive(Bundle)]
struct Foo {
    foo: i32,
}

#[derive(Bundle)]
struct Bar {
    foo: i32,
    bar: String,
}

#[derive(Bundle)]
struct Baz {
    foo: i32,
    bar: String,
    baz: &'static str,
}

#[derive(Query)]
struct Quux<'a> {
    foo: &'a i32,
    bar: &'a mut bool,
}

fn main() {}
