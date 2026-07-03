.PHONY: docs docs-serve

docs:
	uv run --project crates/python --only-group docs mkdocs build --strict

docs-serve:
	uv run --project crates/python --only-group docs mkdocs serve
