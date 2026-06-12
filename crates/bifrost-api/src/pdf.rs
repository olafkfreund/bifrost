//! Render the migration report to PDF (#221).
//!
//! Pure-Rust via `printpdf` with the built-in **Helvetica** font — no font files
//! to bundle and no external tools, so it works air-gapped. We render the same
//! Markdown the `/api/report` endpoint produces (one content source), laying out
//! headings, bullets, tables, and wrapped paragraphs onto A4 pages.

use printpdf::{
    BuiltinFont, IndirectFontRef, Mm, PdfDocument, PdfDocumentReference, PdfLayerReference,
};

const PAGE_W: f32 = 210.0; // A4
const PAGE_H: f32 = 297.0;
const MARGIN: f32 = 18.0;
const BOTTOM: f32 = 18.0;

struct Doc {
    doc: PdfDocumentReference,
    layer: PdfLayerReference,
    regular: IndirectFontRef,
    bold: IndirectFontRef,
    y: f32,
}

impl Doc {
    fn new() -> Self {
        let (doc, page, layer) =
            PdfDocument::new("Migration Status Report", Mm(PAGE_W), Mm(PAGE_H), "Layer");
        let regular = doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();
        let bold = doc.add_builtin_font(BuiltinFont::HelveticaBold).unwrap();
        let layer = doc.get_page(page).get_layer(layer);
        Self {
            doc,
            layer,
            regular,
            bold,
            y: PAGE_H - MARGIN,
        }
    }

    /// Start a new page if the next `needed` mm wouldn't fit.
    fn ensure(&mut self, needed: f32) {
        if self.y - needed < BOTTOM {
            let (page, layer) = self.doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Layer");
            self.layer = self.doc.get_page(page).get_layer(layer);
            self.y = PAGE_H - MARGIN;
        }
    }

    /// Write one line of text at `size` pt, `bold`, indented `indent` mm.
    fn line(&mut self, text: &str, size: f32, bold: bool, indent: f32) {
        let line_h = size * 0.45 + 0.6;
        self.ensure(line_h);
        let font = if bold { &self.bold } else { &self.regular };
        self.layer
            .use_text(text, size, Mm(MARGIN + indent), Mm(self.y), font);
        self.y -= line_h;
    }

    fn gap(&mut self, mm: f32) {
        self.y -= mm;
    }

    fn into_bytes(self) -> Vec<u8> {
        use std::io::BufWriter;
        let mut buf = BufWriter::new(Vec::new());
        self.doc.save(&mut buf).unwrap();
        buf.into_inner().unwrap()
    }
}

/// Strip the inline Markdown emphasis/code markers we don't render in PDF.
fn clean(s: &str) -> String {
    s.replace("**", "").replace('`', "")
}

/// Word-wrap `text` to at most `max` characters per line.
fn wrap(text: &str, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.len() + 1 + word.len() > max {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Render the report Markdown to a PDF byte vector.
pub fn report_pdf(markdown: &str) -> Vec<u8> {
    let mut d = Doc::new();
    for raw in markdown.lines() {
        let trimmed = raw.trim_end();
        if let Some(h) = trimmed.strip_prefix("# ") {
            d.gap(2.0);
            d.line(&clean(h), 16.0, true, 0.0);
            d.gap(1.5);
        } else if let Some(h) = trimmed.strip_prefix("## ") {
            d.gap(2.0);
            d.line(&clean(h), 12.5, true, 0.0);
            d.gap(1.0);
        } else if let Some(b) = trimmed.strip_prefix("- ") {
            for (i, l) in wrap(&clean(b), 95).into_iter().enumerate() {
                let prefix = if i == 0 { "•  " } else { "   " };
                d.line(&format!("{prefix}{l}"), 9.0, false, 3.0);
            }
        } else if trimmed.starts_with('|') {
            // Table row: drop separator rows, render cells as spaced columns.
            if trimmed.contains("---") {
                continue;
            }
            let cells: Vec<String> = trimmed
                .trim_matches('|')
                .split('|')
                .map(|c| clean(c.trim()))
                .collect();
            d.line(&cells.join("    "), 8.5, false, 1.0);
        } else if let Some(q) = trimmed.strip_prefix("> ") {
            for l in wrap(&clean(q), 95) {
                d.line(&l, 9.0, false, 2.0);
            }
        } else if trimmed.is_empty() {
            d.gap(2.0);
        } else {
            for l in wrap(&clean(trimmed), 100) {
                d.line(&l, 9.5, false, 0.0);
            }
        }
    }
    d.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_valid_non_empty_pdf() {
        let md = "# Migration Status Report\n\n## Overview\n\n- one\n- two\n\n\
                  | A | B |\n|---|---|\n| x | y |\n\nA paragraph of body text.\n";
        let bytes = report_pdf(md);
        // A real PDF starts with the %PDF header and the EOF marker.
        assert!(bytes.starts_with(b"%PDF-"), "missing PDF header");
        assert!(bytes.len() > 800, "PDF unexpectedly small: {}", bytes.len());
    }

    #[test]
    fn wrap_breaks_on_word_boundaries() {
        let lines = wrap("alpha beta gamma delta", 11);
        assert!(lines.iter().all(|l| l.len() <= 11));
        assert_eq!(lines.join(" "), "alpha beta gamma delta");
    }
}
