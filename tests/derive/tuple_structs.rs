use hecs::Bundle;

#[derive(Bundle)]
struct Foo(i32);

#[derive(Bundle)]
struct Bar(i32, String);

#[derive(Bundle)]
struct Baz(i32, String, &'static str);

fn main() {}
