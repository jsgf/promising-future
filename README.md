Promising Future
================

[![Build Status](https://travis-ci.org/jsgf/promising-future.svg?branch=master)](https://travis-ci.org/jsgf/promising-future)

This crate implements `Promise`s and `Future`s, where a `Future`
represents an unknown value, and a `Promise` determines what that
value is.

It also provides `FutureStream` which allows multiple `Future`s to be
waited on, yielding values as they become available.

While this could be seen as a "yet another" crate, the API both
simpler and more general than other implementations of Futures, and
more easily composable.

Documentation is available [here](https://jsgf.github.io/promising-future/doc/promising_future/index.html).
