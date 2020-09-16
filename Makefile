build:
	cargo build
test:
	RUST_BACKTRACE=full cargo watch -s 'cargo test -- --nocapture'

docker:
	docker build -t bodymindarts/cepler:latest . && docker push bodymindarts/cepler:latest

integration: build
	bats -t -r test/integration

test-in-ci:
	cargo clippy --all-features
	cargo test --all-features --verbose --locked



.PHONY: test
