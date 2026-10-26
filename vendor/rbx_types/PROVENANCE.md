# Local rbx_types fork

Base: crates.io rbx_types 3.1.0, checksum
de3b89eefdd71f5e2a25543b1ead9f6ea2ed3fdfba8b397cbdb0de053eb2463e.
Use this package checksum to verify the upstream source when refreshing.

TweenInfo source changes are backported from
https://github.com/rojo-rbx/rbx-dom/pull/663 at inspected head
f8f6e64ccbd46c51b9a0a5da0956193a7b64a4b3. The five changed source files are
basic_types.rs, variant.rs, attributes/reader.rs, attributes/writer.rs and
attributes/type_id.rs. These changes applied cleanly to the packaged 3.1.0
source. The existing attribute snapshot's extra blank line at EOF was normalized
without changing expected values; its tests still pass. Other upstream changes
are not incorporated.

LICENSE.txt preserves the upstream MIT notice. The package's generated manifest
is retained except for its reference to the omitted upstream README. This copy
is maintained locally; no tracking branch, automatic update or PR-ref dependency
is configured. Cargo.lock selects this path for all transitive consumers.

Run the fork's tests with:
`cargo test --locked --manifest-path vendor/rbx_types/Cargo.toml --features serde`.
Its separate lockfile makes that test command reproducible. Build output is
ignored locally. Rojo integration tests cover TweenInfo mixed attributes,
binary/XML roundtrips and malformed input in tests/tweeninfo_attributes.rs.
