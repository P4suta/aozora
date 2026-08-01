//! Samply gecko-profile JSON loader.
//!
//! The gecko format is column-oriented: each table (`frameTable`,
//! `funcTable`, `stackTable`, …) is a struct of parallel arrays
//! indexed by row id. We translate to row-oriented Rust structs
//! at load time so callers can iterate idiomatically without
//! cross-referencing column lengths on every access.
//!
//! ## Why not `#[derive(Deserialize)]` the whole schema
//!
//! The full gecko schema has dozens of fields we ignore (markers,
//! profilerOverhead, counters, pages, page state, etc.) and several
//! version-skewed shapes (samples-as-table vs samples-as-array). A
//! field-by-field manual extraction over `serde_json::Value` is
//! both shorter and more forgiving of schema drift than maintaining
//! a complete strongly-typed mirror.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use serde_json::Value;

use crate::trace::{FrameRow, FuncRow, Library, ResourceRow, Sample, StackEntry, Thread, Trace};

/// Load failures.
#[derive(Debug, thiserror::Error)]
pub enum TraceLoadError {
    /// Opening the trace file failed. Read/decode failures that
    /// happen *while* streaming (including corrupt `.gz` data, which
    /// `serde_json::from_reader` reports as an i/o-category error)
    /// surface as [`TraceLoadError::Json`] instead.
    #[error("io error reading {path}: {source}")]
    Io {
        /// The trace path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file was read but is not valid JSON.
    #[error("json parse error in {path}: {source}")]
    Json {
        /// The trace path whose contents failed to parse.
        path: PathBuf,
        /// Underlying `serde_json` parse error.
        #[source]
        source: serde_json::Error,
    },
    /// The JSON parsed but a required gecko table/column was missing
    /// or had the wrong shape (e.g. mismatched column lengths).
    #[error("gecko schema: missing or malformed `{field}`")]
    BadSchema {
        /// Name of the missing/malformed field, or a short phrase
        /// describing the structural problem.
        field: &'static str,
    },
}

impl Trace {
    /// Load a samply trace. Accepts either `.json.gz` (the default
    /// samply output) or plain `.json`.
    pub fn load(path: &Path) -> Result<Self, TraceLoadError> {
        let json = read_json(path)?;
        Self::from_json(&json, path.to_path_buf())
    }

    /// Parse an in-memory JSON value (useful in tests).
    pub fn from_json(json: &Value, source_path: PathBuf) -> Result<Self, TraceLoadError> {
        let libs = libs_from(json)?;
        let threads = threads_from(json)?;
        let mut total_weight = 0_u64;
        for thread in &threads {
            validate_thread(thread, libs.len())?;
            for sample in &thread.samples {
                total_weight =
                    total_weight
                        .checked_add(sample.weight)
                        .ok_or(TraceLoadError::BadSchema {
                            field: "samples.weight total overflow",
                        })?;
            }
        }
        Ok(Self {
            libs,
            threads,
            source_path,
        })
    }
}

fn read_json(path: &Path) -> Result<Value, TraceLoadError> {
    let f = File::open(path).map_err(|source| TraceLoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let buf = BufReader::new(f);
    let parsed: Value = if path.extension().and_then(|e| e.to_str()) == Some("gz") {
        let gz = GzDecoder::new(buf);
        serde_json::from_reader(gz).map_err(|source| TraceLoadError::Json {
            path: path.to_path_buf(),
            source,
        })?
    } else {
        serde_json::from_reader(buf).map_err(|source| TraceLoadError::Json {
            path: path.to_path_buf(),
            source,
        })?
    };
    Ok(parsed)
}

fn libs_from(json: &Value) -> Result<Vec<Library>, TraceLoadError> {
    let arr = json
        .get("libs")
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: "libs" })?;
    arr.iter().map(library_from).collect()
}

fn library_from(v: &Value) -> Result<Library, TraceLoadError> {
    Ok(Library {
        name: required_string_at(v, "name", "libs[].name")?,
        path: required_string_at(v, "path", "libs[].path")?,
        debug_path: optional_string_at(v, "debugPath", "libs[].debugPath")?,
        debug_id: optional_string_alias_at(
            v,
            ("breakpadId", "libs[].breakpadId"),
            ("debugId", "libs[].debugId"),
        )?,
        code_id: optional_string_at(v, "codeId", "libs[].codeId")?,
    })
}

fn threads_from(json: &Value) -> Result<Vec<Thread>, TraceLoadError> {
    let arr = json
        .get("threads")
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: "threads" })?;
    arr.iter().map(thread_from).collect()
}

fn thread_from(v: &Value) -> Result<Thread, TraceLoadError> {
    let name = required_string_at(v, "name", "threads[].name")?;
    let tid = thread_id_at(v)?;
    let is_main = optional_bool_at(v, "isMainThread", "threads[].isMainThread", false)?;
    let string_array = string_array_from(v)?;
    let stack_table = stack_table_from(v)?;
    let frame_table = frame_table_from(v)?;
    let func_table = func_table_from(v)?;
    let resource_table = resource_table_from(v)?;
    let samples = samples_from(v)?;
    let resolved = vec![None; frame_table.len()];

    Ok(Thread {
        tid,
        name,
        is_main,
        samples,
        string_array,
        stack_table,
        frame_table,
        func_table,
        resource_table,
        resolved,
    })
}

fn validate_thread(thread: &Thread, lib_count: usize) -> Result<(), TraceLoadError> {
    validate_stack_prefixes(thread)?;
    if thread
        .stack_table
        .iter()
        .any(|row| row.frame_idx >= thread.frame_table.len())
    {
        return Err(TraceLoadError::BadSchema {
            field: "stackTable.frame index",
        });
    }
    if thread
        .frame_table
        .iter()
        .any(|row| row.func_idx >= thread.func_table.len())
    {
        return Err(TraceLoadError::BadSchema {
            field: "frameTable.func index",
        });
    }
    if thread
        .func_table
        .iter()
        .any(|row| row.name_idx >= thread.string_array.len())
    {
        return Err(TraceLoadError::BadSchema {
            field: "funcTable.name index",
        });
    }
    if thread.func_table.iter().any(|row| {
        row.resource_idx
            .is_some_and(|index| index >= thread.resource_table.len())
    }) {
        return Err(TraceLoadError::BadSchema {
            field: "funcTable.resource index",
        });
    }
    if thread
        .resource_table
        .iter()
        .any(|row| row.lib_idx.is_some_and(|index| index >= lib_count))
    {
        return Err(TraceLoadError::BadSchema {
            field: "resourceTable.lib index",
        });
    }
    if thread.samples.iter().any(|sample| {
        sample
            .stack_idx
            .is_some_and(|index| index >= thread.stack_table.len())
    }) {
        return Err(TraceLoadError::BadSchema {
            field: "samples.stack index",
        });
    }
    Ok(())
}

fn validate_stack_prefixes(thread: &Thread) -> Result<(), TraceLoadError> {
    if thread.stack_table.iter().any(|row| {
        row.prefix
            .is_some_and(|index| index >= thread.stack_table.len())
    }) {
        return Err(TraceLoadError::BadSchema {
            field: "stackTable.prefix index",
        });
    }
    let mut states = vec![0_u8; thread.stack_table.len()];
    for start in 0..thread.stack_table.len() {
        if states[start] != 0 {
            continue;
        }
        let mut path = Vec::new();
        let mut cursor = Some(start);
        while let Some(index) = cursor {
            match states[index] {
                0 => {
                    states[index] = 1;
                    path.push(index);
                    cursor = thread.stack_table[index].prefix;
                }
                1 => {
                    return Err(TraceLoadError::BadSchema {
                        field: "stackTable.prefix cycle",
                    });
                }
                _ => break,
            }
        }
        for index in path {
            states[index] = 2;
        }
    }
    Ok(())
}

fn string_array_from(v: &Value) -> Result<Vec<String>, TraceLoadError> {
    let arr = v
        .get("stringArray")
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema {
            field: "stringArray",
        })?;
    arr.iter()
        .map(|s| {
            s.as_str()
                .map(str::to_owned)
                .ok_or(TraceLoadError::BadSchema {
                    field: "stringArray entry",
                })
        })
        .collect()
}

fn stack_table_from(v: &Value) -> Result<Vec<StackEntry>, TraceLoadError> {
    let st = v.get("stackTable").ok_or(TraceLoadError::BadSchema {
        field: "stackTable",
    })?;
    let prefix = column_nullable_index(st, "prefix")?;
    let frames = column_usize(st, "frame")?;
    if prefix.len() != frames.len() {
        return Err(TraceLoadError::BadSchema {
            field: "stackTable column-length mismatch",
        });
    }
    Ok(prefix
        .into_iter()
        .zip(frames)
        .map(|(prefix, frame_idx)| StackEntry { prefix, frame_idx })
        .collect())
}

fn frame_table_from(v: &Value) -> Result<Vec<FrameRow>, TraceLoadError> {
    let ft = v.get("frameTable").ok_or(TraceLoadError::BadSchema {
        field: "frameTable",
    })?;
    let address = column_frame_address(ft, "address")?;
    let func = column_usize(ft, "func")?;
    if address.len() != func.len() {
        return Err(TraceLoadError::BadSchema {
            field: "frameTable column-length mismatch",
        });
    }
    Ok(address
        .into_iter()
        .zip(func)
        .map(|(address, func_idx)| FrameRow { address, func_idx })
        .collect())
}

fn func_table_from(v: &Value) -> Result<Vec<FuncRow>, TraceLoadError> {
    let ft = v
        .get("funcTable")
        .ok_or(TraceLoadError::BadSchema { field: "funcTable" })?;
    let names = column_usize(ft, "name")?;
    let resources = column_func_resource_index(ft, "resource")?;
    if names.len() != resources.len() {
        return Err(TraceLoadError::BadSchema {
            field: "funcTable column-length mismatch",
        });
    }
    Ok(names
        .into_iter()
        .zip(resources)
        .map(|(name_idx, resource_idx)| FuncRow {
            name_idx,
            resource_idx,
        })
        .collect())
}

fn resource_table_from(v: &Value) -> Result<Vec<ResourceRow>, TraceLoadError> {
    let rt = v.get("resourceTable").ok_or(TraceLoadError::BadSchema {
        field: "resourceTable",
    })?;
    let lib = column_nullable_index(rt, "lib")?;
    Ok(lib
        .into_iter()
        .map(|lib_idx| ResourceRow { lib_idx })
        .collect())
}

fn samples_from(v: &Value) -> Result<Vec<Sample>, TraceLoadError> {
    let s = v
        .get("samples")
        .ok_or(TraceLoadError::BadSchema { field: "samples" })?;
    let stacks = column_nullable_index(s, "stack")?;
    let n = stacks.len();
    let times = sample_times(s, n)?;
    let weights = column_u64_opt(s, "weight")?;
    if times.len() != n {
        return Err(TraceLoadError::BadSchema {
            field: "samples.time length mismatch",
        });
    }
    if weights.as_ref().is_some_and(|values| values.len() != n) {
        return Err(TraceLoadError::BadSchema {
            field: "samples.weight length mismatch",
        });
    }
    Ok((0..n)
        .map(|i| Sample {
            time_ms: times[i],
            stack_idx: stacks[i],
            weight: weights.as_ref().and_then(|values| values[i]).unwrap_or(1),
        })
        .collect())
}

/// Resolve per-sample timestamps across the two version-skewed shapes
/// samply emits:
///
/// - older builds carry `time` — absolute milliseconds per sample;
/// - newer builds carry `timeDeltas` — per-sample deltas whose running
///   prefix-sum reconstructs the absolute timeline.
///
/// `time` wins when both are present. When neither is (e.g. the empty
/// `samply` control thread), every sample gets `0.0`: timestamps are
/// not load-bearing for the sample-counting analyses (hot / inclusive /
/// rollup / libs / stacks / flame all weight by sample, never by time),
/// so a missing timeline is a degraded-but-usable trace, not a load
/// failure.
fn sample_times(s: &Value, n: usize) -> Result<Vec<f64>, TraceLoadError> {
    if s.get("time").is_some() {
        let times = column_f64_opt(s, "time")?;
        return Ok(times.into_iter().map(|t| t.unwrap_or(0.0)).collect());
    }
    if s.get("timeDeltas").is_some() {
        let deltas = column_f64_opt(s, "timeDeltas")?;
        let mut acc = 0.0;
        return Ok(deltas
            .into_iter()
            .map(|d| {
                acc += d.unwrap_or(0.0);
                acc
            })
            .collect());
    }
    Ok(vec![0.0; n])
}

// ---- column helpers -------------------------------------------------

fn required_string_at(v: &Value, key: &str, field: &'static str) -> Result<String, TraceLoadError> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(TraceLoadError::BadSchema { field })
}

fn optional_string_at(v: &Value, key: &str, field: &'static str) -> Result<String, TraceLoadError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(value) => value
            .as_str()
            .map(str::to_owned)
            .ok_or(TraceLoadError::BadSchema { field }),
    }
}

fn optional_string_alias_at(
    v: &Value,
    primary: (&str, &'static str),
    fallback: (&str, &'static str),
) -> Result<String, TraceLoadError> {
    match v.get(primary.0) {
        None | Some(Value::Null) => optional_string_at(v, fallback.0, fallback.1),
        Some(value) => value
            .as_str()
            .map(str::to_owned)
            .ok_or(TraceLoadError::BadSchema { field: primary.1 }),
    }
}

fn thread_id_at(v: &Value) -> Result<String, TraceLoadError> {
    match v.get("tid") {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(value) => {
            if let Some(id) = value.as_i64() {
                return Ok(id.to_string());
            }
            if let Some(id) = value.as_u64() {
                return Ok(id.to_string());
            }
            Err(TraceLoadError::BadSchema {
                field: "threads[].tid",
            })
        }
    }
}

fn optional_bool_at(
    v: &Value,
    key: &str,
    field: &'static str,
    default: bool,
) -> Result<bool, TraceLoadError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value.as_bool().ok_or(TraceLoadError::BadSchema { field }),
    }
}

fn column_usize(v: &Value, key: &'static str) -> Result<Vec<usize>, TraceLoadError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: key })?;
    arr.iter()
        .map(|x| {
            let value = x.as_u64().ok_or(TraceLoadError::BadSchema { field: key })?;
            usize::try_from(value).map_err(|_| TraceLoadError::BadSchema { field: key })
        })
        .collect()
}

fn column_nullable_index(
    v: &Value,
    key: &'static str,
) -> Result<Vec<Option<usize>>, TraceLoadError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: key })?;
    arr.iter()
        .map(|x| {
            if x.is_null() {
                return Ok(None);
            }
            let value = x.as_u64().ok_or(TraceLoadError::BadSchema { field: key })?;
            usize::try_from(value)
                .map(Some)
                .map_err(|_| TraceLoadError::BadSchema { field: key })
        })
        .collect()
}

fn column_func_resource_index(
    v: &Value,
    key: &'static str,
) -> Result<Vec<Option<usize>>, TraceLoadError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: key })?;
    arr.iter()
        .map(|x| {
            if x.is_null() || x.as_i64() == Some(-1) {
                return Ok(None);
            }
            let value = x.as_u64().ok_or(TraceLoadError::BadSchema { field: key })?;
            usize::try_from(value)
                .map(Some)
                .map_err(|_| TraceLoadError::BadSchema { field: key })
        })
        .collect()
}

fn column_frame_address(v: &Value, key: &'static str) -> Result<Vec<u64>, TraceLoadError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: key })?;
    arr.iter()
        .map(|x| match x.as_u64() {
            Some(value) => Ok(value),
            None if x.as_i64() == Some(-1) => Ok(0),
            None => Err(TraceLoadError::BadSchema { field: key }),
        })
        .collect()
}

fn column_u64_opt(
    v: &Value,
    key: &'static str,
) -> Result<Option<Vec<Option<u64>>>, TraceLoadError> {
    let Some(value) = v.get(key) else {
        return Ok(None);
    };
    let arr = value
        .as_array()
        .ok_or(TraceLoadError::BadSchema { field: key })?;
    let values = arr
        .iter()
        .map(|x| {
            if x.is_null() {
                Ok(None)
            } else {
                x.as_u64()
                    .map(Some)
                    .ok_or(TraceLoadError::BadSchema { field: key })
            }
        })
        .collect::<Result<_, _>>()?;
    Ok(Some(values))
}

fn column_f64_opt(v: &Value, key: &'static str) -> Result<Vec<Option<f64>>, TraceLoadError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: key })?;
    arr.iter()
        .map(|x| {
            if x.is_null() {
                Ok(None)
            } else {
                x.as_f64()
                    .map(Some)
                    .ok_or(TraceLoadError::BadSchema { field: key })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn trace_json(samples: &Value) -> Value {
        json!({
            "libs": [],
            "threads": [{
                "name": "t",
                "stringArray": ["leaf"],
                "stackTable": { "prefix": [null], "frame": [0] },
                "frameTable": { "address": [0], "func": [0] },
                "funcTable": { "name": [0], "resource": [0] },
                "resourceTable": { "lib": [null] },
                "samples": samples,
            }],
        })
    }

    fn trace_with_samples(samples: &Value) -> Trace {
        Trace::from_json(&trace_json(samples), PathBuf::from("test")).expect("load")
    }

    #[test]
    fn loads_legacy_absolute_time_column() {
        // Older samply: `time` holds absolute milliseconds per sample.
        let t = trace_with_samples(&json!({
            "stack": [0, 0, 0],
            "time": [0.0, 1.5, 3.0],
        }));
        let s = &t.threads[0].samples;
        assert_eq!(s.len(), 3);
        assert!(approx(s[2].time_ms, 3.0), "got {}", s[2].time_ms);
    }

    #[test]
    fn loads_timedeltas_via_prefix_sum() {
        // Current samply emits per-sample deltas, not absolute `time`;
        // the running prefix-sum reconstructs the absolute timeline.
        let t = trace_with_samples(&json!({
            "stack": [0, 0, 0],
            "timeDeltas": [1.0, 2.0, 0.5],
        }));
        let s = &t.threads[0].samples;
        assert_eq!(s.len(), 3);
        assert!(approx(s[0].time_ms, 1.0), "got {}", s[0].time_ms);
        assert!(approx(s[1].time_ms, 3.0), "got {}", s[1].time_ms);
        assert!(approx(s[2].time_ms, 3.5), "got {}", s[2].time_ms);
    }

    #[test]
    fn loads_when_no_timestamp_column_present() {
        // The empty `samply` control thread carries neither `time` nor
        // `timeDeltas`. A missing timeline degrades to zeros — timestamps
        // are not load-bearing for the sample-counting analyses — rather
        // than failing the whole trace load.
        let t = trace_with_samples(&json!({
            "stack": [0, 0],
        }));
        let s = &t.threads[0].samples;
        assert_eq!(s.len(), 2);
        assert!(s.iter().all(|x| approx(x.time_ms, 0.0)));
    }

    #[test]
    fn loads_fxprof_gecko_shape() {
        let json = json!({
            "libs": [{
                "name": "app",
                "path": "/bin/app",
                "debugName": "app",
                "debugPath": "/bin/app",
                "breakpadId": "AABBCCDD0",
                "codeId": null,
                "arch": null,
            }],
            "threads": [{
                "name": "app",
                "tid": "12345",
                "isMainThread": true,
                "stringArray": ["label", "0x10"],
                "stackTable": { "prefix": [null, 0], "frame": [0, 1] },
                "frameTable": { "address": [-1, 16], "func": [0, 1] },
                "funcTable": { "name": [0, 1], "resource": [-1, 0] },
                "resourceTable": { "lib": [0] },
                "samples": {
                    "stack": [0, 1],
                    "timeDeltas": [0.0, 1.0],
                    "weight": [1, 1],
                },
            }],
        });

        let trace = Trace::from_json(&json, PathBuf::from("test")).expect("load");
        assert_eq!(trace.libs[0].debug_id, "AABBCCDD0");
        assert_eq!(trace.threads[0].tid, "12345");
        assert_eq!(trace.threads[0].frame_table[0].address, 0);
        assert_eq!(trace.threads[0].func_table[0].resource_idx, None);
        assert_eq!(trace.threads[0].func_table[1].resource_idx, Some(0));
    }

    #[test]
    fn accepts_legacy_debug_id_alias() {
        let json = json!({
            "libs": [{
                "name": "app",
                "path": "/bin/app",
                "debugId": "LEGACY",
            }],
            "threads": [],
        });
        let trace = Trace::from_json(&json, PathBuf::from("test")).expect("load");
        assert_eq!(trace.libs[0].debug_id, "LEGACY");
    }

    #[test]
    fn rejects_non_numeric_required_table_entries() {
        let cases = [
            ("/threads/0/frameTable/address/0", json!("zero"), "address"),
            ("/threads/0/frameTable/func/0", json!("zero"), "func"),
            ("/threads/0/stringArray/0", json!(7), "stringArray entry"),
        ];
        for (pointer, value, expected_field) in cases {
            let mut json = trace_json(&json!({ "stack": [0] }));
            *json.pointer_mut(pointer).expect("fixture pointer") = value;
            let error = Trace::from_json(&json, PathBuf::from("test"))
                .expect_err("malformed required column entry must fail loading");
            assert!(
                matches!(error, TraceLoadError::BadSchema { field } if field == expected_field),
                "pointer {pointer}: {error}"
            );
        }
    }

    #[test]
    fn rejects_non_numeric_optional_table_entries() {
        let cases = [
            ("/threads/0/stackTable/prefix/0", "prefix"),
            ("/threads/0/funcTable/resource/0", "resource"),
            ("/threads/0/resourceTable/lib/0", "lib"),
            ("/threads/0/samples/stack/0", "stack"),
            ("/threads/0/samples/weight/0", "weight"),
            ("/threads/0/samples/time/0", "time"),
        ];
        for (pointer, expected_field) in cases {
            let mut json = trace_json(&json!({
                "stack": [0],
                "weight": [1],
                "time": [0.0],
            }));
            *json.pointer_mut(pointer).expect("fixture pointer") = json!("zero");
            let error = Trace::from_json(&json, PathBuf::from("test"))
                .expect_err("non-numeric optional column entry must fail loading");
            assert!(
                matches!(error, TraceLoadError::BadSchema { field } if field == expected_field),
                "pointer {pointer}: {error}"
            );
        }
    }

    #[test]
    fn accepts_null_optional_table_entries() {
        let mut json = trace_json(&json!({
            "stack": [null],
            "weight": [null],
            "time": [null],
        }));
        *json
            .pointer_mut("/threads/0/funcTable/resource/0")
            .expect("fixture pointer") = Value::Null;
        let trace = Trace::from_json(&json, PathBuf::from("test")).expect("load");
        let thread = &trace.threads[0];
        assert_eq!(thread.stack_table[0].prefix, None);
        assert_eq!(thread.func_table[0].resource_idx, None);
        assert_eq!(thread.resource_table[0].lib_idx, None);
        assert_eq!(thread.samples[0].stack_idx, None);
        assert_eq!(thread.samples[0].weight, 1);
        assert!(approx(thread.samples[0].time_ms, 0.0));
    }

    #[test]
    fn accepts_negative_one_func_resource_sentinel() {
        let mut json = trace_json(&json!({ "stack": [0] }));
        *json
            .pointer_mut("/threads/0/funcTable/resource/0")
            .expect("fixture pointer") = json!(-1);

        let trace = Trace::from_json(&json, PathBuf::from("test")).expect("load");
        let thread = &trace.threads[0];
        assert_eq!(thread.stack_table[0].prefix, None);
        assert_eq!(thread.func_table[0].resource_idx, None);
        assert_eq!(thread.resource_table[0].lib_idx, None);
        assert_eq!(thread.samples[0].stack_idx, Some(0));
    }

    #[test]
    fn rejects_negative_values_other_than_gecko_sentinels() {
        let cases = [
            ("/threads/0/frameTable/address/0", json!(-2), "address"),
            ("/threads/0/frameTable/func/0", json!(-1), "func"),
            ("/threads/0/stackTable/prefix/0", json!(-1), "prefix"),
            ("/threads/0/funcTable/resource/0", json!(-2), "resource"),
            ("/threads/0/resourceTable/lib/0", json!(-1), "lib"),
            ("/threads/0/samples/stack/0", json!(-1), "stack"),
            ("/threads/0/samples/weight/0", json!(-1), "weight"),
        ];
        for (pointer, value, expected_field) in cases {
            let mut json = trace_json(&json!({ "stack": [0], "weight": [1] }));
            *json.pointer_mut(pointer).expect("fixture pointer") = value;
            let error = Trace::from_json(&json, PathBuf::from("test"))
                .expect_err("unsupported negative value must fail loading");
            assert!(
                matches!(error, TraceLoadError::BadSchema { field } if field == expected_field),
                "pointer {pointer}: {error}"
            );
        }
    }

    #[test]
    fn rejects_stack_prefix_cycles() {
        let mut self_cycle = trace_json(&json!({ "stack": [0] }));
        *self_cycle
            .pointer_mut("/threads/0/stackTable/prefix/0")
            .expect("fixture pointer") = json!(0);

        let mut two_node_cycle = trace_json(&json!({ "stack": [0] }));
        *two_node_cycle
            .pointer_mut("/threads/0/stackTable/prefix")
            .expect("fixture pointer") = json!([1, 0]);
        *two_node_cycle
            .pointer_mut("/threads/0/stackTable/frame")
            .expect("fixture pointer") = json!([0, 0]);

        for json in [self_cycle, two_node_cycle] {
            let error = Trace::from_json(&json, PathBuf::from("test"))
                .expect_err("cyclic stack prefix chain must fail loading");
            assert!(matches!(
                error,
                TraceLoadError::BadSchema {
                    field: "stackTable.prefix cycle"
                }
            ));
        }
    }

    #[test]
    fn rejects_weight_column_length_mismatch() {
        for weight in [json!([]), json!([1, 1])] {
            let json = trace_json(&json!({ "stack": [0], "weight": weight }));
            let error = Trace::from_json(&json, PathBuf::from("test"))
                .expect_err("weight column length must match samples");
            assert!(matches!(
                error,
                TraceLoadError::BadSchema {
                    field: "samples.weight length mismatch"
                }
            ));
        }
    }

    #[test]
    fn rejects_weight_total_overflow_across_threads() {
        let mut json = trace_json(&json!({
            "stack": [0],
            "weight": [u64::MAX],
        }));
        let second = json["threads"][0].clone();
        json["threads"]
            .as_array_mut()
            .expect("threads array")
            .push(second);

        let error = Trace::from_json(&json, PathBuf::from("test"))
            .expect_err("aggregate sample weight must fit u64");
        assert!(matches!(
            error,
            TraceLoadError::BadSchema {
                field: "samples.weight total overflow"
            }
        ));
    }

    #[test]
    fn rejects_missing_or_malformed_required_names() {
        let cases = [
            (
                json!({
                    "libs": [{ "path": "/bin/app" }],
                    "threads": [],
                }),
                "libs[].name",
            ),
            (
                json!({
                    "libs": [{ "name": "app", "path": 7 }],
                    "threads": [],
                }),
                "libs[].path",
            ),
            (
                json!({
                    "libs": [{ "name": "app", "path": "/bin/app", "debugPath": 7 }],
                    "threads": [],
                }),
                "libs[].debugPath",
            ),
            (
                json!({
                    "libs": [{ "name": "app", "path": "/bin/app", "breakpadId": 7 }],
                    "threads": [],
                }),
                "libs[].breakpadId",
            ),
        ];
        for (json, expected_field) in cases {
            let error = Trace::from_json(&json, PathBuf::from("test"))
                .expect_err("malformed library metadata must fail loading");
            assert!(
                matches!(error, TraceLoadError::BadSchema { field } if field == expected_field),
                "{error}"
            );
        }

        let mut json = trace_json(&json!({ "stack": [0] }));
        json.pointer_mut("/threads/0")
            .and_then(Value::as_object_mut)
            .expect("fixture thread")
            .remove("name");
        let error = Trace::from_json(&json, PathBuf::from("test"))
            .expect_err("missing thread name must fail loading");
        assert!(matches!(
            error,
            TraceLoadError::BadSchema {
                field: "threads[].name"
            }
        ));
    }

    #[test]
    fn optional_thread_metadata_defaults_only_when_absent_or_null() {
        let mut absent = trace_json(&json!({ "stack": [0] }));
        let trace = Trace::from_json(&absent, PathBuf::from("test")).expect("load");
        assert_eq!(trace.threads[0].tid, "");
        assert!(!trace.threads[0].is_main);

        let thread = absent
            .pointer_mut("/threads/0")
            .and_then(Value::as_object_mut)
            .expect("fixture thread");
        thread.insert("tid".to_owned(), Value::Null);
        thread.insert("isMainThread".to_owned(), Value::Null);
        Trace::from_json(&absent, PathBuf::from("test")).expect("null metadata loads");

        for (value, expected) in [
            (json!(7), "7"),
            (json!(-7), "-7"),
            (json!("worker-1"), "worker-1"),
        ] {
            let mut json = trace_json(&json!({ "stack": [0] }));
            json.pointer_mut("/threads/0")
                .and_then(Value::as_object_mut)
                .expect("fixture thread")
                .insert("tid".to_owned(), value);
            let trace = Trace::from_json(&json, PathBuf::from("test")).expect("load tid");
            assert_eq!(trace.threads[0].tid, expected);
        }

        for (key, value, expected_field) in [
            ("tid", json!(true), "threads[].tid"),
            ("tid", json!(1.5), "threads[].tid"),
            ("isMainThread", json!("yes"), "threads[].isMainThread"),
        ] {
            let mut json = trace_json(&json!({ "stack": [0] }));
            json.pointer_mut("/threads/0")
                .and_then(Value::as_object_mut)
                .expect("fixture thread")
                .insert(key.to_owned(), value);
            let error = Trace::from_json(&json, PathBuf::from("test"))
                .expect_err("malformed thread metadata must fail loading");
            assert!(matches!(
                error,
                TraceLoadError::BadSchema { field } if field == expected_field
            ));
        }
    }

    #[test]
    fn rejects_every_out_of_range_table_reference() {
        let cases = [
            (
                "/threads/0/stackTable/prefix/0",
                json!(1),
                "stackTable.prefix index",
            ),
            (
                "/threads/0/stackTable/frame/0",
                json!(1),
                "stackTable.frame index",
            ),
            (
                "/threads/0/frameTable/func/0",
                json!(1),
                "frameTable.func index",
            ),
            (
                "/threads/0/funcTable/name/0",
                json!(1),
                "funcTable.name index",
            ),
            (
                "/threads/0/funcTable/resource/0",
                json!(1),
                "funcTable.resource index",
            ),
            (
                "/threads/0/resourceTable/lib/0",
                json!(0),
                "resourceTable.lib index",
            ),
            (
                "/threads/0/samples/stack/0",
                json!(1),
                "samples.stack index",
            ),
        ];
        for (pointer, value, expected_field) in cases {
            let mut json = trace_json(&json!({ "stack": [0] }));
            *json.pointer_mut(pointer).expect("fixture pointer") = value;
            let error = Trace::from_json(&json, PathBuf::from("test"))
                .expect_err("out-of-range table reference must fail loading");
            assert!(
                matches!(error, TraceLoadError::BadSchema { field } if field == expected_field),
                "pointer {pointer}: {error}"
            );
        }
    }
}
