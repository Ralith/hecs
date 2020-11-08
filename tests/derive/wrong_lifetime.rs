use hecs::Query;

#[derive(Query)]
struct Foo<'a> {
    foo: &'a i32,
    bar: &'static mut bool,
}

fn main() {}
