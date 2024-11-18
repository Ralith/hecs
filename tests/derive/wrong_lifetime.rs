use hecs::Query;

#[derive(Query)]
struct Foo<'a> {
    foo: &'a i32,
    bar: &'static mut bool,
}

#[derive(Query)]
enum Bar<'a> {
    Foo(&'a i32),
    Bar(&'static mut bool)
}

fn main() {}
