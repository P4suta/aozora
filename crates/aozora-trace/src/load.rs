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
    #[error("io error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("json parse error in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("gecko schema: missing or malformed `{field}`")]
    BadSchema { field: &'static str },
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
    Ok(arr.iter().map(library_from).collect())
}

fn library_from(v: &Value) -> Library {
    Library {
        name: string_at(v, "name"),
        path: string_at(v, "path"),
        debug_path: string_at(v, "debugPath"),
        debug_id: string_at(v, "debugId"),
        code_id: string_at(v, "codeId"),
    }
}

fn threads_from(json: &Value) -> Result<Vec<Thread>, TraceLoadError> {
    let arr = json
        .get("threads")
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: "threads" })?;
    arr.iter().map(thread_from).collect()
}

fn thread_from(v: &Value) -> Result<Thread, TraceLoadError> {
    let string_array = string_array_from(v)?;
    let stack_table = stack_table_from(v)?;
    let frame_table = frame_table_from(v)?;
    let func_table = func_table_from(v)?;
    let resource_table = resource_table_from(v)?;
    let samples = samples_from(v)?;
    let resolved = vec![None; frame_table.len()];

    Ok(Thread {
        tid: v.get("tid").and_then(Value::as_i64).unwrap_or(0),
        name: string_at(v, "name"),
        is_main: v
            .get("isMainThread")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        samples,
        string_array,
        stack_table,
        frame_table,
        func_table,
        resource_table,
        resolved,
    })
}

fn string_array_from(v: &Value) -> Result<Vec<String>, TraceLoadError> {
    let arr = v
        .get("stringArray")
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema {
            field: "stringArray",
        })?;
    Ok(arr
        .iter()
        .map(|s| s.as_str().unwrap_or("").to_owned())
        .collect())
}

fn stack_table_from(v: &Value) -> Result<Vec<StackEntry>, TraceLoadError> {
    let st = v.get("stackTable").ok_or(TraceLoadError::BadSchema {
        field: "stackTable",
    })?;
    let prefix = column_usize_opt(st, "prefix")?;
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
    let address = column_u64(ft, "address")?;
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
    let resources = column_usize_opt(ft, "resource")?;
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
    let lib = column_usize_opt(rt, "lib")?;
    Ok(lib
        .into_iter()
        .map(|lib_idx| ResourceRow { lib_idx })
        .collect())
}

fn samples_from(v: &Value) -> Result<Vec<Sample>, TraceLoadError> {
    let s = v
        .get("samples")
        .ok_or(TraceLoadError::BadSchema { field: "samples" })?;
    let stacks = column_usize_opt(s, "stack")?;
    let times = column_f64_opt(s, "time")?;
    let weights = column_u64_opt(s, "weight");
    let n = stacks.len();
    if times.len() != n {
        return Err(TraceLoadError::BadSchema {
            field: "samples.time length mismatch",
        });
    }
    Ok((0..n)
        .map(|i| Sample {
            time_ms: times[i].unwrap_or(0.0),
            stack_idx: stacks[i],
            weight: weights.get(i).copied().flatten().unwrap_or(1),
        })
        .collect())
}

// ---- column helpers -------------------------------------------------

fn string_at(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

fn column_usize(v: &Value, key: &'static str) -> Result<Vec<usize>, TraceLoadError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: key })?;
    Ok(arr
        .iter()
        .map(|x| x.as_u64().map_or(0, |n| n as usize))
        .collect())
}

fn column_usize_opt(v: &Value, key: &'static str) -> Result<Vec<Option<usize>>, TraceLoadError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: key })?;
    Ok(arr
        .iter()
        .map(|x| {
            if x.is_null() {
                None
            } else {
                x.as_u64().map(|n| n as usize)
            }
        })
        .collect())
}

fn column_u64(v: &Value, key: &'static str) -> Result<Vec<u64>, TraceLoadError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: key })?;
    Ok(arr.iter().map(|x| x.as_u64().unwrap_or(0)).collect())
}

fn column_u64_opt(v: &Value, key: &'static str) -> Vec<Option<u64>> {
    // The weight column is optional altogether — callers fall back
    // to all-1s when it's absent. Returning a plain Vec lets the
    // call site stay shape-symmetric with the other column helpers
    // without paying for a Result that can never error.
    let Some(arr) = v.get(key).and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .map(|x| if x.is_null() { None } else { x.as_u64() })
        .collect()
}

fn column_f64_opt(v: &Value, key: &'static str) -> Result<Vec<Option<f64>>, TraceLoadError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or(TraceLoadError::BadSchema { field: key })?;
    Ok(arr
        .iter()
        .map(|x| if x.is_null() { None } else { x.as_f64() })
        .collect())
}
