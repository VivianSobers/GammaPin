//! C1 Parquet (docs/CONTRACTS.md): one `part-0.parquet` per instrument under
//! `instrument=<key with | replaced by _>/`, rows sorted by (ltt_ms, recv_ns).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, Date32Array, FixedSizeListArray, Float64Array, Int32Array, Int64Array, StringArray,
    UInt8Array,
};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use gp_feed::{DEPTH, Event};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

fn depth_type(item: DataType) -> DataType {
    DataType::FixedSizeList(Arc::new(Field::new_list_field(item, false)), DEPTH as i32)
}

/// The C1 schema, in contract order.
pub fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("instrument_key", DataType::Utf8, false),
        Field::new("underlying", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("expiry", DataType::Date32, true),
        Field::new("strike", DataType::Float64, true),
        Field::new("lot_size", DataType::Int32, false),
        Field::new("ltt_ms", DataType::Int64, false),
        Field::new("recv_ns", DataType::Int64, false),
        Field::new("ltp", DataType::Float64, false),
        Field::new("ltq", DataType::Int64, false),
        Field::new("cum_volume", DataType::Int64, false),
        Field::new("oi", DataType::Int64, false),
        Field::new("bid_px", depth_type(DataType::Float64), false),
        Field::new("bid_qty", depth_type(DataType::Int64), false),
        Field::new("ask_px", depth_type(DataType::Float64), false),
        Field::new("ask_qty", depth_type(DataType::Int64), false),
        Field::new("src_mask", DataType::UInt8, false),
    ]))
}

fn f64_list(rows: &[&Event], get: impl Fn(&Event) -> [f64; DEPTH]) -> ArrayRef {
    let values: Float64Array = rows.iter().flat_map(|e| get(e)).collect();
    let field = Arc::new(Field::new_list_field(DataType::Float64, false));
    Arc::new(FixedSizeListArray::new(
        field,
        DEPTH as i32,
        Arc::new(values),
        None,
    ))
}

fn i64_list(rows: &[&Event], get: impl Fn(&Event) -> [i64; DEPTH]) -> ArrayRef {
    let values: Int64Array = rows.iter().flat_map(|e| get(e)).collect();
    let field = Arc::new(Field::new_list_field(DataType::Int64, false));
    Arc::new(FixedSizeListArray::new(
        field,
        DEPTH as i32,
        Arc::new(values),
        None,
    ))
}

/// One record batch of C1 rows.
pub fn batch(rows: &[&Event]) -> RecordBatch {
    let cols: Vec<ArrayRef> = vec![
        Arc::new(
            rows.iter()
                .map(|e| Some(e.instrument_key.as_str()))
                .collect::<StringArray>(),
        ),
        Arc::new(
            rows.iter()
                .map(|e| Some(e.underlying.as_str()))
                .collect::<StringArray>(),
        ),
        Arc::new(
            rows.iter()
                .map(|e| Some(e.kind.as_str()))
                .collect::<StringArray>(),
        ),
        Arc::new(
            rows.iter()
                .map(|e| e.expiry.map(|d| d.0))
                .collect::<Date32Array>(),
        ),
        Arc::new(rows.iter().map(|e| e.strike).collect::<Float64Array>()),
        Arc::new(
            rows.iter()
                .map(|e| Some(e.lot_size))
                .collect::<Int32Array>(),
        ),
        Arc::new(rows.iter().map(|e| Some(e.ltt_ms)).collect::<Int64Array>()),
        Arc::new(rows.iter().map(|e| Some(e.recv_ns)).collect::<Int64Array>()),
        Arc::new(rows.iter().map(|e| Some(e.ltp)).collect::<Float64Array>()),
        Arc::new(rows.iter().map(|e| Some(e.ltq)).collect::<Int64Array>()),
        Arc::new(
            rows.iter()
                .map(|e| Some(e.cum_volume))
                .collect::<Int64Array>(),
        ),
        Arc::new(rows.iter().map(|e| Some(e.oi)).collect::<Int64Array>()),
        f64_list(rows, |e| e.bid_px),
        i64_list(rows, |e| e.bid_qty),
        f64_list(rows, |e| e.ask_px),
        i64_list(rows, |e| e.ask_qty),
        Arc::new(
            rows.iter()
                .map(|e| Some(e.src_mask))
                .collect::<UInt8Array>(),
        ),
    ];
    RecordBatch::try_new(schema(), cols).expect("columns match the C1 schema")
}

/// `instrument=` directory name for a key.
pub fn partition_name(instrument_key: &str) -> String {
    format!("instrument={}", instrument_key.replace('|', "_"))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct PartitionStats {
    pub files: u64,
    pub rows: u64,
}

/// Write every instrument's rows under `day_dir`.
pub fn write_partitions(day_dir: &Path, events: &[Event]) -> anyhow::Result<PartitionStats> {
    let mut by_key: BTreeMap<&str, Vec<&Event>> = BTreeMap::new();
    for e in events {
        by_key.entry(&e.instrument_key).or_default().push(e);
    }
    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::try_new(3)?))
        .build();
    let mut stats = PartitionStats::default();
    for (key, mut rows) in by_key {
        rows.sort_by_key(|e| (e.ltt_ms, e.recv_ns));
        let dir = day_dir.join(partition_name(key));
        std::fs::create_dir_all(&dir)?;
        let file = std::fs::File::create(dir.join("part-0.parquet"))?;
        let mut w = ArrowWriter::try_new(file, schema(), Some(props.clone()))?;
        w.write(&batch(&rows))?;
        w.close()?;
        stats.files += 1;
        stats.rows += rows.len() as u64;
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Array, AsArray};
    use arrow::datatypes::{Float64Type, Int64Type};
    use gp_feed::{Date, Kind};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    fn ev(key: &str, kind: Kind, ltt: i64, recv: i64) -> Event {
        Event {
            instrument_key: key.into(),
            underlying: "NIFTY".into(),
            kind,
            expiry: (kind != Kind::Idx).then(|| Date::from_ymd(2026, 10, 20)),
            strike: kind.is_option().then_some(24_950.0),
            lot_size: if kind == Kind::Idx { 0 } else { 65 },
            ltt_ms: ltt,
            recv_ns: recv,
            ltp: 12.5,
            ltq: 65,
            cum_volume: 650,
            oi: 3,
            bid_px: [12.45, 12.4, 12.35, 12.3, 0.0],
            bid_qty: [65, 130, 195, 260, 0],
            ask_px: [12.55, 12.6, 0.0, 0.0, 0.0],
            ask_qty: [65, 65, 0, 0, 0],
            src_mask: 3,
        }
    }

    #[test]
    fn writes_contract_layout_and_types() {
        let dir = tempfile::tempdir().unwrap();
        let events = vec![
            ev("NSE_FO|7", Kind::Pe, 20, 2),
            ev("NSE_INDEX|Nifty 50", Kind::Idx, 5, 1),
            ev("NSE_FO|7", Kind::Pe, 10, 9),
            ev("NSE_FO|7", Kind::Pe, 10, 3),
        ];
        let stats = write_partitions(dir.path(), &events).unwrap();
        assert_eq!(stats, PartitionStats { files: 2, rows: 4 });

        let path = dir.path().join("instrument=NSE_FO_7/part-0.parquet");
        let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&path).unwrap())
            .unwrap()
            .build()
            .unwrap();
        let batches: Vec<RecordBatch> = reader.map(Result::unwrap).collect();
        let b = &batches[0];
        assert_eq!(b.schema(), schema());
        assert_eq!(b.num_rows(), 3);
        let ltt = b.column(6).as_primitive::<Int64Type>();
        let recv = b.column(7).as_primitive::<Int64Type>();
        assert_eq!(
            (0..3)
                .map(|i| (ltt.value(i), recv.value(i)))
                .collect::<Vec<_>>(),
            [(10, 3), (10, 9), (20, 2)]
        );
        let ask = b.column(14).as_fixed_size_list();
        assert_eq!(ask.value_length(), 5);
        let first = ask.value(0);
        assert_eq!(
            first.as_primitive::<Float64Type>().values()[..2],
            [12.55, 12.6]
        );
        let expiry = b.column(3).as_primitive::<arrow::datatypes::Date32Type>();
        assert_eq!(expiry.value(0), Date::from_ymd(2026, 10, 20).0);

        let idx = dir
            .path()
            .join("instrument=NSE_INDEX_Nifty 50/part-0.parquet");
        let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&idx).unwrap())
            .unwrap()
            .build()
            .unwrap();
        let b = reader.map(Result::unwrap).next().unwrap();
        assert!(b.column(3).is_null(0) && b.column(4).is_null(0));
    }
}
