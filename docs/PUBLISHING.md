# Publishing TokenOpt

## crates.io — `tokenopt-core`

```bash
cd crates/tokenopt-core
cargo publish --dry-run
cargo publish
```

## PyPI — `tokenopt` (HTTP client + optional native)

```bash
pip install maturin
cd bindings/python
maturin build --release -o dist/
# wheel includes tokenopt._native when built with maturin
maturin publish
```

## npm — `@tokenopt/client` and `@tokenopt/native`

```bash
cd bindings/typescript && npm publish --access public
cd bindings/node && npm install && npm run build && npm publish --access public
```
