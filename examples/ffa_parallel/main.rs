#[cfg(feature = "parallel-iterators")]
mod ffa;

#[cfg(feature = "parallel-iterators")]
fn main() {
    ffa::main();
}

#[cfg(not(feature = "parallel-iterators"))]
fn main() {}
