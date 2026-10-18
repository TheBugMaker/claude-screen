# Common development commands. Run `make help` to list them.

CARGO ?= cargo

.DEFAULT_GOAL := help
.PHONY: help run build release fmt fmt-check lint test deny check clean log

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}'

run: ## Run ccs (pass flags with ARGS="--log-level debug")
	$(CARGO) run -p ccs -- $(ARGS)

build: ## Debug build
	$(CARGO) build --workspace

release: ## Optimized build (target/release/ccs)
	$(CARGO) build --release -p ccs

fmt: ## Format all code
	$(CARGO) fmt --all

fmt-check: ## Check formatting without changing files
	$(CARGO) fmt --all --check

lint: ## Run clippy, treating warnings as errors
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings

test: ## Run all tests
	$(CARGO) test --workspace --locked

deny: ## Check licenses, advisories and sources (needs: cargo install cargo-deny)
	$(CARGO) deny check

check: fmt-check lint test deny ## Run all checks: format, lint, test, deny

clean: ## Remove build artifacts
	$(CARGO) clean

log: ## Follow the ccs log file
	tail -F $${XDG_STATE_HOME:-$$HOME/.local/state}/ccs/ccs.log
