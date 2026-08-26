.PHONY: run build release check clean watch fmt lint test kill-port publish publish-dry

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

clean:
	cargo clean

watch: kill-port
	cargo watch -x "run --example proxy"

test:
	cargo test

publish:
	@./scripts/publish.sh

publish-dry:
	@./scripts/publish.sh --dry-run

