use hecs::Bundle;

#[derive(Bundle)]
struct Foo<T> {
    foo: T,
}

#[derive(Bundle)]
struct Bar<T> {
    foo: i32,
    bar: T,
}

#[derive(Bundle)]
struct Baz<T, U, V> {
    foo: T,
    bar: U,
    baz: V,
}

fn main() {}
