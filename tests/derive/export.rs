mod inner {
    use hecs::{Bundle, Query};

    #[derive(Bundle)]
    pub struct Foo;

    #[derive(Query)]
    pub struct Bar<'a> {
        foo: &'a i32,
    }

    #[derive(Query)]
    pub struct GenericBar<'a, T = i32> {
        foo: &'a T,
    }

    #[derive(Query)]
    pub enum GenericBaz<'a, T = i32> {
        Foo(&'a T),
    }
}

type Foo = inner::Foo;
type Bar<'a> = inner::Bar<'a>;
type GenericBar<'a, T = i32> = inner::GenericBar<'a, T>;
type GenericBaz<'a, T = i32> = inner::GenericBaz<'a, T>;

fn main() {}
