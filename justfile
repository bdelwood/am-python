set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

sync:
    uv sync --group dev

fmt:
    cargo fmt --all
    uvx ruff format .

fmt-check target="all":
    just fmt-check-{{target}}

fmt-check-rs:
    cargo fmt --all -- --check

fmt-check-py:
    uvx ruff format --check .

fmt-check-all:
    just fmt-check-rs
    just fmt-check-py

lint target="all":
    just lint-{{target}}

lint-rs:
    cargo clippy --release

lint-py:
    uvx ruff check .

lint-all:
    just lint-rs
    just lint-py

typecheck:
    uv run --group dev pyrefly check

test target="all":
    just test-{{target}}

test-py:
    uv run --group dev pytest

test-rs:
    cargo test --release

test-all:
    just test-py
    just test-rs

precommit:
    uv run --group dev pre-commit run --all-files --show-diff-on-failure

prepush:
    uv run --group dev pre-commit run --all-files --hook-stage pre-push --show-diff-on-failure

docs target="all":
    just docs-{{target}}

docs-py:
    mkdir -p docs/_build/python
    uv run --group dev sphinx-build -W --keep-going -b html docs docs/_build/python

docs-rs:
    cargo doc --no-deps --document-private-items

docs-all:
    just docs-py
    just docs-rs
    rm -rf public
    mkdir -p public/rust
    cp -R docs/_build/python/. public/
    cp -R target/doc/. public/rust/
