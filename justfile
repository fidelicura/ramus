# Copyright 2026 Ramus
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

set quiet

# Build workspace (profile: dev|release)
build profile="dev":
    cargo build \
        --workspace \
        --profile={{profile}}

# Test workspace (profile: dev|release)
test profile="dev":
    cargo test \
        --workspace \
        --profile={{profile}}

alias doc := docs
# Build library API docs, no dependencies
docs:
    cargo doc \
        --no-deps \
        --lib

alias cov := coverage
# Coverage report (format: html|lcov|json|...)
coverage format="html":
    cargo llvm-cov --{{format}}
    cargo llvm-cov report --show-missing-lines

# Lint source code with Clippy
lint:
    cargo clippy --all

# Audit dependencies for known vulnerabilities
audit:
    cargo audit

alias clear := clean
# Remove build artifacts
clean:
    cargo clean

# Run all pre-push checks (audit, lint, coverage)
push: audit lint coverage

# Run Arbiter example (pass-through arguments)
run *args:
    cargo run \
        --package=arbiter \
        --profile=release \
        -- \
        {{args}}

# Take a flamegraph of Arbiter example
flame *args:
    cargo build \
        --package=arbiter \
        --profile=bench
    sudo "$(command -v flamegraph)" \
        -- \
        ./target/release/arbiter {{args}}
