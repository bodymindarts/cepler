build:
	cargo build
test:
	RUST_BACKTRACE=full cargo watch -s 'cargo test -- --nocapture'

integration: build
	bats -t -r test/integration

test-in-ci:
	cargo clippy --all-features
	cargo test --all-features --verbose --locked

build-x86_64-unknown-linux-musl-release:
	cargo build --release --locked

build-x86_64-apple-darwin-release:
	/workspace/compile.sh

.PHONY: test
