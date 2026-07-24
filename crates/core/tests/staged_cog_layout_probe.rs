//! Maps the two staged raster objects to BigTIFF layout descriptors and asserts the frozen measured contract.
//! Manual command: `POURPOINT_STAGED_R2_COG_PROBE=1 cargo test -p pourpoint-core --test staged_cog_layout_probe -- --ignored`

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt};
use pourpoint_core::source::DatasetSource;

const STAGED_GRIT_PREFIX: &str = "https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/";
const FIRST_IFD_OFFSET: u64 = 200;
const FIRST_IFD_ENTRY_COUNT: u64 = 19;
const IMAGE_WIDTH: u64 = 1_070_000;
const IMAGE_LENGTH: u64 = 500_000;
const TILE_WIDTH: u64 = 512;
const TILE_LENGTH: u64 = 512;
const TILE_COLUMNS: u64 = 2_090;
const TILE_ROWS: u64 = 977;
const TILE_COUNT: u64 = 2_041_930;
const TILE_OFFSETS_START: u64 = 3_998;
const TILE_OFFSETS_END: u64 = 16_339_438;
const TILE_BYTE_COUNTS_START: u64 = 16_339_438;
const TILE_BYTE_COUNTS_END: u64 = 24_507_158;

const IMAGE_WIDTH_TAG: u16 = 256;
const IMAGE_LENGTH_TAG: u16 = 257;
const BITS_PER_SAMPLE_TAG: u16 = 258;
const COMPRESSION_TAG: u16 = 259;
const PREDICTOR_TAG: u16 = 317;
const TILE_WIDTH_TAG: u16 = 322;
const TILE_LENGTH_TAG: u16 = 323;
const TILE_OFFSETS_TAG: u16 = 324;
const TILE_BYTE_COUNTS_TAG: u16 = 325;
const SAMPLE_FORMAT_TAG: u16 = 339;
const GDAL_NODATA_TAG: u16 = 42_113;

const TYPE_ASCII: u16 = 2;
const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_LONG8: u16 = 16;

struct RasterExpectation {
    name: &'static str,
    artifact: &'static str,
    bits_per_sample: u64,
    sample_format: u64,
    object_size: u64,
    nodata: ExpectedNodata,
}

enum ExpectedNodata {
    Value(f64),
    Nan,
}

#[derive(Clone, Copy, Debug)]
struct IfdEntry {
    tiff_type: u16,
    count: u64,
    value_or_offset: u64,
    inline_value: [u8; 8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IndexDescriptor {
    tiff_type: u16,
    count: u64,
    element_width: u64,
    value_offset: u64,
    end_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IndexGeometry {
    first_ifd_offset: u64,
    tile_width: u64,
    tile_length: u64,
    tile_columns: u64,
    tile_rows: u64,
    tile_count: u64,
    tile_offsets: IndexDescriptor,
    tile_byte_counts: IndexDescriptor,
}

#[test]
#[ignore = "network-gated staged R2 COG probe; set POURPOINT_STAGED_R2_COG_PROBE=1"]
fn staged_planetary_cog_layouts_match_frozen_contract() {
    if std::env::var("POURPOINT_STAGED_R2_COG_PROBE").as_deref() != Ok("1") {
        eprintln!(
            "SKIPPED: staged COG layout guard requires POURPOINT_STAGED_R2_COG_PROBE=1; \
             no network request was made and NOTHING was verified"
        );
        return;
    }

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should start");
    runtime.block_on(async {
        // This endpoint returns 403 to Python urllib's default User-Agent but 206 to curl.
        // That is User-Agent filtering, not authentication; a future 403 does not mean
        // that this public prefix is private or requires credentials.
        let (store, root) =
            match DatasetSource::parse(STAGED_GRIT_PREFIX).expect("public R2 prefix should parse") {
                DatasetSource::Remote { store, root, .. } => (store, root),
                DatasetSource::Local(_) => panic!("staged GRIT URL should be a remote source"),
            };

        let expectations = [
            RasterExpectation {
                name: "flow_dir",
                artifact: "aux/d8/flow_dir.tif",
                bits_per_sample: 8,
                sample_format: 1,
                object_size: 50_686_516_478,
                nodata: ExpectedNodata::Value(255.0),
            },
            RasterExpectation {
                name: "flow_acc",
                artifact: "aux/d8/flow_acc.tif",
                bits_per_sample: 32,
                sample_format: 3,
                object_size: 205_069_870_081,
                nodata: ExpectedNodata::Nan,
            },
        ];

        let mut geometries = Vec::with_capacity(expectations.len());
        for expectation in &expectations {
            let path = remote_artifact_path(&root, expectation.artifact);
            geometries.push(
                probe_raster(Arc::clone(&store), &path, expectation).await,
            );
        }

        assert_eq!(
            geometries[0], geometries[1],
            "the two objects must have byte-identical index geometry; index values and payloads may differ"
        );
    });
}

fn remote_artifact_path(root: &ObjectPath, artifact: &str) -> ObjectPath {
    if root.as_ref().is_empty() {
        ObjectPath::from(artifact)
    } else {
        ObjectPath::from(format!(
            "{}/{artifact}",
            root.as_ref().trim_end_matches('/')
        ))
    }
}

async fn probe_raster(
    store: Arc<dyn ObjectStore>,
    path: &ObjectPath,
    expectation: &RasterExpectation,
) -> IndexGeometry {
    let metadata = store
        .head(path)
        .await
        .unwrap_or_else(|error| panic!("{} metadata head failed: {error}", expectation.name));
    assert_eq!(
        metadata.size, expectation.object_size,
        "{} object size must remain exactly {} bytes",
        expectation.name, expectation.object_size
    );

    let header = bounded_range(&store, path, 0..16, expectation.name).await;
    assert_eq!(
        &header[0..2],
        b"II",
        "{} must remain little-endian BigTIFF",
        expectation.name
    );
    assert_eq!(
        le_u16(&header[2..4]),
        43,
        "{} BigTIFF magic must remain 43",
        expectation.name
    );
    assert_eq!(
        le_u16(&header[4..6]),
        8,
        "{} BigTIFF offset size must remain 8",
        expectation.name
    );
    assert_eq!(
        le_u16(&header[6..8]),
        0,
        "{} BigTIFF reserved header field must remain 0",
        expectation.name
    );
    assert_eq!(
        le_u64(&header[8..16]),
        FIRST_IFD_OFFSET,
        "{} first IFD must remain at byte 200",
        expectation.name
    );

    // tiff 0.9.1 cannot report tag byte offsets: decoder::ifd::Entry keeps its
    // type, count, and offset private, while Decoder exposes values but no offsets.
    // This guard asserts byte extents, so that decoder cannot supply the evidence.
    // Integration tests are separate crates and only see public pourpoint_core API;
    // `mod cog` is pub(crate), so an in-crate pub(crate) or cfg(test) walker is
    // unreachable here. This file therefore owns the bounded walker.
    let entries = read_first_ifd(&store, path, expectation.name).await;

    for tag in [
        IMAGE_WIDTH_TAG,
        IMAGE_LENGTH_TAG,
        BITS_PER_SAMPLE_TAG,
        COMPRESSION_TAG,
        PREDICTOR_TAG,
        TILE_WIDTH_TAG,
        TILE_LENGTH_TAG,
        TILE_OFFSETS_TAG,
        TILE_BYTE_COUNTS_TAG,
        SAMPLE_FORMAT_TAG,
        GDAL_NODATA_TAG,
    ] {
        let matches = entries.get(&tag).map_or(0, Vec::len);
        assert_eq!(
            matches, 1,
            "{} must contain exactly one required tag {tag}",
            expectation.name
        );
    }

    let image_width = required_entry(&entries, IMAGE_WIDTH_TAG, expectation.name);
    let image_length = required_entry(&entries, IMAGE_LENGTH_TAG, expectation.name);
    let bits_per_sample = required_entry(&entries, BITS_PER_SAMPLE_TAG, expectation.name);
    let compression = required_entry(&entries, COMPRESSION_TAG, expectation.name);
    let predictor = required_entry(&entries, PREDICTOR_TAG, expectation.name);
    let tile_width = required_entry(&entries, TILE_WIDTH_TAG, expectation.name);
    let tile_length = required_entry(&entries, TILE_LENGTH_TAG, expectation.name);
    let tile_offsets = required_entry(&entries, TILE_OFFSETS_TAG, expectation.name);
    let tile_byte_counts = required_entry(&entries, TILE_BYTE_COUNTS_TAG, expectation.name);
    let sample_format = required_entry(&entries, SAMPLE_FORMAT_TAG, expectation.name);
    let nodata = required_entry(&entries, GDAL_NODATA_TAG, expectation.name);

    assert_entry_type(image_width, TYPE_LONG, IMAGE_WIDTH_TAG, expectation.name);
    assert_entry_type(image_length, TYPE_LONG, IMAGE_LENGTH_TAG, expectation.name);
    for (tag, entry) in [
        (BITS_PER_SAMPLE_TAG, bits_per_sample),
        (COMPRESSION_TAG, compression),
        (PREDICTOR_TAG, predictor),
        (TILE_WIDTH_TAG, tile_width),
        (TILE_LENGTH_TAG, tile_length),
        (SAMPLE_FORMAT_TAG, sample_format),
    ] {
        assert_entry_type(entry, TYPE_SHORT, tag, expectation.name);
    }

    let width = scalar_value(&store, path, image_width, expectation.name, IMAGE_WIDTH_TAG).await;
    let length = scalar_value(
        &store,
        path,
        image_length,
        expectation.name,
        IMAGE_LENGTH_TAG,
    )
    .await;
    let bits = scalar_value(
        &store,
        path,
        bits_per_sample,
        expectation.name,
        BITS_PER_SAMPLE_TAG,
    )
    .await;
    let compression_value =
        scalar_value(&store, path, compression, expectation.name, COMPRESSION_TAG).await;
    let predictor_value =
        scalar_value(&store, path, predictor, expectation.name, PREDICTOR_TAG).await;
    let tile_width_value =
        scalar_value(&store, path, tile_width, expectation.name, TILE_WIDTH_TAG).await;
    let tile_length_value =
        scalar_value(&store, path, tile_length, expectation.name, TILE_LENGTH_TAG).await;
    let sample_format_value = scalar_value(
        &store,
        path,
        sample_format,
        expectation.name,
        SAMPLE_FORMAT_TAG,
    )
    .await;

    assert_eq!(
        (width, length),
        (IMAGE_WIDTH, IMAGE_LENGTH),
        "{} dimensions must remain 1,070,000 x 500,000 px",
        expectation.name
    );
    assert_eq!(
        (tile_width_value, tile_length_value),
        (TILE_WIDTH, TILE_LENGTH),
        "{} tile dimensions must remain 512 x 512",
        expectation.name
    );
    assert_eq!(
        compression_value, 8,
        "{} compression must remain 8 (DEFLATE)",
        expectation.name
    );
    assert_eq!(
        predictor_value, 1,
        "{} Predictor tag (317) must be present with value 1",
        expectation.name
    );
    assert_eq!(
        bits, expectation.bits_per_sample,
        "{} BitsPerSample must remain {}",
        expectation.name, expectation.bits_per_sample
    );
    assert_eq!(
        sample_format_value, expectation.sample_format,
        "{} SampleFormat must remain {}",
        expectation.name, expectation.sample_format
    );

    let tile_columns = width.div_ceil(tile_width_value);
    let tile_rows = length.div_ceil(tile_length_value);
    let derived_tile_count = tile_columns
        .checked_mul(tile_rows)
        .expect("derived tile count must fit u64");
    assert_eq!(
        (tile_columns, tile_rows),
        (TILE_COLUMNS, TILE_ROWS),
        "{} tile grid must remain 2090 x 977",
        expectation.name
    );
    assert_eq!(
        derived_tile_count, TILE_COUNT,
        "{} derived tile count must remain ceil(1,070,000 / 512) = 2090, ceil(500,000 / 512) = 977, and 2090 * 977 = 2,041,930",
        expectation.name
    );
    assert_eq!(
        tile_offsets.count, TILE_COUNT,
        "{} TileOffsets tag-array count must remain 2,041,930",
        expectation.name
    );
    assert_eq!(
        tile_byte_counts.count, TILE_COUNT,
        "{} TileByteCounts tag-array count must remain 2,041,930",
        expectation.name
    );

    let tile_offsets_descriptor =
        index_descriptor(tile_offsets, TYPE_LONG8, TILE_OFFSETS_TAG, expectation.name);
    let tile_byte_counts_descriptor = index_descriptor(
        tile_byte_counts,
        TYPE_LONG,
        TILE_BYTE_COUNTS_TAG,
        expectation.name,
    );
    assert_eq!(
        (
            tile_offsets_descriptor.value_offset,
            tile_offsets_descriptor.end_offset
        ),
        (TILE_OFFSETS_START, TILE_OFFSETS_END),
        "{} TileOffsets must span bytes [3,998, 16,339,438)",
        expectation.name
    );
    assert_eq!(
        (
            tile_byte_counts_descriptor.value_offset,
            tile_byte_counts_descriptor.end_offset
        ),
        (TILE_BYTE_COUNTS_START, TILE_BYTE_COUNTS_END),
        "{} TileByteCounts must span bytes [16,339,438, 24,507,158)",
        expectation.name
    );

    assert_entry_type(nodata, TYPE_ASCII, GDAL_NODATA_TAG, expectation.name);
    assert_eq!(
        nodata.count, 4,
        "{} GDAL_NODATA tag 42113 must have ASCII count 4",
        expectation.name
    );
    let nodata_bytes =
        entry_value_bytes(&store, path, nodata, expectation.name, GDAL_NODATA_TAG).await;
    let nodata_text = std::str::from_utf8(&nodata_bytes)
        .unwrap_or_else(|error| panic!("{} GDAL_NODATA is not ASCII: {error}", expectation.name))
        .trim_end_matches('\0')
        .trim_matches(|character: char| character.is_ascii_whitespace());
    let nodata_value = nodata_text.parse::<f64>().unwrap_or_else(|error| {
        panic!(
            "{} GDAL_NODATA value {nodata_text:?} is not numeric: {error}",
            expectation.name
        )
    });
    // The 2026-07-24 second probe measured both ASCII content and sentinel values:
    // `255\0` for flow_dir and lowercase `nan\0` for flow_acc. Numeric parsing guards
    // against spelling changes that preserve the measured 4-byte length, such as `NaN\0`;
    // re-encodes that change the byte length are deliberately caught by the count assertion.
    match expectation.nodata {
        ExpectedNodata::Value(expected) => assert_eq!(
            nodata_value, expected,
            "{} GDAL_NODATA sentinel must remain numerically 255",
            expectation.name
        ),
        ExpectedNodata::Nan => assert!(
            nodata_value.is_nan(),
            "{} GDAL_NODATA sentinel must remain NaN",
            expectation.name
        ),
    }

    IndexGeometry {
        first_ifd_offset: FIRST_IFD_OFFSET,
        tile_width: tile_width_value,
        tile_length: tile_length_value,
        tile_columns,
        tile_rows,
        tile_count: derived_tile_count,
        tile_offsets: tile_offsets_descriptor,
        tile_byte_counts: tile_byte_counts_descriptor,
    }
}

async fn read_first_ifd(
    store: &Arc<dyn ObjectStore>,
    path: &ObjectPath,
    raster_name: &str,
) -> HashMap<u16, Vec<IfdEntry>> {
    let count_end = FIRST_IFD_OFFSET
        .checked_add(8)
        .expect("IFD count range must fit u64");
    let count_bytes = bounded_range(store, path, FIRST_IFD_OFFSET..count_end, raster_name).await;
    let entry_count = le_u64(&count_bytes);
    assert_eq!(
        entry_count, FIRST_IFD_ENTRY_COUNT,
        "{raster_name} first-IFD entry count must remain 19"
    );

    let entries_byte_count = entry_count
        .checked_mul(20)
        .expect("IFD entry byte count must fit u64");
    let entries_start = count_end;
    let entries_end = entries_start
        .checked_add(entries_byte_count)
        .expect("IFD entries range must fit u64");
    let entry_bytes = bounded_range(store, path, entries_start..entries_end, raster_name).await;
    let next_ifd_end = entries_end
        .checked_add(8)
        .expect("next-IFD pointer range must fit u64");
    let next_ifd_bytes = bounded_range(store, path, entries_end..next_ifd_end, raster_name).await;
    let _next_ifd_offset = le_u64(&next_ifd_bytes);

    let mut entries = HashMap::<u16, Vec<IfdEntry>>::new();
    for bytes in entry_bytes.chunks_exact(20) {
        let tag = le_u16(&bytes[0..2]);
        let mut inline_value = [0_u8; 8];
        inline_value.copy_from_slice(&bytes[12..20]);
        entries.entry(tag).or_default().push(IfdEntry {
            tiff_type: le_u16(&bytes[2..4]),
            count: le_u64(&bytes[4..12]),
            value_or_offset: le_u64(&bytes[12..20]),
            inline_value,
        });
    }
    assert_eq!(
        entries.values().map(Vec::len).sum::<usize>(),
        usize::try_from(entry_count).expect("IFD entry count must fit usize"),
        "{raster_name} walker must parse all 19 IFD entries"
    );
    entries
}

fn required_entry(entries: &HashMap<u16, Vec<IfdEntry>>, tag: u16, raster_name: &str) -> IfdEntry {
    let matches = entries
        .get(&tag)
        .unwrap_or_else(|| panic!("{raster_name} is missing required tag {tag}"));
    assert_eq!(
        matches.len(),
        1,
        "{raster_name} has duplicate required tag {tag}"
    );
    matches[0]
}

fn assert_entry_type(entry: IfdEntry, expected_type: u16, tag: u16, raster_name: &str) {
    assert_eq!(
        entry.tiff_type, expected_type,
        "{raster_name} tag {tag} TIFF type drifted from {expected_type}"
    );
}

fn index_descriptor(
    entry: IfdEntry,
    expected_type: u16,
    tag: u16,
    raster_name: &str,
) -> IndexDescriptor {
    assert_entry_type(entry, expected_type, tag, raster_name);
    let element_width = element_width(entry.tiff_type, raster_name, tag);
    let byte_count = entry
        .count
        .checked_mul(element_width)
        .unwrap_or_else(|| panic!("{raster_name} tag {tag} byte count overflowed"));
    let end_offset = entry
        .value_or_offset
        .checked_add(byte_count)
        .unwrap_or_else(|| panic!("{raster_name} tag {tag} extent overflowed"));
    IndexDescriptor {
        tiff_type: entry.tiff_type,
        count: entry.count,
        element_width,
        value_offset: entry.value_or_offset,
        end_offset,
    }
}

async fn scalar_value(
    store: &Arc<dyn ObjectStore>,
    path: &ObjectPath,
    entry: IfdEntry,
    raster_name: &str,
    tag: u16,
) -> u64 {
    assert_eq!(
        entry.count, 1,
        "{raster_name} scalar tag {tag} must have count 1"
    );
    let bytes = entry_value_bytes(store, path, entry, raster_name, tag).await;
    match entry.tiff_type {
        1 | 2 => u64::from(bytes[0]),
        3 => u64::from(le_u16(&bytes)),
        4 => u64::from(le_u32(&bytes)),
        16 => le_u64(&bytes),
        other => panic!("{raster_name} tag {tag} has unsupported scalar TIFF type {other}"),
    }
}

async fn entry_value_bytes(
    store: &Arc<dyn ObjectStore>,
    path: &ObjectPath,
    entry: IfdEntry,
    raster_name: &str,
    tag: u16,
) -> Vec<u8> {
    let width = element_width(entry.tiff_type, raster_name, tag);
    let byte_count = entry
        .count
        .checked_mul(width)
        .unwrap_or_else(|| panic!("{raster_name} tag {tag} byte count overflowed"));
    let byte_count_usize = usize::try_from(byte_count)
        .unwrap_or_else(|_| panic!("{raster_name} tag {tag} byte count does not fit usize"));
    if byte_count <= 8 {
        entry.inline_value[..byte_count_usize].to_vec()
    } else {
        let end = entry
            .value_or_offset
            .checked_add(byte_count)
            .unwrap_or_else(|| panic!("{raster_name} tag {tag} value range overflowed"));
        bounded_range(store, path, entry.value_or_offset..end, raster_name).await
    }
}

fn element_width(tiff_type: u16, raster_name: &str, tag: u16) -> u64 {
    match tiff_type {
        1 | 2 => 1,
        3 => 2,
        4 => 4,
        16 => 8,
        other => panic!("{raster_name} tag {tag} has unsupported TIFF type {other}"),
    }
}

async fn bounded_range(
    store: &Arc<dyn ObjectStore>,
    path: &ObjectPath,
    range: Range<u64>,
    raster_name: &str,
) -> Vec<u8> {
    assert!(
        range.start <= range.end,
        "{raster_name} bounded range start exceeds its end"
    );
    let requested = range
        .end
        .checked_sub(range.start)
        .expect("bounded range length must not underflow");
    let requested_usize = usize::try_from(requested)
        .unwrap_or_else(|_| panic!("{raster_name} bounded range length does not fit usize"));
    let bytes = store
        .get_range(path, range)
        .await
        .unwrap_or_else(|error| panic!("{raster_name} bounded range read failed: {error}"));
    assert_eq!(
        bytes.len(),
        requested_usize,
        "{raster_name} bounded range returned the wrong byte count"
    );
    bytes.to_vec()
}

fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(
        bytes
            .try_into()
            .expect("little-endian u16 requires exactly 2 bytes"),
    )
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(
        bytes
            .try_into()
            .expect("little-endian u32 requires exactly 4 bytes"),
    )
}

fn le_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(
        bytes
            .try_into()
            .expect("little-endian u64 requires exactly 8 bytes"),
    )
}
