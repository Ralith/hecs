use hecs::Query;

#[derive(Query)]
struct Foo<'a> {
    foo: &'a i32,
    bar: Bar<'a>,
}

#[derive(Query)]
struct Bar<'a> {
    baz: &'a mut bool,
}

fn main() {}
