# hecs

[![Documentation](https://docs.rs/hecs/badge.svg)](https://docs.rs/hecs/)
[![Crates.io](https://img.shields.io/crates/v/hecs.svg)](https://crates.io/crates/hecs)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)

hecs provides a high-performance, minimalist entity-component-system (ECS)
world. It is a library, not a framework. In place of an explicit "System"
abstraction, a `World`'s entities are easily queried from regular code. Organize
your application however you like!

### Other Libraries

hecs would not exist if not for the great work done by others to introduce and
develop the ECS paradigm in the Rust ecosystem. In particular:

- [specs] played a key role in popularizing ECS in Rust
- [legion] reduced boilerplate and improved cache locality with sparse
  components

hecs builds on these successes by focusing on further simplification, boiling
the paradigm down to a minimal, light-weight and ergonomic core, without
compromising on performance or flexibility.

### Disclaimer

This is not an official Google product (experimental or otherwise), it is just
code that happens to be owned by Google.

[specs]: https://github.com/amethyst/specs
[legion]: https://github.com/TomGillen/legion
