//! Dataset loading and normalization.

use std::{fs::File, io::Read, path::Path};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

pub type DataRow = Map<String, Value>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dataset {
    pub rows: Vec<DataRow>,
}

impl Dataset {
    pub fn from_csv(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)
            .with_context(|| format!("failed to open CSV dataset {}", path.display()))?;

        Self::from_csv_reader(file)
            .with_context(|| format!("failed to parse CSV dataset {}", path.display()))
    }

    pub fn from_csv_reader(reader: impl Read) -> Result<Self> {
        let mut reader = csv::Reader::from_reader(reader);
        let headers = reader.headers()?.clone();
        let mut rows = Vec::new();

        for record in reader.records() {
            let record = record?;
            let row = headers
                .iter()
                .zip(record.iter())
                .map(|(key, value)| (key.to_owned(), Value::String(value.to_owned())))
                .collect();
            rows.push(row);
        }

        Ok(Self { rows })
    }

    pub fn from_json(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)
            .with_context(|| format!("failed to open JSON dataset {}", path.display()))?;

        Self::from_json_reader(file)
            .with_context(|| format!("failed to parse JSON dataset {}", path.display()))
    }

    pub fn from_json_reader(reader: impl Read) -> Result<Self> {
        let value: Value = serde_json::from_reader(reader)?;
        let Value::Array(values) = value else {
            bail!("a JSON dataset must be an array of objects");
        };

        let rows = values
            .into_iter()
            .enumerate()
            .map(|(index, value)| match value {
                Value::Object(row) => Ok(row),
                _ => bail!("JSON dataset row {index} must be an object"),
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self { rows })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Dataset;

    #[test]
    fn reads_csv_rows_as_json_objects() {
        let dataset = Dataset::from_csv_reader("name,title\nAda,Engineer\n".as_bytes()).unwrap();

        assert_eq!(dataset.rows.len(), 1);
        assert_eq!(dataset.rows[0]["name"], json!("Ada"));
    }

    #[test]
    fn reads_nested_json_values() {
        let dataset = Dataset::from_json_reader(
            r#"[{"invoice":"INV-1","items":[{"description":"Signs"}]}]"#.as_bytes(),
        )
        .unwrap();

        assert!(dataset.rows[0]["items"].is_array());
    }
}
