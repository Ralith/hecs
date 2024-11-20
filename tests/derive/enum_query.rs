use hecs::Query;

#[derive(Query)]
enum Foo<'a> {
    Foo(&'a i32)
}

#[derive(Query)]
enum Bar<'a> {
    Bar {
        bar: &'a bool
    },
}

#[derive(Query)]
enum All<'a> {
    Foo(&'a i32),
    Bar {
        bar: &'a bool
    },
    Baz
}

fn main() {}
