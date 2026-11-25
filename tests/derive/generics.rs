use hecs::{Bundle, Component, Query};

#[derive(Bundle)]
struct Foo<T> {
    foo: T,
}

#[derive(Bundle)]
struct Bar<T> {
    foo: i32,
    bar: T,
}

#[derive(Bundle)]
struct Baz<T, U, V> {
    foo: T,
    bar: U,
    baz: V,
}

pub struct NonClone;

#[derive(Query)]
struct QueryFoo<'a, T> {
    foo: &'a T,
}

#[derive(Query)]
struct QueryBar<'a, T> {
    foo: &'a i32,
    bar: &'a mut T,
}

#[derive(Query)]
struct QueryTuple<'a, T, U, V>(&'a T, &'a mut U, Option<&'a V>);

#[derive(Query)]
pub struct DrawSpriteQuery<'q, T: Component = NonClone> {
    pub entity_data: &'q T,
}

#[derive(Query)]
struct ArrayQuery<'a, T, const N: usize = 4>
where
    T: 'a,
{
    arr: &'a [T; N],
}

#[derive(Query)]
struct NestedGenericQuery<'a, T> {
    inner: QueryFoo<'a, T>,
}

#[derive(Query)]
enum GenericEnumQuery<'a, T, U = NonClone> {
    First(&'a T),
    Second { value: &'a mut U },
}

#[derive(Query)]
enum GenericEnumWithUnit<'a, T> {
    Present(&'a T),
    Empty,
}

fn main() {}
