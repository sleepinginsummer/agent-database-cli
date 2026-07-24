use crate::types::OutputFormat;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

pub fn write_output<T: Serialize>(data: &T, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string(data)?);
        }
        OutputFormat::Table => {
            println!("{}", format_table(&serde_json::to_value(data)?));
        }
        OutputFormat::Compact => {
            println!("{}", format_compact(&serde_json::to_value(data)?)?);
        }
    }
    Ok(())
}

/// Render rows as compact array-of-rows JSON: column names are listed once in
/// `fields`, and each row becomes a positional array aligned to that order.
/// Stays valid, machine-parseable JSON (escaping and number/string/null types
/// preserved) while dropping the per-row key repetition of keyed JSON.
fn format_compact(data: &Value) -> Result<String> {
    let rows = if let Some(rows) = data.get("rows").and_then(Value::as_array) {
        rows.clone()
    } else if let Some(rows) = data.as_array() {
        rows.clone()
    } else {
        vec![data.clone()]
    };

    // Prefer the explicit `fields` order returned by the adapter (it matches the
    // database's column order); fall back to the union of keys across rows.
    let columns: Vec<String> = match data.get("fields").and_then(Value::as_array) {
        Some(fields) => fields
            .iter()
            .filter_map(|f| f.as_str().map(str::to_string))
            .collect(),
        None => {
            let mut cols: Vec<String> = Vec::new();
            for row in &rows {
                if let Some(obj) = row.as_object() {
                    for key in obj.keys() {
                        if !cols.contains(key) {
                            cols.push(key.clone());
                        }
                    }
                }
            }
            cols
        }
    };

    let compact_rows: Vec<Value> = rows
        .iter()
        .map(|row| match row.as_object() {
            Some(obj) => Value::Array(
                columns
                    .iter()
                    .map(|c| obj.get(c).cloned().unwrap_or(Value::Null))
                    .collect(),
            ),
            // Scalar rows (e.g. Redis/Mongo) have no columns; emit them as-is.
            None => row.clone(),
        })
        .collect();

    let mut out = serde_json::Map::new();
    out.insert(
        "fields".to_string(),
        Value::Array(columns.into_iter().map(Value::String).collect()),
    );
    out.insert("rows".to_string(), Value::Array(compact_rows));
    if let Some(row_count) = data.get("rowCount") {
        out.insert("rowCount".to_string(), row_count.clone());
    }
    Ok(serde_json::to_string(&Value::Object(out))?)
}

fn format_table(data: &Value) -> String {
    let rows = if let Some(rows) = data.get("rows").and_then(Value::as_array) {
        rows.clone()
    } else if let Some(rows) = data.as_array() {
        rows.clone()
    } else {
        vec![data.clone()]
    };
    if rows.is_empty() {
        return String::new();
    }
    let objects: Vec<serde_json::Map<String, Value>> = rows
        .into_iter()
        .map(|row| {
            row.as_object()
                .cloned()
                .unwrap_or_else(|| serde_json::Map::from_iter([("value".to_string(), row)]))
        })
        .collect();
    let mut columns: Vec<String> = Vec::new();
    for row in &objects {
        for key in row.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }
    let widths: Vec<usize> = columns
        .iter()
        .map(|column| {
            let cell_width = objects
                .iter()
                .map(|row| stringify_cell(row.get(column)).len())
                .max()
                .unwrap_or(0);
            column.len().max(cell_width)
        })
        .collect();
    let header = columns
        .iter()
        .enumerate()
        .map(|(i, c)| pad(c, widths[i]))
        .collect::<Vec<_>>()
        .join("  ");
    let divider = widths
        .iter()
        .map(|w| "-".repeat(*w))
        .collect::<Vec<_>>()
        .join("  ");
    let body = objects
        .iter()
        .map(|row| {
            columns
                .iter()
                .enumerate()
                .map(|(i, c)| pad(&stringify_cell(row.get(c)), widths[i]))
                .collect::<Vec<_>>()
                .join("  ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    [header, divider, body].join("\n")
}

fn stringify_cell(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(value) => value.to_string(),
    }
}

fn pad(value: &str, width: usize) -> String {
    format!("{value:<width$}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compact_uses_fields_order_and_positional_rows() {
        let data = json!({
            "fields": ["id", "name", "email"],
            "rows": [
                {"id": 1, "name": "Alice", "email": "a@x.com"},
                {"id": 2, "name": "Bob", "email": "b@x.com"}
            ],
            "rowCount": 2
        });
        let out = format_compact(&data).unwrap();
        assert_eq!(
            out,
            r#"{"fields":["id","name","email"],"rows":[[1,"Alice","a@x.com"],[2,"Bob","b@x.com"]],"rowCount":2}"#
        );
    }

    #[test]
    fn compact_output_is_smaller_than_minified_json() {
        let data = json!({
            "fields": ["id", "name"],
            "rows": [{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}]
        });
        let compact = format_compact(&data).unwrap();
        let minified = serde_json::to_string(&data).unwrap();
        assert!(compact.len() < minified.len());
    }

    #[test]
    fn compact_preserves_types_escaping_and_nulls() {
        let data = json!({
            "fields": ["n", "s", "missing"],
            "rows": [{"n": 42, "s": "a,\"b\"\nc"}]
        });
        let out = format_compact(&data).unwrap();
        // number stays a number, string is JSON-escaped, absent column -> null
        assert_eq!(out, r#"{"fields":["n","s","missing"],"rows":[[42,"a,\"b\"\nc",null]]}"#);
    }

    #[test]
    fn compact_derives_columns_when_fields_absent() {
        let data = json!({
            "rows": [{"a": 1, "b": 2}, {"a": 3, "c": 4}]
        });
        let out = format_compact(&data).unwrap();
        assert_eq!(out, r#"{"fields":["a","b","c"],"rows":[[1,2,null],[3,null,4]]}"#);
    }

    #[test]
    fn compact_handles_empty_rows() {
        let data = json!({"fields": ["id"], "rows": []});
        let out = format_compact(&data).unwrap();
        assert_eq!(out, r#"{"fields":["id"],"rows":[]}"#);
    }
}
