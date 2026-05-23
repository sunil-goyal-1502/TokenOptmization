.PHONY: build test release server cli analyze-sample

build:
	cargo build --workspace

test:
	cargo test --workspace

release:
	cargo build --release --workspace

server:
	cargo run -p tokenopt-server -- --bind 127.0.0.1:8787

cli:
	cargo build -p tokenopt-cli

analyze-sample:
	cargo run -p tokenopt-cli -- analyze --trace fixtures/sample-trace.json

compile-sample:
	cargo run -p tokenopt-cli -- compile --trace fixtures/sample-trace.json --budget 32000 --soft-sufficiency

ts-build:
	cd bindings/typescript && npm install && npm run build
