# Reader-floor check red baseline — 2026-08-06

Observed before any reader-floor annotation was added to an offering page.

Command: `python3 scripts/check_reader_floors.py --root .`
Exit status: `1`

```text
reader-floor check failed: 11 bare occurrence(s) in 6 offering page(s)
CHANGELOG.md:10: bare occurrence of basin-delineations-public
README.md:89: bare occurrence of basin-delineations-public
README.md:102: bare occurrence of basin-delineations-public
README.md:109: bare occurrence of basin-delineations-public
README.md:119: bare occurrence of basin-delineations-public
README.md:174: bare occurrence of basin-delineations-public
crates/python/README.md:36: bare occurrence of basin-delineations-public
docs/guide/datasets.md:30: bare occurrence of basin-delineations-public
docs/guide/datasets.md:49: bare occurrence of basin-delineations-public
docs/guide/staged-api.md:16: bare occurrence of basin-delineations-public
docs/quickstart.md:50: bare occurrence of basin-delineations-public
```
