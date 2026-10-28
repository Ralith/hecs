use hecs::{Bundle, Component};

struct A(i32);
impl Component for A {}

#[derive(Bundle)]
struct Foo<T> {
    foo: T,
}

#[derive(Bundle)]
struct Bar<T> {
    foo: A,
    bar: T,
}

#[derive(Bundle)]
struct Baz<T, U, V> {
    foo: T,
    bar: U,
    baz: V,
}

fn main() {}
