#!/bin/sh
set -eu
export ACYCLIC_REQUIRE_APP=1 PYTHONUNBUFFERED=1
cargo build --quiet --locked --profile journeys -p acyclic -p acyclic-fixtures
uv run --no-project --with playwright==1.64.0 python -m playwright install chromium webkit
uv run --no-project --with playwright==1.64.0 python workspace/acyclic-app/tests/browser/app.py target/journeys/acyclic target/journeys/acyclic-fixtures
