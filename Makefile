CARGO ?= cargo

.PHONY: build test lint clean run fmt

build:
	$(CARGO) build

test:
	$(CARGO) test --workspace

lint:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

clean:
	$(CARGO) clean

run:
	$(CARGO) run -p pirs
