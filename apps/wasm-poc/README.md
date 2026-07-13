# WASM POCs

Build a POC from this checkout before opening it. The repository Cargo config
writes component artifacts to the checkout-level `target/` directory.

```sh
cd apps/wasm-poc/counter
cargo component build --release --target wasm32-wasip2
cd ../../..
plexi app open apps/wasm-poc/counter
```

`cargo-component` writes the host-loadable component under
`target/wasm32-wasip1/release/`. Its `wasm32-wasip2` output is the core module,
not the component entry Plexi opens.

Run the build and open commands from the same checkout. A different worktree
has its own `target/` directory.
