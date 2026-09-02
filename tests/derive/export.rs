mod inner {
    use hecs::{Bundle, Component, Query};

    struct A(i32);
    impl Component for A {}

    #[derive(Bundle)]
    pub struct Foo;

    #[derive(Query)]
    pub struct Bar<'a> {
        foo: &'a A,
    }
}

type Foo = inner::Foo;
type Bar<'a> = inner::Bar<'a>;

fn main() {}
