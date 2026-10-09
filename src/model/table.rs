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

    /// The table as CSV ([RFC 4180](https://www.rfc-editor.org/rfc/rfc4180)): one record per
    /// row, header rows first as they are, records ended by CRLF.
    ///
    /// A merged cell's text is in its top-left position and the positions it covers are
    /// empty, so every record has the same number of fields and a value is never counted
    /// twice. A field holding the delimiter, a quote or a line break is quoted, with quotes
    /// doubled; a cell's paragraphs are kept on their own lines inside it.
    ///
    /// ```
    /// use unpdf::model::{Table, TableCell, TableRow};
    ///
    /// let mut table = Table::with_header(1);
    /// table.add_row(TableRow::header(vec![TableCell::text("Item"), TableCell::text("Note")]));
    /// table.add_row(TableRow::from_strings(["Bolt, M6", "He said \"no\""]));
    /// assert_eq!(table.to_csv(), "Item,Note\r\n\"Bolt, M6\",\"He said \"\"no\"\"\"\r\n");
    /// ```
    pub fn to_csv(&self) -> String {
        self.to_delimited(',')
    }

    /// The table as delimited text, like [`Table::to_csv`] with `delimiter` between fields
    /// (`'\t'` for TSV).
    pub fn to_delimited(&self, delimiter: char) -> String {
        let mut out = String::new();
        for record in self.grid() {
            for (i, field) in record.iter().enumerate() {
                if i > 0 {
                    out.push(delimiter);
                }
                let quote = field.contains(delimiter)
                    || field.contains('"')
                    || field.contains('\n')
                    || field.contains('\r');
                if quote {
                    out.push('"');
                    out.push_str(&field.replace('"', "\"\""));
                    out.push('"');
                } else {
                    out.push_str(field);
                }
            }
            out.push_str("\r\n");
        }
        out
    }

    /// The table laid out on its grid: each merged cell's text at its top-left position, the
    /// positions it covers empty, every row as wide as the widest.
    fn grid(&self) -> Vec<Vec<String>> {
        // Rows still covered, per column, by a cell merged down from a row above.
        let mut covered: Vec<usize> = Vec::new();
        let mut grid: Vec<Vec<String>> = Vec::new();
        for row in &self.rows {
            let mut record: Vec<String> = Vec::new();
            let mut col = 0;
            for cell in &row.cells {
                skip_covered(&mut covered, &mut record, &mut col);
                let text = cell
                    .content
                    .iter()
                    .map(|p| p.plain_text())
                    .collect::<Vec<_>>()
                    .join("\n");
                record.push(text);
                let span = usize::from(cell.colspan.max(1));
                record.extend(std::iter::repeat_n(String::new(), span - 1));
                if covered.len() < col + span {
                    covered.resize(col + span, 0);
                }
                for c in &mut covered[col..col + span] {
                    *c = usize::from(cell.rowspan.max(1)) - 1;
                }
                col += span;
            }
            // Columns past the row's last cell that a cell above still covers.
            while col < covered.len() {
                if covered[col] > 0 {
                    covered[col] -= 1;
                }
                record.push(String::new());
                col += 1;
            }
            grid.push(record);
        }
        let width = grid.iter().map(Vec::len).max().unwrap_or(0);
        for record in &mut grid {
            record.resize(width, String::new());
        }
        grid
    }
}

/// Pass the columns, from `col` on, that a cell merged down from a row above still covers,
/// leaving each one empty in `record`.
fn skip_covered(covered: &mut [usize], record: &mut Vec<String>, col: &mut usize) {
    while covered.get(*col).is_some_and(|&n| n > 0) {
        covered[*col] -= 1;
        record.push(String::new());
        *col += 1;
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

    fn merged(text: &str, rowspan: u8, colspan: u8) -> TableCell {
        let mut cell = TableCell::text(text);
        cell.rowspan = rowspan;
        cell.colspan = colspan;
        cell
    }

    #[test]
    fn merged_cells_keep_their_value_once_and_their_grid() {
        // | Region (2 rows) | Sales (2 cols)  |
        // |                 | 2024   | 2025   |
        // | North           | 10     | 12     |
        let mut table = Table::with_header(2);
        table.add_row(TableRow::header(vec![
            merged("Region", 2, 1),
            merged("Sales", 1, 2),
        ]));
        table.add_row(TableRow::header(vec![
            TableCell::text("2024"),
            TableCell::text("2025"),
        ]));
        table.add_row(TableRow::from_strings(["North", "10", "12"]));
        assert_eq!(
            table.to_csv(),
            "Region,Sales,\r\n,2024,2025\r\nNorth,10,12\r\n"
        );
    }

    #[test]
    fn a_cell_merged_down_at_the_row_end_leaves_the_next_row_aligned() {
        let mut table = Table::new();
        table.add_row(TableRow::new(vec![
            TableCell::text("a"),
            merged("tall", 2, 1),
        ]));
        table.add_row(TableRow::from_strings(["b"]));
        table.add_row(TableRow::from_strings(["c", "d"]));
        assert_eq!(table.to_csv(), "a,tall\r\nb,\r\nc,d\r\n");
    }

    #[test]
    fn a_cells_paragraphs_stay_on_their_own_lines_and_tsv_uses_tabs() {
        let mut cell = TableCell::text("first");
        cell.content
            .push(crate::model::Paragraph::with_text("second"));
        let mut table = Table::new();
        table.add_row(TableRow::new(vec![cell, TableCell::text("x\ty")]));
        assert_eq!(table.to_csv(), "\"first\nsecond\",x\ty\r\n");
        assert_eq!(table.to_delimited('\t'), "\"first\nsecond\"\t\"x\ty\"\r\n");
    }
}
