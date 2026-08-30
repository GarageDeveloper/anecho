# Anecho — developer entry points. See CLAUDE.md for the rules behind these targets.

CONTRACT_DIR := contract
# Baseline for breaking-change detection: latest contract tag, or main before the first tag.
CONTRACT_BASE ?= $(shell git tag --list 'contract-v*' --sort=-v:refname | head -1)
ifeq ($(CONTRACT_BASE),)
CONTRACT_BASE := main
endif

.PHONY: help contract-check contract-lint generate build test fmt fmt-check lint check serve dev testbench frontend-check clean

help:
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-16s %s\n", $$1, $$2}'

contract-lint: ## Lint the protobuf schemas
	cd $(CONTRACT_DIR) && buf lint

contract-check: contract-lint ## Refuse any breaking change to the contract (mandatory before merge)
	@if git ls-tree -d "$(CONTRACT_BASE):$(CONTRACT_DIR)" >/dev/null 2>&1; then \
		echo "buf breaking against $(CONTRACT_BASE)"; \
		cd $(CONTRACT_DIR) && buf breaking --against "../.git#ref=$(CONTRACT_BASE),subdir=$(CONTRACT_DIR)"; \
	else \
		echo "WARNING: $(CONTRACT_BASE) has no $(CONTRACT_DIR)/ yet — breaking check skipped"; \
	fi

generate: ## Generate TypeScript types from the contract (Rust is generated at cargo build time)
	cd $(CONTRACT_DIR) && buf generate

build: ## Build the whole workspace
	cargo build --workspace

test: ## Run all tests (unit, golden, headless integration)
	cargo test --workspace

fmt: ## Format Rust code
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint: ## Clippy, warnings are errors
	cargo clippy --workspace --all-targets -- -D warnings

frontend-check: generate ## Type-check and build the web frontend
	cd frontend && pnpm install --frozen-lockfile && pnpm exec svelte-check --tsconfig ./tsconfig.json && pnpm run check:mock && pnpm exec vitest run && pnpm exec vite build

check: fmt-check lint test contract-check frontend-check ## Everything CI runs

serve: ## Run the headless backend
	cargo run -p anecho -- serve

dev: ## Run the Tauri frontend in dev mode (needs the backend crates built)
	cd frontend && pnpm tauri dev

testbench: ## A/B bench: compare Anecho with REW (REW must run in API mode on :4735)
	cargo run -p anecho-testbench -- compare

testbench-thd: ## A/B bench: same sine through a loopback device (BlackHole) — fundamental, RTA peak, THD, THD+N
	cargo run -p anecho-testbench -- compare-thd

clean:
	cargo clean
	rm -rf frontend/src/gen
