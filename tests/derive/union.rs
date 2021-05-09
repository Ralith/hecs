use hecs::{Bundle, Query};

#[derive(Query)]
union Foo {
    u8: u8,
}

#[derive(Bundle)]
union Bar {
    u8: u8,
}

fn main() {}
