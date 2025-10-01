default:
    @just --list

# Run pre-commit hooks on all files, including autoformatting
pre-commit-all:
    pre-commit run --all-files

# Run 'cargo run' on the project
run *ARGS:
    cargo run {{ARGS}}

# Run 'bacon' to run the project (auto-recompiles)
watch *ARGS:
	bacon --job run -- -- {{ ARGS }}

# Run CLI - Note: for multi-word strings, escape quotes: just cli group create TestGroup --description \"Test cluster\"
cli *ARGS:
    cargo run --bin pattern-cli -- {{ARGS}}

# Shortcuts for group operations
group-create name description="Default group" pattern="round_robin":
    cargo run --bin pattern-cli -- group create "{{name}}" --description "{{description}}" --pattern {{pattern}}

group-add group agent role="regular":
    cargo run --bin pattern-cli -- group add-member "{{group}}" "{{agent}}" --role {{role}}

pattern:
    cargo run --bin pattern-cli -- -c bsky_agent/constellation.toml chat --discord --group "Pattern Cluster"

pattern-db:
    surreal start rocksdb://./pattern_external.db
