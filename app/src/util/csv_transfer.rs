//! Shared CSV transfer core for the admin import/export.
//!
//! Owns the byte-level concerns every entity's CSV service
//! shares: decoding an uploaded file (UTF-8, UTF-8 with BOM,
//! or Shift_JIS — the three encodings Japanese EC platforms
//! exchange), parsing it with the `csv` crate (descriptions
//! carry commas, quotes and newlines, so splitting by hand
//! would corrupt them), writing an export file (UTF-8 with
//! BOM, so Excel opens it without mojibake), and the report
//! types serialised into a batch row's JSON.

use serde::{Deserialize, Serialize};

/// One problem found while parsing or validating an upload,
/// pinned to the file line it came from (1-based; the header
/// is line 1). Shown verbatim on the preview page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineError {
    /// File line the problem sits on.
    pub line: usize,
    /// Operator-facing message (Japanese).
    pub message: String,
}

impl LineError {
    /// Builds one line error.
    pub fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

/// The validation outcome of one uploaded batch: how many
/// records would be created, how many updated, and every
/// problem found. Stored as JSON on the batch row and read
/// back for the preview page; counts exclude errored records
/// (the unit of application is one whole record — see the
/// entity services).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatchReport {
    /// Records the apply step would create.
    pub new_count: usize,
    /// Records the apply step would update.
    pub update_count: usize,
    /// Every problem found, in file order.
    pub errors: Vec<LineError>,
}

impl BatchReport {
    /// How many problems were found.
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    /// Serialises the report for the batch row's JSON column.
    /// Infallible in practice (plain data only); a failure
    /// degrades to an empty object rather than failing the
    /// upload.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Parses a stored report back; a corrupt value reads as
    /// an empty report (the preview then shows no counts,
    /// never invented ones).
    pub fn from_json(raw: &str) -> Self {
        serde_json::from_str(raw).unwrap_or_default()
    }
}

/// One parsed data row: its cells plus the file line it
/// started on (a quoted field can span lines, so the row's
/// line is where the record begins).
#[derive(Debug, Clone)]
pub struct CsvRow {
    /// 1-based file line of the record's first line.
    pub line: usize,
    /// The record's cells, in column order.
    pub cells: Vec<String>,
}

impl CsvRow {
    /// Returns one cell by column index; a short row reads
    /// missing trailing cells as empty, matching how the
    /// platforms pad their exports.
    pub fn get(&self, index: usize) -> &str {
        self.cells.get(index).map(String::as_str).unwrap_or("")
    }
}

/// Decodes an uploaded CSV file's bytes to text. A UTF-8 BOM
/// is stripped; strict UTF-8 is tried next; anything else is
/// decoded as Shift_JIS (encoding_rs replaces an undecodable
/// byte rather than failing — a mangled file then fails the
/// header check downstream, which names the real problem).
pub fn decode_bytes(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    encoding_rs::SHIFT_JIS.decode(bytes).0.into_owned()
}

/// Parses CSV text into data rows, checking the header row
/// against `expected` exactly (order and spelling): a file
/// for the wrong entity — or a hand-edited header — is one
/// clear line-1 error, not a cascade of cell errors. Blank
/// lines are skipped, as the platforms emit them.
pub fn parse_rows(text: &str, expected: &[&str]) -> Result<Vec<CsvRow>, LineError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(text.as_bytes());
    let header = reader
        .headers()
        .map_err(|e| LineError::new(1, format!("CSVのヘッダーを読み取れませんでした: {e}")))?;
    let actual: Vec<&str> = header.iter().collect();
    if actual != expected {
        return Err(LineError::new(
            1,
            format!("ヘッダーが一致しません（想定: {}）", expected.join("、")),
        ));
    }
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| {
            // The csv crate knows where the read failed; the
            // old code blamed line 1 for every error.
            let line = e.position().map(|pos| pos.line() as usize).unwrap_or(1);
            LineError::new(line, format!("CSVを読み取れませんでした: {e}"))
        })?;
        if record.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        let line = record.position().map(|p| p.line() as usize).unwrap_or(0);
        rows.push(CsvRow {
            line,
            cells: record.iter().map(str::to_string).collect(),
        });
    }
    Ok(rows)
}

/// Renders an export file: UTF-8 with BOM, one header row
/// plus the given data rows, CRLF line endings (the csv
/// crate's default, and what Excel expects).
pub fn write_csv(headers: &[&str], rows: &[Vec<String>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 + rows.len() * 64);
    out.extend_from_slice(b"\xEF\xBB\xBF");
    out.extend_from_slice(write_csv_text(headers, rows).as_bytes());
    out
}

/// Renders CSV text without a BOM: the same writer and
/// formula escaping as [`write_csv`], for callers that encode
/// the text themselves (the carrier label files are
/// Shift_JIS, which has no BOM to give).
pub fn write_csv_text(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut buf = Vec::with_capacity(rows.len() * 64);
    {
        let mut writer = csv::Writer::from_writer(&mut buf);
        // Writing to an in-memory Vec cannot fail; the expect
        // documents that invariant rather than handling an error.
        writer
            .write_record(headers.iter().copied())
            .expect("in-memory CSV header write");
        for row in rows {
            writer
                .write_record(row.iter().map(|cell| escape_formula(cell)))
                .expect("in-memory CSV row write");
        }
        writer.flush().expect("in-memory CSV flush");
    }
    // The writer only ever emits the caller's UTF-8 strings
    // plus ASCII punctuation.
    String::from_utf8(buf).expect("in-memory CSV is UTF-8")
}

/// Neutralises spreadsheet formula injection (SEC-10): a
/// cell whose first character would start a formula in a
/// spreadsheet app (`=`, `+`, `-`, `@`, tab, CR) is prefixed
/// with a single quote, which spreadsheet apps treat as a
/// literal-text marker. Applied to every exported cell in
/// this one place, so all exporters inherit it.
fn escape_formula(cell: &str) -> String {
    match cell.chars().next() {
        Some('=' | '+' | '-' | '@' | '\t' | '\r') => format!("'{cell}"),
        _ => cell.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_leading_cells_are_quoted() {
        assert_eq!(escape_formula("=1+1"), "'=1+1");
        assert_eq!(escape_formula("+SUM(A1:A2)"), "'+SUM(A1:A2)");
        assert_eq!(escape_formula("-2+3"), "'-2+3");
        assert_eq!(escape_formula("@cmd"), "'@cmd");
        assert_eq!(escape_formula("Rust T"), "Rust T");
        assert_eq!(escape_formula(""), "");
        assert_eq!(escape_formula("a=b"), "a=b");
    }

    #[test]
    fn write_csv_quotes_formula_cells() {
        let out = write_csv(&["name"], &[vec!["=cmd".to_string()]]);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("'=cmd"), "got: {text}");
    }

    #[test]
    fn decodes_utf8_with_and_without_bom() {
        assert_eq!(decode_bytes("商品名".as_bytes()), "商品名");
        let mut bom = b"\xEF\xBB\xBF".to_vec();
        bom.extend_from_slice("商品名".as_bytes());
        assert_eq!(decode_bytes(&bom), "商品名");
    }

    #[test]
    fn decodes_shift_jis() {
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode("商品名、価格");
        assert_eq!(decode_bytes(&bytes), "商品名、価格");
    }

    #[test]
    fn parses_quoted_commas_and_embedded_newlines() {
        let text = "名前,説明\n\"シャツ\",\"綿100%, 白\n大きいサイズ\"\n";
        let rows = parse_rows(text, &["名前", "説明"]).expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].line, 2);
        assert_eq!(rows[0].get(0), "シャツ");
        assert_eq!(rows[0].get(1), "綿100%, 白\n大きいサイズ");
    }

    #[test]
    fn rejects_a_mismatched_header() {
        let err = parse_rows("名前,価格\nx,1\n", &["名前", "説明"]).expect_err("header");
        assert_eq!(err.line, 1);
    }

    #[test]
    fn writes_bom_and_round_trips() {
        let bytes = write_csv(&["名前", "説明"], &[vec!["a,b".into(), "x\ny".into()]]);
        assert!(bytes.starts_with(b"\xEF\xBB\xBF"));
        let text = decode_bytes(&bytes);
        let rows = parse_rows(&text, &["名前", "説明"]).expect("reparse");
        assert_eq!(rows[0].get(0), "a,b");
        assert_eq!(rows[0].get(1), "x\ny");
    }
}
