use hecs::{Bundle, Query};

#[derive(Bundle, Query)]
union Foo {
    u8: u8,
}

fn main() {}
