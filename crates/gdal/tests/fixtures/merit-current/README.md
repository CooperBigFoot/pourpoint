# Current local MERIT evidence

This evidence is separate from immutable Oracle C. It was captured from a
licensed local Pfaf-23 HFX build. No MERIT source artifact is committed or
published.

Build provenance:

- HFX checkout commit: `5603645f91f80873e3d1cb9c236feb303def949e`
- The capture records that exact checkout commit; it does not describe it as
  `origin/main`.
- Adapter version recorded by `manifest.json`: `0.2.0`
- HFX format: `0.3.0`
- D8 declaration: `hfx.aux.d8_raster.v2`

The adapter build completed its strict phase-4 validation before capture. The
capture command was:

```bash
POURPOINT_MERIT_RECAPTURE_ROOT="$LICENSED_MERIT_HFX_ROOT" \
POURPOINT_MERIT_RECAPTURE_OUTPUT=crates/gdal/tests/fixtures/merit-current/rhine-basel.json \
POURPOINT_MERIT_RECAPTURE_HFX_COMMIT=5603645f91f80873e3d1cb9c236feb303def949e \
POURPOINT_MERIT_RECAPTURE_BLESS=1 \
cargo test -p pourpoint-gdal --test merit_local_recapture -- --ignored --nocapture
```

The literal local root is intentionally not recorded. The capture records
vector-quantized seed provenance, scalar results, and canonical WKB SHA-256, but
not licensed source data or geometry bytes.
