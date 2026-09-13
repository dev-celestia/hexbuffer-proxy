.PHONY: app run build release check clean watch fmt lint test test-all kill-ports publish publish-dry

PORT := 8080
UI_PORT := 8081
UPSTREAM_PORT := 8082

kill-ports:
	@lsof -ti :$(PORT) -ti :$(UI_PORT) -ti :$(UPSTREAM_PORT) | xargs kill -9 2>/dev/null; true

app: kill-ports
	cargo run --example test_app --features decoder

run: kill-ports
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
