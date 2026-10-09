# Grove compatibility patch

This directory vendors [`block` 0.1.6](http://github.com/SSheldon/rust-block),
licensed MIT by Steven Sheldon, because it is the newest crates.io release and
is still required by Grove's GPUI dependency graph.

The source changes:

- Declare `_NSConcreteStackBlock` as `c_void` and store its address as
  `*const c_void`. The symbol is opaque runtime storage; Rust must not model it
  as the crate's uninhabited `Class` enum.
- Spell every previously implicit foreign ABI as `extern "C"`, preserving
  Rust's historical default while avoiding `missing_abi` deprecation warnings.

These changes remove the future-incompatibility and deprecation warnings
without changing the Objective-C Blocks ABI.

Remove this patch when Grove's dependency graph no longer includes `block`
0.1.6 or when a compatible upstream release contains the same correction.
