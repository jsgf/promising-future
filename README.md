# Promising Future

[![Build Status](https://travis-ci.org/jsgf/promising-future.svg?branch=master)](https://travis-ci.org/jsgf/promising-future)
[![Crates.io](https://img.shields.io/crates/v/promising-future.svg)]()
[![Coverage Status](https://coveralls.io/repos/github/jsgf/promising-future/badge.svg?branch=master)](https://coveralls.io/github/jsgf/promising-future?branch=master)

This crate implements `Promise`s and `Future`s, where a `Future`
represents an unknown value, and a `Promise` determines what that
value is.

It also provides `FutureStream` which allows multiple `Future`s to be
waited on, yielding values as they become available.

While this could be seen as a "yet another" crate, the API both
simpler and more general than other implementations of Futures, and
more easily composable.

Documentation is available [here](https://jsgf.github.io/promising-future/doc/promising_future/index.html).

## License

Licensed under either of

 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
