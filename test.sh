#!/bin/bash
# OneAmp Test Script
# Run all tests and checks

set -e

echo "🔍 Running cargo check..."
cargo check

echo ""
echo "🧪 Running unit tests..."
cargo test --lib

echo ""
echo "📋 Running clippy..."
cargo clippy -- -D warnings

echo ""
echo "🎨 Running rustfmt check..."
cargo fmt -- --check

echo ""
echo "✅ All checks passed!"
