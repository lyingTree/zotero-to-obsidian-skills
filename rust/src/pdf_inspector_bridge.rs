//! Pure-Rust PDF quality preflight & 4-layer Mojibake firewall bridge
//! powered by `firecrawl/pdf-inspector` (default-features = false, zero PDFium)
//! combined with `lopdf` Type-3 / missing-ToUnicode CMap font inspection.
use anyhow::{anyhow, ensure, Context, Result};
use lopdf::{Document, Object};
use pdf_inspector::{
    extract_pages_markdown_mem, process_pdf_mem_with_options, DetectionConfig, PdfOptions,
    PdfType, ProcessMode, ScanStrategy,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
};

use crate::{check_write, files, mojibake};

pub const PDF_INSPECTOR_VERSION: &str = "1.27.1";
const MAX_PDF_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct PageQuality {
    pub page: u64,
    pub needs_ocr: bool,
    pub reasons: Vec<String>,
    pub has_table: bool,
    pub is_multicolumn: bool,
    pub has_type3_or_unmapped_font: bool,
    pub unmapped_fonts: Vec<String>,
    pub mojibake_reasons: Vec<String>,
    pub markdown: String,
}

#[derive(Clone, Debug)]
pub struct PdfQualityReport {
    pub pdf_type: String,
    pub confidence: f32,
    pub page_count: u32,
    pub has_encoding_issues: bool,
    pub is_complex: bool,
    pub pages_needing_ocr: Vec<u32>,
    pub pages_with_tables: Vec<u32>,
    pub pages_with_columns: Vec<u32>,
    pub type3_or_unmapped_font_pages: Vec<u32>,
    pub mojibake_pages: Vec<u32>,
    pub cmap_gaps: Vec<Value>,
    pub pages: BTreeMap<u64, PageQuality>,
}

impl PdfQualityReport {
    pub fn to_json(&self) -> Value {
        let page_summaries: Vec<Value> = self
            .pages
            .values()
            .map(|p| {
                json!({
                    "page": p.page,
                    "needs_ocr": p.needs_ocr,
                    "reasons": p.reasons,
                    "has_table": p.has_table,
                    "is_multicolumn": p.is_multicolumn,
                    "has_type3_or_unmapped_font": p.has_type3_or_unmapped_font,
                    "unmapped_fonts": p.unmapped_fonts,
                    "mojibake_reasons": p.mojibake_reasons,
                })
            })
            .collect();
        json!({
            "backend": "pdf-inspector",
            "backend_version": PDF_INSPECTOR_VERSION,
            "pdf_type": self.pdf_type,
            "confidence": self.confidence,
            "page_count": self.page_count,
            "has_encoding_issues": self.has_encoding_issues,
            "is_complex": self.is_complex,
            "pages_needing_ocr": self.pages_needing_ocr,
            "pages_with_tables": self.pages_with_tables,
            "pages_with_columns": self.pages_with_columns,
            "type3_or_unmapped_font_pages": self.type3_or_unmapped_font_pages,
            "mojibake_pages": self.mojibake_pages,
            "cmap_gaps": self.cmap_gaps,
            "pages": page_summaries,
        })
    }
}

fn pdf_type_label(t: PdfType) -> &'static str {
    match t {
        PdfType::TextBased => "TextBased",
        PdfType::Scanned => "Scanned",
        PdfType::ImageBased => "ImageBased",
        PdfType::Mixed => "Mixed",
    }
}

fn resolve_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a lopdf::Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_dict().ok()),
        _ => None,
    }
}

/// Layer 2 audit: inspect every page's `/Resources /Font` dictionary for Type 3 fonts
/// without `/ToUnicode` or Identity-H/V CID (`/Type0`) fonts lacking `/ToUnicode`.
fn inspect_page_fonts(bytes: &[u8]) -> BTreeMap<u64, Vec<String>> {
    let mut bad_by_page = BTreeMap::new();
    let Ok(doc) = Document::load_mem(bytes) else {
        return bad_by_page;
    };
    for (page_num, page_id) in doc.get_pages() {
        let Ok((res_opt, _)) = doc.get_page_resources(page_id) else {
            continue;
        };
        let Some(res) = res_opt else {
            continue;
        };
        let Ok(fonts_obj) = res.get(b"Font") else {
            continue;
        };
        let Some(fonts) = resolve_dict(&doc, fonts_obj) else {
            continue;
        };
        let mut flagged = BTreeSet::new();
        for (res_name, font_obj) in fonts.iter() {
            let Some(font) = resolve_dict(&doc, font_obj) else {
                continue;
            };
            let subtype = font
                .get(b"Subtype")
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).into_owned())
                .unwrap_or_default();
            let base_font = font
                .get(b"BaseFont")
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).into_owned())
                .unwrap_or_else(|| String::from_utf8_lossy(res_name).into_owned());
            let has_to_unicode = font.has(b"ToUnicode");

            if subtype == "Type3" && !has_to_unicode {
                flagged.insert(format!("Type3:{base_font}"));
            } else if subtype == "Type0" && !has_to_unicode {
                let encoding = font
                    .get(b"Encoding")
                    .ok()
                    .and_then(|o| o.as_name().ok())
                    .map(|n| String::from_utf8_lossy(n).into_owned())
                    .unwrap_or_default();
                if encoding.starts_with("Identity-") {
                    flagged.insert(format!("Type0-NoToUnicode:{base_font}"));
                }
            }
        }
        if !flagged.is_empty() {
            bad_by_page.insert(u64::from(page_num), flagged.into_iter().collect());
        }
    }
    bad_by_page
}

/// Run pure-Rust `pdf-inspector` + `lopdf` font CMap + `mojibake` Layer 1/2/4 audit on PDF bytes.
pub fn inspect_pdf_bytes(bytes: &[u8]) -> Result<PdfQualityReport> {
    let options = PdfOptions::new()
        .mode(ProcessMode::Analyze)
        .detection(DetectionConfig {
            strategy: ScanStrategy::Full,
            ..Default::default()
        });

    let analysis = catch_unwind(AssertUnwindSafe(|| {
        process_pdf_mem_with_options(bytes, options)
    }))
    .map_err(|_| anyhow!("pdf-inspector panicked during PDF analysis"))?
    .map_err(|e| anyhow!("pdf-inspector analysis failed: {e}"))?;

    let pages_ext = catch_unwind(AssertUnwindSafe(|| extract_pages_markdown_mem(bytes, None)))
        .map_err(|_| anyhow!("pdf-inspector panicked during per-page extraction"))?
        .map_err(|e| anyhow!("pdf-inspector per-page extraction failed: {e}"))?;

    let unmapped_fonts_by_page = inspect_page_fonts(bytes);

    let mut reasons_map: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    for entry in &analysis.ocr_reasons_by_page {
        reasons_map
            .entry(entry.page)
            .or_default()
            .extend(entry.reasons.iter().cloned());
    }
    for entry in &pages_ext.ocr_reasons_by_page {
        reasons_map
            .entry(entry.page)
            .or_default()
            .extend(entry.reasons.iter().cloned());
    }

    let table_set: BTreeSet<u32> = analysis
        .layout
        .pages_with_tables
        .iter()
        .chain(pages_ext.pages_with_tables.iter())
        .copied()
        .collect();
    let col_set: BTreeSet<u32> = analysis
        .layout
        .pages_with_columns
        .iter()
        .chain(pages_ext.pages_with_columns.iter())
        .copied()
        .collect();
    let mut ocr_pages_set: BTreeSet<u32> = analysis
        .pages_needing_ocr
        .iter()
        .chain(pages_ext.pages_needing_ocr.iter())
        .copied()
        .collect();

    let mut type3_pages = BTreeSet::new();
    let mut mojibake_pages = BTreeSet::new();
    let mut pages = BTreeMap::new();

    for pmd in pages_ext.pages {
        let page_1 = pmd.page + 1;
        let page_u64 = u64::from(page_1);
        let mut reasons = reasons_map.remove(&page_1).unwrap_or_default();
        if let Some(r) = pmd.ocr_reason {
            reasons.insert(r);
        }
        let unmapped_fonts = unmapped_fonts_by_page
            .get(&page_u64)
            .cloned()
            .unwrap_or_default();
        let has_type3_or_unmapped_font = !unmapped_fonts.is_empty();
        if has_type3_or_unmapped_font {
            type3_pages.insert(page_1);
        }

        let quality = mojibake::analyze_text(&pmd.markdown);
        if quality.is_garbled {
            mojibake_pages.insert(page_1);
            reasons.insert("mojibake_detected".into());
            ocr_pages_set.insert(page_1);
        }

        let needs_ocr = pmd.needs_ocr || ocr_pages_set.contains(&page_1) || quality.is_garbled;
        if needs_ocr {
            ocr_pages_set.insert(page_1);
        }

        pages.insert(
            page_u64,
            PageQuality {
                page: page_u64,
                needs_ocr,
                reasons: reasons.into_iter().collect(),
                has_table: table_set.contains(&page_1),
                is_multicolumn: col_set.contains(&page_1),
                has_type3_or_unmapped_font,
                unmapped_fonts,
                mojibake_reasons: quality.reasons,
                markdown: pmd.markdown,
            },
        );
    }

    // Ensure every page up to page_count has an entry even if extraction returned fewer.
    for p in 1..=analysis.page_count {
        let page_u64 = u64::from(p);
        pages.entry(page_u64).or_insert_with(|| {
            let unmapped_fonts = unmapped_fonts_by_page
                .get(&page_u64)
                .cloned()
                .unwrap_or_default();
            let has_type3_or_unmapped_font = !unmapped_fonts.is_empty();
            if has_type3_or_unmapped_font {
                type3_pages.insert(p);
            }
            let reasons = reasons_map
                .remove(&p)
                .map(|s| s.into_iter().collect())
                .unwrap_or_default();
            PageQuality {
                page: page_u64,
                needs_ocr: ocr_pages_set.contains(&p),
                reasons,
                has_table: table_set.contains(&p),
                is_multicolumn: col_set.contains(&p),
                has_type3_or_unmapped_font,
                unmapped_fonts,
                mojibake_reasons: vec![],
                markdown: String::new(),
            }
        });
    }

    let cmap_gaps: Vec<Value> = analysis
        .cmap_gaps
        .iter()
        .map(|g| {
            json!({
                "font": g.font,
                "codes": g.codes,
                "interpolated": g.interpolated,
                "unmapped": g.unmapped,
            })
        })
        .collect();
    let has_encoding_issues = analysis.has_encoding_issues
        || !mojibake_pages.is_empty()
        || analysis.cmap_gaps.iter().any(|g| g.unmapped > 0);

    Ok(PdfQualityReport {
        pdf_type: pdf_type_label(analysis.pdf_type).into(),
        confidence: analysis.confidence,
        page_count: analysis.page_count,
        has_encoding_issues,
        is_complex: analysis.layout.is_complex || pages_ext.is_complex,
        pages_needing_ocr: ocr_pages_set.into_iter().collect(),
        pages_with_tables: table_set.into_iter().collect(),
        pages_with_columns: col_set.into_iter().collect(),
        type3_or_unmapped_font_pages: type3_pages.into_iter().collect(),
        mojibake_pages: mojibake_pages.into_iter().collect(),
        cmap_gaps,
        pages,
    })
}

/// Public operation entrypoint: inspect PDF quality, encoding health, CMap gaps,
/// Type 3 fonts, multi-column & table complexity, and OCR routing needs.
pub fn inspect_pdf_file(pdf_path: &Path, expected_sha: Option<&str>) -> Result<Value> {
    check_write::no_link(pdf_path)?;
    ensure!(
        std::fs::metadata(pdf_path)?.len() <= MAX_PDF_BYTES,
        "PDF exceeds inspection size budget"
    );
    let (raw, binding) = files::read_bound(pdf_path)?;
    if let Some(expected) = expected_sha {
        let actual = binding["sha256"].as_str().context("source sha256")?;
        ensure!(actual == expected, "SOURCE_PDF_HASH_MISMATCH");
    }
    let report = inspect_pdf_bytes(&raw)?;
    Ok(json!({
        "schema_version": 1,
        "status": "ok",
        "source_file": binding,
        "quality": report.to_json(),
        "source_fidelity": "not_assessed",
        "boundary": "Pure-Rust PDF structural preflight and Mojibake risk diagnosis via pdf-inspector and lopdf font inspection. Does not replace visual review of formulas, tables, or figures.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Stream};

    fn make_test_pdf(include_scan_page: bool, include_type3_font: bool) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let c1 = doc.add_object(Stream::new(
            dictionary! {},
            b"BT /F1 12 Tf 40 700 Tm (Ocean internal waves and stratification dynamics.) Tj 0 -18 Td (Observed amplitudes exceed ten meters in deep water.) Tj ET".to_vec(),
        ));
        let p1 = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "Contents" => c1,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        });
        let mut kids = vec![Object::Reference(p1)];

        if include_scan_page {
            let img = doc.add_object(Stream::new(
                dictionary! {
                    "Type" => "XObject",
                    "Subtype" => "Image",
                    "Width" => 2,
                    "Height" => 2,
                    "BitsPerComponent" => 8,
                    "ColorSpace" => "DeviceRGB",
                },
                vec![200u8; 12],
            ));
            let c2 = doc.add_object(Stream::new(
                dictionary! {},
                b"q 600 0 0 800 0 0 cm /Im1 Do Q".to_vec(),
            ));
            let p2 = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages,
                "Contents" => c2,
                "Resources" => dictionary! { "XObject" => dictionary! { "Im1" => img } },
            });
            kids.push(Object::Reference(p2));
        }

        if include_type3_font {
            let t3 = doc.add_object(dictionary! {
                "Type" => "Font",
                "Subtype" => "Type3",
                "FontBBox" => vec![0.into(), 0.into(), 1000.into(), 1000.into()],
                "FontMatrix" => vec![0.001.into(), 0.into(), 0.into(), 0.001.into(), 0.into(), 0.into()],
                "CharProcs" => dictionary! {},
                "Encoding" => "WinAnsiEncoding",
                "FirstChar" => 32,
                "LastChar" => 32,
                "Widths" => vec![500.into()],
            });
            let c3 = doc.add_object(Stream::new(
                dictionary! {},
                b"BT /F3 12 Tf 40 700 Tm ( ) Tj ET".to_vec(),
            ));
            let p3 = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages,
                "Contents" => c3,
                "Resources" => dictionary! { "Font" => dictionary! { "F3" => t3 } },
            });
            kids.push(Object::Reference(p3));
        }

        let count = kids.len() as i64;
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => count,
                "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        let mut buf = Vec::new();
        doc.save_to(&mut buf).unwrap();
        buf
    }

    #[test]
    fn classifies_text_scanned_and_type3_pages_pure_rust() {
        let born = make_test_pdf(false, false);
        let r1 = inspect_pdf_bytes(&born).unwrap();
        assert_eq!(r1.page_count, 1);
        assert_eq!(r1.pdf_type, "TextBased");
        assert!(r1.pages_needing_ocr.is_empty(), "{r1:?}");
        assert!(!r1.has_encoding_issues);

        let mixed = make_test_pdf(true, true);
        let r2 = inspect_pdf_bytes(&mixed).unwrap();
        assert_eq!(r2.page_count, 3);
        assert!(r2.pages_needing_ocr.contains(&2), "{r2:?}");
        assert!(r2.type3_or_unmapped_font_pages.contains(&3), "{r2:?}");
    }
}
