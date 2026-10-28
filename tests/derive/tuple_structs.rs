use hecs::{Bundle, Component, Query};

struct A(i32);
impl Component for A {}

struct S(String);
impl Component for S {}

struct T(&'static str);
impl Component for T {}

#[derive(Bundle)]
struct Foo(A);

#[derive(Bundle)]
struct Bar(A, S);

#[derive(Bundle)]
struct Baz(A, S, T);

#[derive(Query)]
struct Quux<'a>(&'a A);

fn main() {}
