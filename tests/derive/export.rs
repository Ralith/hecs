mod inner {
    use hecs::{Bundle, Component, Query};

    struct Private(i32);

    #[derive(Bundle)]
    pub struct Foo;

    #[derive(Query)]
    pub struct Bar<'a> {
        foo: &'a i32,
    }

    #[derive(Query)]
    pub struct GenericBar<'a, T: Component = i32> {
        foo: &'a T,
        priv_field: &'a Private,
    }

    #[derive(Query)]
    pub enum GenericBaz<'a, T: Component = i32> {
        Foo(&'a T),
    }

    #[derive(Query)]
    pub struct GenericQueryParam<'a, Q: for<'q> Query<Item<'q> = Q>> {
        foo: &'a i32,
        inner: Q,
    }
}

type Foo = inner::Foo;
type Bar<'a> = inner::Bar<'a>;
type GenericBar<'a, T = i32> = inner::GenericBar<'a, T>;
type GenericBaz<'a, T = i32> = inner::GenericBaz<'a, T>;
type GenericQueryParam<'a, Q> = inner::GenericQueryParam<'a, Q>;

fn main() {}
