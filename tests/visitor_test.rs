//! Integration tests for the visitor pattern.

use unpdf::model::{Paragraph, Table, TableCell, TableRow};
use unpdf::render::visitor::{
    CompositeVisitor, DefaultVisitor, DocumentVisitor, MaxHeadingDepthVisitor, SimpleTableVisitor,
    SkipImagesVisitor, VisitorAction,
};

/// Custom visitor that tracks visit counts.
struct CountingVisitor {
    paragraph_count: usize,
    table_count: usize,
    image_count: usize,
    heading_count: usize,
}

impl CountingVisitor {
    fn new() -> Self {
        Self {
            paragraph_count: 0,
            table_count: 0,
            image_count: 0,
            heading_count: 0,
        }
    }
}

impl DocumentVisitor for CountingVisitor {
    fn visit_paragraph(&mut self, _para: &Paragraph) -> VisitorAction {
        self.paragraph_count += 1;
        VisitorAction::Continue
    }

    fn visit_table(&mut self, _table: &Table) -> VisitorAction {
        self.table_count += 1;
        VisitorAction::Continue
    }

    fn visit_image(&mut self, _id: &str, _alt: Option<&str>) -> VisitorAction {
        self.image_count += 1;
        VisitorAction::Continue
    }

    fn visit_heading(&mut self, _text: &str, _level: u8) -> VisitorAction {
        self.heading_count += 1;
        VisitorAction::Continue
    }
}

#[test]
fn test_default_visitor_all_continue() {
    let mut visitor = DefaultVisitor::new();
    let para = Paragraph::new();
    let table = Table::new();

    assert!(matches!(
        visitor.visit_paragraph(&para),
        VisitorAction::Continue
    ));
    assert!(matches!(
        visitor.visit_table(&table),
        VisitorAction::Continue
    ));
    assert!(matches!(
        visitor.visit_image("img1", None),
        VisitorAction::Continue
    ));
    assert!(matches!(
        visitor.visit_heading("Test", 1),
        VisitorAction::Continue
    ));
}

#[test]
fn test_skip_images_visitor() {
    let mut visitor = SkipImagesVisitor;

    // Images should be skipped
    let action = visitor.visit_image("img1", Some("Alt text"));
    assert!(action.should_skip());

    // Other elements should continue
    let para = Paragraph::new();
    let action = visitor.visit_paragraph(&para);
    assert!(matches!(action, VisitorAction::Continue));
}

#[test]
fn test_max_heading_depth_visitor() {
    let mut visitor = MaxHeadingDepthVisitor::new(2);

    // H1 should stay H1
    let action = visitor.visit_heading("Title", 1);
    assert!(action.is_replace());
    let content = action.replacement().unwrap();
    assert!(content.starts_with("# "));

    // H4 should become H2
    let action = visitor.visit_heading("Subsection", 4);
    assert!(action.is_replace());
    let content = action.replacement().unwrap();
    assert!(content.starts_with("## "));
}

#[test]
fn test_simple_table_visitor() {
    let mut visitor = SimpleTableVisitor;

    // Create a simple table
    let mut table = Table::new();
    let row = TableRow {
        cells: vec![TableCell::text("A"), TableCell::text("B")],
        is_header: true,
    };
    table.rows.push(row);

    let action = visitor.visit_table(&table);
    assert!(action.is_replace());

    let content = action.replacement().unwrap();
    assert!(content.contains("A"));
    assert!(content.contains("B"));
    assert!(content.contains("|"));
}

#[test]
fn test_composite_visitor_chaining() {
    let mut composite = CompositeVisitor::new()
        .with_visitor(SkipImagesVisitor)
        .with_visitor(MaxHeadingDepthVisitor::new(3))
        .with_visitor(DefaultVisitor);

    // First matching action wins
    let action = composite.visit_image("img1", None);
    assert!(action.should_skip());

    // Heading goes to second visitor
    let action = composite.visit_heading("Test", 5);
    assert!(action.is_replace());
    let content = action.replacement().unwrap();
    assert!(content.starts_with("### "));

    // Paragraph continues through all
    let para = Paragraph::new();
    let action = composite.visit_paragraph(&para);
    assert!(matches!(action, VisitorAction::Continue));
}

#[test]
fn test_visitor_action_methods() {
    let continue_action = VisitorAction::Continue;
    assert!(!continue_action.should_skip());
    assert!(!continue_action.is_replace());
    assert!(continue_action.replacement().is_none());

    let skip_action = VisitorAction::Skip;
    assert!(skip_action.should_skip());
    assert!(!skip_action.is_replace());
    assert!(skip_action.replacement().is_none());

    let replace_action = VisitorAction::Replace("replaced".to_string());
    assert!(!replace_action.should_skip());
    assert!(replace_action.is_replace());
    assert_eq!(replace_action.replacement(), Some("replaced"));
}

#[test]
fn test_counting_visitor() {
    let mut visitor = CountingVisitor::new();

    // Visit various elements
    visitor.visit_paragraph(&Paragraph::new());
    visitor.visit_paragraph(&Paragraph::new());
    visitor.visit_table(&Table::new());
    visitor.visit_image("img1", None);
    visitor.visit_heading("Test", 1);
    visitor.visit_heading("Sub", 2);

    assert_eq!(visitor.paragraph_count, 2);
    assert_eq!(visitor.table_count, 1);
    assert_eq!(visitor.image_count, 1);
    assert_eq!(visitor.heading_count, 2);
}
