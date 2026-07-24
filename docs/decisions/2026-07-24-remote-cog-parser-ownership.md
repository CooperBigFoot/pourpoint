# Own the remote COG parser and chunk decoder

The remote COG path currently constructs `tiff::decoder::Decoder`, which eagerly materializes `TileOffsets` and `TileByteCounts`. The staged planetary rasters contain 2,041,930 tiles. Their little-endian BigTIFF IFD stores `TileOffsets` as LONG8 at `[3998, 16339438)` and `TileByteCounts` as LONG at `[16339438, 24507158)`, so metadata construction reads 24,503,160 index bytes before a window is selected. Both rasters use TIFF `Compression=8` and predictor 1.

M1-S2a provides a classic-TIFF fixture with magic 42 and 4-byte offsets and a planetary BigTIFF fixture with magic 43 and 8-byte offsets. The BigTIFF fixture preserves the staged mixed-width index layout. This decision evaluates an owned minimal IFD walker and `async-tiff` 0.3.0 against those fixtures and the staged layout.

The owned prototype reaches dimensions and GeoTIFF scale/tiepoint without either index array. Classic TIFF uses four requests and 274 requested bytes; BigTIFF uses four requests and 476 requested bytes. The BigTIFF total is below the first index offset, and both indexes remain descriptors. The prototype preserves independent type, element width, count, and storage for LONG8 8-byte offsets and LONG 4-byte byte counts. It passes both M1-S2a fixtures, including magic 42 with 4-byte offsets and magic 43 with 8-byte offsets.

The `async-tiff` 0.3.0 candidate is rejected on source inspection. `ImageFileDirectoryReader::read` reads every tag; `read_tag_value` follows out-of-line multi-value tags into `TagValue::List`; `ImageFileDirectory::from_tags` converts both tile indexes into vectors. It therefore materializes at least 24,503,160 planetary index bytes before returning the IFD. Requests were not measured because the candidate already fails the no-materialization criterion. Source inspection shows SHORT, LONG, and LONG8 support and support for both magic values and offset widths, but values are eagerly materialized.

The owned chunk-decode seam uses a direct zlib inflater and owns predictor dispatch. A known-value `Compression=8` payload expands under predictor 1 to `[1, 2, 3, 4, 5, 6, 7, 8]` with no horizontal differencing. Source inspection shows that `async-tiff` exposes tile decode with parsed predictor application. Both candidates keep core GDAL-free and PROJ-free: the owned walker uses byte/object-store facilities and the decoder uses `flate2`, while `async-tiff` requires neither GDAL nor PROJ.

The `async-tiff` result is an inspection-based rejection, not a failed prototype. Its 0.3.0 archive was available in the local Cargo cache but was absent from this repository's lockfile. It was not added or compiled because its public metadata construction model already violates the decisive lazy-index criterion and the executor has no network access.

`crates/core` will own the minimal classic-TIFF/BigTIFF IFD and lazy-index walker used by remote reads. It will preserve tile indexes as independently typed descriptors and resolve only entries needed for a requested window. `crates/core` will also own DEFLATE expansion and predictor application after compressed chunks are fetched.

The retained M1 prototype remains test-only. M2 introduces the production extent consumer and deletes `EXTENT_HEADER_RANGE_BYTES` instead of raising or repurposing it. M3 introduces production index descriptors, resolves only covered entries, fetches bounded chunks, and ships DEFLATE expansion plus predictor application.

`flate2 = "=1.1.9"` is declared in `[dev-dependencies]` for the test-only decode prototype. M3 must promote it to `[dependencies]` when owned decode enters library code. The same promotion obligation applies to any lazy-reader crate selected in the future: test-only or transitive availability is insufficient for a production import.

The `tiff` crate remains only for local window encoding in `write_window_geotiff`; it will not own the remote read path. `crates/core` remains GDAL-free and PROJ-free.

The remote parser surface stays limited to TIFF types and tags required by the HFX COG contract, but pourpoint assumes responsibility for checked offset arithmetic, classic/BigTIFF structural differences, mixed index widths, DEFLATE framing, and predictor semantics. Unsupported layouts must fail loudly rather than fall back to eager reads. The fixture evidence fixes the intended seam before M2 and M3 add production consumers.
