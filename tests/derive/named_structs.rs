use hecs::Bundle;

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

fn main() {}
