.PHONY: build-wasms integration-test

build-wasms:
	cargo build --release --target wasm32-unknown-unknown \
		-p registry -p intent_settlement -p proofs -p badges

integration-test: build-wasms
	cargo test -p integration-tests
