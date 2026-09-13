.PHONY: run build release check clean watch fmt lint test test-all kill-port publish publish-dry

PORT := 8080

kill-port:
	@lsof -ti :$(PORT) | xargs kill -9 2>/dev/null; true

run: kill-port
	cargo run --example proxy

build:
	cargo build

release:
	cargo build --release

check:
	cargo check
	cargo check --features decoder

clean:
	cargo clean

watch: kill-port
	cargo watch -x "run --example proxy"

test:
	cargo test
	cargo test --features decoder

test-all:
	cargo test --all-targets --all-features

fmt:
	cargo fmt --all

lint:
	cargo clippy --all-targets --all-features -- -D warnings

publish:
	@./scripts/publish.sh

publish-dry:
	@./scripts/publish.sh --dry-run
