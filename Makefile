# Local quality gates and offline analysis entry points.
REPO ?= .

.PHONY: fmt lint test check analyze json

# Format every workspace crate using the standard Rust formatter.
fmt:
	cargo fmt --all

# Run the same pedantic Clippy checks used by CI.
lint:
	cargo lint

# Execute unit, fixture and documentation tests with the locked dependency graph.
test:
	cargo test --workspace --locked

# Verify formatting, lints, tests and repository text policy.
check:
	cargo fmt --all --check
	cargo lint
	cargo test --workspace --locked
	bash tools/check-text.sh

# Analyze committed HEAD in REPO and print a plain-text table.
analyze:
	cargo run --locked -- analyze "$(REPO)"

# Analyze committed HEAD in REPO and print the versioned JSON report.
json:
	cargo run --locked -- analyze "$(REPO)" --json
