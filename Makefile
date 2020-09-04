build:
	cargo build
test:
	RUST_BACKTRACE=full cargo watch -s 'cargo test -- --nocapture'

docker:
	docker build -t bodymindarts/cepler:latest . && docker push bodymindarts/cepler:latest

.PHONY: test
