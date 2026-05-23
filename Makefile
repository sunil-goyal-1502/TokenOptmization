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

bench-agent:
	cargo run -p tokenopt-cli -- bench agent-loop --turns 25 --payload-bytes 12000 --keep-recent 2

test-agent-http:
	cargo run -p tokenopt-server -- --bind 127.0.0.1:8787 &
	sleep 1
	python3 examples/test_agent_loop.py

ts-build:
	cd bindings/typescript && npm install && npm run build
