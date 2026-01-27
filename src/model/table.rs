//! Table types.

use super::{Alignment, Paragraph};
use serde::{Deserialize, Serialize};

/// A table structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    /// Rows in the table
    pub rows: Vec<TableRow>,

    /// Number of header rows (0 = no header)
    pub header_rows: u8,

    /// Column widths in points (optional)
    pub column_widths: Option<Vec<f32>>,

    /// Table caption
    pub caption: Option<String>,
}

impl Table {
    /// Create a new empty table.
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            header_rows: 0,
            column_widths: None,
            caption: None,
        }
    }

    /// Create a table with header.
    pub fn with_header(header_rows: u8) -> Self {
        Self {
            header_rows,
            ..Self::new()
        }
    }

    /// Add a row to the table.
    pub fn add_row(&mut self, row: TableRow) {
        self.rows.push(row);
    }

    /// Get the number of rows.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Get the number of columns (based on first row).
    pub fn column_count(&self) -> usize {
        self.rows.first().map(|r| r.cells.len()).unwrap_or(0)
    }

    /// Check if the table is empty.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Get header rows.
    pub fn header(&self) -> &[TableRow] {
        &self.rows[..self.header_rows as usize]
    }

    /// Get body rows (non-header).
    pub fn body(&self) -> &[TableRow] {
        &self.rows[self.header_rows as usize..]
    }

    /// Get plain text representation of the table.
    pub fn plain_text(&self) -> String {
        self.rows
            .iter()
            .map(|row| row.plain_text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Check if the table has complex structure (merged cells).
    pub fn has_merged_cells(&self) -> bool {
        self.rows
            .iter()
            .flat_map(|r| &r.cells)
            .any(|c| c.rowspan > 1 || c.colspan > 1)
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

/// A table row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRow {
    /// Cells in the row
    pub cells: Vec<TableCell>,

    /// Whether this is a header row
    pub is_header: bool,
}

impl TableRow {
    /// Create a new row with cells.
    pub fn new(cells: Vec<TableCell>) -> Self {
        Self {
            cells,
            is_header: false,
        }
    }

    /// Create a header row.
    pub fn header(cells: Vec<TableCell>) -> Self {
        Self {
            cells,
            is_header: true,
        }
    }

    /// Create a row from text values.
    pub fn from_strings<S: Into<String>>(values: impl IntoIterator<Item = S>) -> Self {
        Self::new(values.into_iter().map(TableCell::text).collect())
    }

    /// Get plain text representation.
    pub fn plain_text(&self) -> String {
        self.cells
            .iter()
            .map(|c| c.plain_text())
            .collect::<Vec<_>>()
            .join("\t")
    }
}

/// A table cell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableCell {
    /// Cell content (paragraphs)
    pub content: Vec<Paragraph>,

    /// Number of rows this cell spans
    pub rowspan: u8,

    /// Number of columns this cell spans
    pub colspan: u8,

    /// Cell alignment
    pub alignment: Alignment,

    /// Vertical alignment
    pub vertical_alignment: VerticalAlignment,
}

impl TableCell {
    /// Create a new cell with text content.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![Paragraph::with_text(text)],
            rowspan: 1,
            colspan: 1,
            alignment: Alignment::Left,
            vertical_alignment: VerticalAlignment::Top,
        }
    }

    /// Create an empty cell.
    pub fn empty() -> Self {
        Self {
            content: Vec::new(),
            rowspan: 1,
            colspan: 1,
            alignment: Alignment::Left,
            vertical_alignment: VerticalAlignment::Top,
        }
    }

    /// Create a cell with multiple paragraphs.
    pub fn with_content(content: Vec<Paragraph>) -> Self {
        Self {
            content,
            rowspan: 1,
            colspan: 1,
            alignment: Alignment::Left,
            vertical_alignment: VerticalAlignment::Top,
        }
    }

    /// Set colspan and return self.
    pub fn colspan(mut self, span: u8) -> Self {
        self.colspan = span;
        self
    }

    /// Set rowspan and return self.
    pub fn rowspan(mut self, span: u8) -> Self {
        self.rowspan = span;
        self
    }

    /// Set alignment and return self.
    pub fn align(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Get plain text content.
    pub fn plain_text(&self) -> String {
        self.content
            .iter()
            .map(|p| p.plain_text())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Check if the cell is empty.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty() || self.plain_text().trim().is_empty()
    }

    /// Check if this cell spans multiple rows or columns.
    pub fn is_merged(&self) -> bool {
        self.rowspan > 1 || self.colspan > 1
    }
}

/// Vertical alignment for table cells.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerticalAlignment {
    /// Top alignment
    #[default]
    Top,
    /// Middle/center alignment
    Middle,
    /// Bottom alignment
    Bottom,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_new() {
        let table = Table::new();
        assert!(table.is_empty());
        assert_eq!(table.row_count(), 0);
        assert_eq!(table.column_count(), 0);
    }

    #[test]
    fn test_table_with_data() {
        let mut table = Table::with_header(1);
        table.add_row(TableRow::header(vec![
            TableCell::text("Name"),
            TableCell::text("Age"),
        ]));
        table.add_row(TableRow::from_strings(["Alice", "30"]));
        table.add_row(TableRow::from_strings(["Bob", "25"]));

        assert_eq!(table.row_count(), 3);
        assert_eq!(table.column_count(), 2);
        assert_eq!(table.header().len(), 1);
        assert_eq!(table.body().len(), 2);
    }

    #[test]
    fn test_merged_cells() {
        let mut table = Table::new();
        table.add_row(TableRow::new(vec![TableCell::text("Merged").colspan(2)]));

        assert!(table.has_merged_cells());
    }

    #[test]
    fn test_cell_text() {
        let cell = TableCell::text("Hello");
        assert_eq!(cell.plain_text(), "Hello");
        assert!(!cell.is_empty());
    }
}
