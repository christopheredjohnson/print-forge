//! Dataset loading and normalization.

use std::{
    fmt,
    fs::File,
    io::{self, Read},
    ops::ControlFlow,
    path::{Path, PathBuf},
};

use serde::de::{Deserializer, Error as _, SeqAccess, Visitor};
use serde_json::{Map, Value};
use thiserror::Error;

pub type DataRow = Map<String, Value>;

/// The supported on-disk dataset encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetFormat {
    Csv,
    Json,
}

impl DatasetFormat {
    /// Infers a dataset format from a case-insensitive file extension.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, DatasetError> {
        let path = path.as_ref();
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("csv") => Ok(Self::Csv),
            Some("json") => Ok(Self::Json),
            _ => Err(DatasetError::UnsupportedFormat {
                path: path.to_owned(),
            }),
        }
    }
}

/// A recoverable dataset loading or decoding failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DatasetError {
    #[error("failed to open dataset {path}")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read CSV dataset: {0}")]
    Csv(#[from] csv::Error),
    #[error("failed to read JSON dataset: {0}")]
    Json(#[from] serde_json::Error),
    #[error("dataset must have a .csv or .json extension: {path}")]
    UnsupportedFormat { path: PathBuf },
}

/// Describes whether a streaming row visitor consumed the complete dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisitOutcome {
    Complete { rows: usize },
    Stopped { rows: usize },
}

impl VisitOutcome {
    #[must_use]
    pub const fn rows(self) -> usize {
        match self {
            Self::Complete { rows } | Self::Stopped { rows } => rows,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dataset {
    pub rows: Vec<DataRow>,
}

impl Dataset {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, DatasetError> {
        let path = path.as_ref();
        match DatasetFormat::from_path(path)? {
            DatasetFormat::Csv => Self::from_csv(path),
            DatasetFormat::Json => Self::from_json(path),
        }
    }

    pub fn from_csv(path: impl AsRef<Path>) -> Result<Self, DatasetError> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|source| DatasetError::Open {
            path: path.to_owned(),
            source,
        })?;

        Self::from_csv_reader(file)
    }

    pub fn from_csv_reader(reader: impl Read) -> Result<Self, DatasetError> {
        let mut rows = Vec::new();
        visit_csv_reader(reader, |_, row| {
            rows.push(row);
            ControlFlow::Continue(())
        })?;
        Ok(Self { rows })
    }

    pub fn from_json(path: impl AsRef<Path>) -> Result<Self, DatasetError> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|source| DatasetError::Open {
            path: path.to_owned(),
            source,
        })?;

        Self::from_json_reader(file)
    }

    pub fn from_json_reader(reader: impl Read) -> Result<Self, DatasetError> {
        let mut rows = Vec::new();
        visit_json_reader(reader, |_, row| {
            rows.push(row);
            ControlFlow::Continue(())
        })?;
        Ok(Self { rows })
    }
}

/// Visits rows without retaining the complete dataset in memory.
pub fn visit_path<F>(path: impl AsRef<Path>, visitor: F) -> Result<VisitOutcome, DatasetError>
where
    F: FnMut(usize, DataRow) -> ControlFlow<()>,
{
    let path = path.as_ref();
    let format = DatasetFormat::from_path(path)?;
    let file = File::open(path).map_err(|source| DatasetError::Open {
        path: path.to_owned(),
        source,
    })?;
    match format {
        DatasetFormat::Csv => visit_csv_reader(file, visitor),
        DatasetFormat::Json => visit_json_reader(file, visitor),
    }
}

/// Streams CSV rows through `visitor`.
pub fn visit_csv_reader<F>(reader: impl Read, mut visitor: F) -> Result<VisitOutcome, DatasetError>
where
    F: FnMut(usize, DataRow) -> ControlFlow<()>,
{
    let mut reader = csv::Reader::from_reader(reader);
    let headers = reader.headers()?.clone();
    let mut rows = 0;
    for record in reader.records() {
        let record = record?;
        let row = headers
            .iter()
            .zip(record.iter())
            .map(|(key, value)| (key.to_owned(), Value::String(value.to_owned())))
            .collect();
        rows += 1;
        if visitor(rows - 1, row).is_break() {
            return Ok(VisitOutcome::Stopped { rows });
        }
    }
    Ok(VisitOutcome::Complete { rows })
}

/// Streams members of a top-level JSON array through `visitor`.
pub fn visit_json_reader<F>(reader: impl Read, mut visitor: F) -> Result<VisitOutcome, DatasetError>
where
    F: FnMut(usize, DataRow) -> ControlFlow<()>,
{
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let outcome = deserializer.deserialize_seq(JsonRowsVisitor {
        visitor: &mut visitor,
    })?;
    deserializer.end()?;
    Ok(outcome)
}

struct JsonRowsVisitor<'a, F> {
    visitor: &'a mut F,
}

impl<'de, F> Visitor<'de> for JsonRowsVisitor<'_, F>
where
    F: FnMut(usize, DataRow) -> ControlFlow<()>,
{
    type Value = VisitOutcome;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON array of objects")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut rows = 0;
        while let Some(value) = sequence.next_element::<Value>()? {
            let Value::Object(row) = value else {
                return Err(A::Error::custom(format!(
                    "JSON dataset row {rows} must be an object"
                )));
            };
            rows += 1;
            if (self.visitor)(rows - 1, row).is_break() {
                while sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                return Ok(VisitOutcome::Stopped { rows });
            }
        }
        Ok(VisitOutcome::Complete { rows })
    }
}

#[cfg(test)]
mod tests {
    use std::ops::ControlFlow;

    use serde_json::json;

    use super::{Dataset, DatasetError, VisitOutcome, visit_csv_reader, visit_json_reader};

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

    #[test]
    fn streams_csv_rows_and_can_stop_early() {
        let mut names = Vec::new();
        let outcome = visit_csv_reader("name\nAda\nGrace\nLinus\n".as_bytes(), |_, row| {
            names.push(row["name"].as_str().unwrap().to_owned());
            if names.len() == 2 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .unwrap();

        assert_eq!(outcome, VisitOutcome::Stopped { rows: 2 });
        assert_eq!(names, ["Ada", "Grace"]);
    }

    #[test]
    fn streams_json_array_rows() {
        let mut indices = Vec::new();
        let outcome = visit_json_reader(r#"[{"id":1},{"id":2}]"#.as_bytes(), |index, _| {
            indices.push(index);
            ControlFlow::Continue(())
        })
        .unwrap();

        assert_eq!(outcome, VisitOutcome::Complete { rows: 2 });
        assert_eq!(indices, [0, 1]);
    }

    #[test]
    fn reports_non_object_json_rows_with_a_typed_error() {
        let error = Dataset::from_json_reader("[1]".as_bytes()).unwrap_err();
        assert!(matches!(error, DatasetError::Json(_)));
        assert!(error.to_string().contains("row 0 must be an object"));
    }
}
