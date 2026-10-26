# Local rbx_dom_weak fork

Base: exact crates.io rbx_dom_weak 4.2.0 package, checksum
63614f84aae649ff71ef6e693d4fc9e2184c6ef55852d48a29685ebd0470114f.
Package VCS revision: f96e7383aea61730993aabb2c8e7192dd3e19b21,
path rbx_dom_weak in https://github.com/rojo-rbx/rbx-dom.

Source changes are confined to src/dom.rs: additive destroy_many API,
test-only traversal instrumentation and regression tests. Other packaged
source is copied unchanged. Four clone-test snapshots have redundant final
blank lines normalized; their expected values are unchanged. The generated Cargo.toml omits
the upstream README reference; the packaged lockfile pins standalone tests.
LICENSE.txt preserves the upstream MIT license already carried by our
rbx_types fork of the same repository. Runtime dependency versions remain
unchanged; the root Cargo patch replaces only the selected source.

No automatic upstream updates or remote contributions are configured.
Test with `cargo test --locked --manifest-path vendor/rbx_dom_weak/Cargo.toml`.
