//! Pure-Rust multi-format document readers for HTML/XHTML, EPUB, ODT, and TXT/Markdown.
//! Produces schema-1 source inventory records with smart encoding transcoding and
//! zero-Mojibake auditing, using zero Python or external binaries.
use anyhow::{anyhow, ensure, Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::Path,
    sync::OnceLock,
};

use crate::{check_ledger, check_write, files, mojibake};

pub const BACKEND: &str = "rust-text-formats-v1";
const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MEMBER_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ARCHIVE_FILES: usize = 2048;
const MAX_BLOCKS: usize = 200_000;

fn compile_rx(s: &str) -> Regex {
    Regex::new(s).expect("text_formats regex")
}
macro_rules! rx {
    ($s:expr) => {{
        static R: OnceLock<Regex> = OnceLock::new();
        R.get_or_init(|| compile_rx($s))
    }};
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatKind {
    Html,
    Epub,
    Odt,
    Text,
}

impl FormatKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Epub => "epub",
            Self::Odt => "odt",
            Self::Text => "text",
        }
    }
}

fn id_ok(id: &str) -> bool {
    id.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_block(
    blocks: &mut Vec<Value>,
    source_id: &str,
    fmt: &str,
    kind: &str,
    text: &str,
    part: &str,
    order_in_part: usize,
) -> Result<()> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    ensure!(blocks.len() < MAX_BLOCKS, "Document exceeds block budget");
    let order = blocks.len();
    let id = format!("{source_id}-{fmt}-b{}", order + 1);
    blocks.push(json!({
        "id": id,
        "kind": kind,
        "order": order,
        "text": trimmed,
        "locator": {
            "part": part,
            "index": order_in_part,
        },
    }));
    Ok(())
}

/// Extract clean structured blocks from HTML/XHTML markup without external DOM crates.
fn parse_html_blocks(
    html: &str,
    source_id: &str,
    fmt: &str,
    part: &str,
    blocks: &mut Vec<Value>,
) -> Result<()> {
    // Strip non-visible containers first.
    let cleaned = rx!(r"(?is)<(?:script|style|noscript|template|svg|head)\b[^>]*>.*?</(?:script|style|noscript|template|svg|head)>")
        .replace_all(html, " ");
    let cleaned = rx!(r"(?s)<!--.*?-->").replace_all(&cleaned, " ");

    // Extract tables first or walk major block tags in source order.
    let block_rx = rx!(
        r"(?is)<(h[1-6]|p|li|blockquote|pre|figcaption|table|dt|dd)\b[^>]*>(.*?)</(?:h[1-6]|p|li|blockquote|pre|figcaption|table|dt|dd)>"
    );
    let tag_rx = rx!(r"(?s)<[^>]+>");
    let mut part_idx = 0usize;

    for cap in block_rx.captures_iter(&cleaned) {
        let tag = cap
            .get(1)
            .map(|m| m.as_str().to_ascii_lowercase())
            .unwrap_or_default();
        let inner = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        if tag == "table" {
            let mut md_rows = Vec::new();
            let tr_rx = rx!(r"(?is)<tr\b[^>]*>(.*?)</tr>");
            let cell_rx = rx!(r"(?is)<t[hd]\b[^>]*>(.*?)</t[hd]>");
            for tr in tr_rx.captures_iter(inner) {
                let row_inner = tr.get(1).map(|m| m.as_str()).unwrap_or("");
                let cells: Vec<String> = cell_rx
                    .captures_iter(row_inner)
                    .map(|c| {
                        let raw = tag_rx.replace_all(c.get(1).map(|m| m.as_str()).unwrap_or(""), " ");
                        normalize_ws(&html_escape::decode_html_entities(&raw))
                            .replace('|', "\\|")
                    })
                    .collect();
                if !cells.is_empty() {
                    md_rows.push(cells);
                }
            }
            if !md_rows.is_empty() {
                let cols = md_rows.iter().map(Vec::len).max().unwrap_or(1);
                let mut table_md = String::new();
                for (ri, mut row) in md_rows.into_iter().enumerate() {
                    row.resize(cols, String::new());
                    table_md.push_str("| ");
                    table_md.push_str(&row.join(" | "));
                    table_md.push_str(" |\n");
                    if ri == 0 {
                        table_md.push_str("| ");
                        table_md.push_str(&vec!["---"; cols].join(" | "));
                        table_md.push_str(" |\n");
                    }
                }
                part_idx += 1;
                push_block(blocks, source_id, fmt, "table", &table_md, part, part_idx)?;
            }
            continue;
        }

        let stripped = tag_rx.replace_all(inner, " ");
        let decoded = html_escape::decode_html_entities(&stripped);
        let text = if tag == "pre" {
            decoded.trim().to_owned()
        } else {
            normalize_ws(&decoded)
        };
        if text.is_empty() {
            continue;
        }
        let kind = match tag.as_str() {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
            "figcaption" => "caption",
            "li" => "list_item",
            "blockquote" => "quote",
            "pre" => "code",
            _ => "paragraph",
        };
        part_idx += 1;
        push_block(blocks, source_id, fmt, kind, &text, part, part_idx)?;
    }

    // Fallback when HTML had no standard block tags (e.g., bare <div>/<body>)
    if part_idx == 0 {
        let stripped = tag_rx.replace_all(&cleaned, "\n");
        let decoded = html_escape::decode_html_entities(&stripped);
        for chunk in decoded.split("\n\n") {
            let text = normalize_ws(chunk);
            if !text.is_empty() {
                part_idx += 1;
                push_block(blocks, source_id, fmt, "paragraph", &text, part, part_idx)?;
            }
        }
    }
    Ok(())
}

fn parse_plain_text_blocks(
    text: &str,
    source_id: &str,
    part: &str,
    blocks: &mut Vec<Value>,
) -> Result<()> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut buf = Vec::new();
    let mut part_idx = 0usize;

    let flush = |buf: &mut Vec<&str>, blocks: &mut Vec<Value>, part_idx: &mut usize| -> Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        let joined = buf.join("\n");
        buf.clear();
        let trimmed = joined.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        *part_idx += 1;
        push_block(blocks, source_id, "text", "paragraph", trimmed, part, *part_idx)
    };

    for line in normalized.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut buf, blocks, &mut part_idx)?;
            continue;
        }
        if trimmed.starts_with('#')
            && trimmed
                .trim_start_matches('#')
                .starts_with([' ', '\t'])
        {
            flush(&mut buf, blocks, &mut part_idx)?;
            part_idx += 1;
            let heading_text = trimmed.trim_start_matches('#').trim();
            push_block(blocks, source_id, "text", "heading", heading_text, part, part_idx)?;
            continue;
        }
        buf.push(line);
    }
    flush(&mut buf, blocks, &mut part_idx)?;
    Ok(())
}

fn read_zip_member(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let mut file = archive
        .by_name(name)
        .with_context(|| format!("Missing archive member: {name}"))?;
    ensure!(
        file.size() <= MAX_MEMBER_BYTES,
        "Archive member exceeds size budget: {name}"
    );
    let mut buf = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut buf)?;
    ensure!(
        buf.len() as u64 <= MAX_MEMBER_BYTES,
        "Archive member exceeds size budget: {name}"
    );
    Ok(buf)
}

fn resolve_zip_relative(base_dir: &str, href: &str) -> Result<String> {
    let clean = href.split('#').next().unwrap_or(href).replace('\\', "/");
    let mut parts: Vec<&str> = if base_dir.is_empty() {
        vec![]
    } else {
        base_dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in clean.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts
                    .pop()
                    .ok_or_else(|| anyhow!("Archive path escapes root: {href}"))?;
            }
            other => parts.push(other),
        }
    }
    ensure!(!parts.is_empty(), "Empty archive member path");
    Ok(parts.join("/"))
}

fn parse_epub_blocks(raw: &[u8], source_id: &str, blocks: &mut Vec<Value>) -> Result<Vec<String>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(raw)).context("Open EPUB archive")?;
    ensure!(archive.len() <= MAX_ARCHIVE_FILES, "EPUB exceeds file count budget");

    let container_bytes = read_zip_member(&mut archive, "META-INF/container.xml")?;
    let container_text = mojibake::decode_text_bytes(&container_bytes, "META-INF/container.xml")?.text;
    let container_xml = roxmltree::Document::parse(&container_text).context("Parse EPUB container.xml")?;
    let opf_path = container_xml
        .descendants()
        .find(|n| n.tag_name().name() == "rootfile")
        .and_then(|n| n.attribute("full-path"))
        .context("EPUB container.xml missing rootfile full-path")?
        .to_owned();

    let opf_dir = opf_path
        .rsplit_once('/')
        .map(|(d, _)| d.to_owned())
        .unwrap_or_default();
    let opf_bytes = read_zip_member(&mut archive, &opf_path)?;
    let opf_text = mojibake::decode_text_bytes(&opf_bytes, &opf_path)?.text;
    let opf_xml = roxmltree::Document::parse(&opf_text).context("Parse EPUB OPF package")?;

    let mut manifest = std::collections::BTreeMap::<String, String>::new();
    for node in opf_xml.descendants().filter(|n| n.tag_name().name() == "item") {
        if let (Some(id), Some(href)) = (node.attribute("id"), node.attribute("href")) {
            let media = node.attribute("media-type").unwrap_or("");
            if media.contains("html") || media.contains("xml") || href.ends_with(".xhtml") || href.ends_with(".html") || href.ends_with(".htm") {
                manifest.insert(id.to_owned(), href.to_owned());
            }
        }
    }

    let mut spine_parts = Vec::new();
    for node in opf_xml.descendants().filter(|n| n.tag_name().name() == "itemref") {
        if let Some(idref) = node.attribute("idref") {
            if let Some(href) = manifest.get(idref) {
                let member = resolve_zip_relative(&opf_dir, href)?;
                spine_parts.push(member);
            }
        }
    }
    ensure!(!spine_parts.is_empty(), "EPUB spine contains no readable XHTML/HTML documents");

    for part in &spine_parts {
        let bytes = read_zip_member(&mut archive, part)?;
        let decoded = mojibake::decode_text_bytes(&bytes, part)?;
        parse_html_blocks(&decoded.text, source_id, "epub", part, blocks)?;
    }
    Ok(spine_parts)
}

fn collect_odt_text(node: roxmltree::Node<'_, '_>, out: &mut String) {
    for child in node.children() {
        if child.is_text() {
            if let Some(t) = child.text() {
                out.push_str(t);
            }
            continue;
        }
        if !child.is_element() {
            continue;
        }
        match child.tag_name().name() {
            "s" => {
                let count = child
                    .attribute(("urn:oasis:names:tc:opendocument:xmlns:text:1.0", "c"))
                    .or_else(|| child.attribute("c"))
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .min(64);
                for _ in 0..count {
                    out.push(' ');
                }
            }
            "tab" => out.push('\t'),
            "line-break" => out.push('\n'),
            _ => collect_odt_text(child, out),
        }
    }
}

fn parse_odt_blocks(raw: &[u8], source_id: &str, blocks: &mut Vec<Value>) -> Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(raw)).context("Open ODT archive")?;
    ensure!(archive.len() <= MAX_ARCHIVE_FILES, "ODT exceeds file count budget");
    let content_bytes = read_zip_member(&mut archive, "content.xml")?;
    let content_text = mojibake::decode_text_bytes(&content_bytes, "content.xml")?.text;
    let xml = roxmltree::Document::parse(&content_text).context("Parse ODT content.xml")?;
    let body = xml
        .descendants()
        .find(|n| n.tag_name().name() == "text")
        .context("ODT content.xml missing office:text body")?;

    let mut part_idx = 0usize;
    for child in body.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "h" => {
                let mut t = String::new();
                collect_odt_text(child, &mut t);
                let text = normalize_ws(&t);
                if !text.is_empty() {
                    part_idx += 1;
                    push_block(blocks, source_id, "odt", "heading", &text, "content.xml", part_idx)?;
                }
            }
            "p" => {
                let mut t = String::new();
                collect_odt_text(child, &mut t);
                let text = normalize_ws(&t);
                if !text.is_empty() {
                    part_idx += 1;
                    push_block(blocks, source_id, "odt", "paragraph", &text, "content.xml", part_idx)?;
                }
            }
            "list" => {
                for item in child.descendants().filter(|n| n.tag_name().name() == "p") {
                    let mut t = String::new();
                    collect_odt_text(item, &mut t);
                    let text = normalize_ws(&t);
                    if !text.is_empty() {
                        part_idx += 1;
                        push_block(blocks, source_id, "odt", "list_item", &text, "content.xml", part_idx)?;
                    }
                }
            }
            "table" => {
                let mut md_rows = Vec::new();
                for row in child.children().filter(|n| n.tag_name().name() == "table-row") {
                    let mut cells = Vec::new();
                    for cell in row.children().filter(|n| n.tag_name().name() == "table-cell") {
                        let mut t = String::new();
                        collect_odt_text(cell, &mut t);
                        cells.push(normalize_ws(&t).replace('|', "\\|"));
                    }
                    if !cells.is_empty() {
                        md_rows.push(cells);
                    }
                }
                if !md_rows.is_empty() {
                    let cols = md_rows.iter().map(Vec::len).max().unwrap_or(1);
                    let mut table_md = String::new();
                    for (ri, mut row) in md_rows.into_iter().enumerate() {
                        row.resize(cols, String::new());
                        table_md.push_str("| ");
                        table_md.push_str(&row.join(" | "));
                        table_md.push_str(" |\n");
                        if ri == 0 {
                            table_md.push_str("| ");
                            table_md.push_str(&vec!["---"; cols].join(" | "));
                            table_md.push_str(" |\n");
                        }
                    }
                    part_idx += 1;
                    push_block(blocks, source_id, "odt", "table", &table_md, "content.xml", part_idx)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn read_bound_document(
    source: &Path,
    id: &str,
    kind: FormatKind,
    expected_sha: Option<&str>,
) -> Result<Value> {
    ensure!(id_ok(id), "Invalid source ID: {id}");
    check_write::no_link(source)?;
    ensure!(
        fs::metadata(source)?.len() <= MAX_INPUT_BYTES,
        "Source document exceeds size budget: {}",
        source.display()
    );
    let (raw, binding) = files::read_bound(source)?;
    if let Some(exp) = expected_sha {
        ensure!(
            binding["sha256"].as_str() == Some(exp),
            "Source document hash mismatch: {id}"
        );
    }

    let file_name = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    let mut blocks = Vec::new();
    let mut encoding = "utf-8".to_owned();
    let mut parts = vec![file_name.to_owned()];

    match kind {
        FormatKind::Html => {
            let decoded = mojibake::decode_text_bytes(&raw, file_name)?;
            encoding = decoded.encoding;
            parse_html_blocks(&decoded.text, id, "html", file_name, &mut blocks)?;
        }
        FormatKind::Text => {
            let decoded = mojibake::decode_text_bytes(&raw, file_name)?;
            encoding = decoded.encoding;
            parse_plain_text_blocks(&decoded.text, id, file_name, &mut blocks)?;
        }
        FormatKind::Epub => {
            parts = parse_epub_blocks(&raw, id, &mut blocks)?;
        }
        FormatKind::Odt => {
            parse_odt_blocks(&raw, id, &mut blocks)?;
            parts = vec!["content.xml".into()];
        }
    }
    ensure!(
        !blocks.is_empty(),
        "No readable text blocks extracted from {} document: {id}",
        kind.as_str()
    );

    let full_text: String = blocks
        .iter()
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let audit = mojibake::analyze_text(&full_text);

    Ok(json!({
        "schema_version": 1,
        "format": kind.as_str(),
        "inventory_backend": BACKEND,
        "source_file": binding,
        "encoding": encoding,
        "parts": parts,
        "native_blocks": blocks,
        "figures": [],
        "mojibake_audit": audit.to_json(),
        "source_fidelity": "not_assessed",
    }))
}

fn write_atomic_new(dir: &Path, name: &str, bytes: &[u8]) -> Result<Value> {
    let p = dir.join(name);
    ensure!(!p.exists() && !p.is_symlink(), "Keep existing source evidence: {}", p.display());
    let mut tmp = tempfile::Builder::new().prefix(".pending-doc-").tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist_noclobber(&p)
        .map_err(|e| anyhow!("Cannot publish {}: {}", p.display(), e.error))?;
    let b = check_ledger::bind_file(&p)?;
    ensure!(b["sha256"] == files::digest(bytes), "Evidence changed after write");
    Ok(b)
}

fn render_clean_markdown(doc: &Value) -> String {
    let fmt = doc["format"].as_str().unwrap_or("text");
    let mut out = format!("<!-- Derived from pure-Rust {fmt} native reader. -->\n\n");
    for b in doc["native_blocks"].as_array().into_iter().flatten() {
        let id = b["id"].as_str().unwrap_or("");
        let kind = b["kind"].as_str().unwrap_or("paragraph");
        let text = b["text"].as_str().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }
        match kind {
            "heading" => out.push_str(&format!("## {text} <!-- {id} -->\n\n")),
            "caption" => out.push_str(&format!("<!-- {id} (caption) -->\n**{text}**\n\n")),
            "list_item" => out.push_str(&format!("- <!-- {id} --> {text}\n")),
            "quote" => out.push_str(&format!("> <!-- {id} --> {text}\n\n")),
            "code" => out.push_str(&format!("<!-- {id} (code) -->\n```\n{text}\n```\n\n")),
            "table" => out.push_str(&format!("<!-- {id} (table) -->\n{text}\n\n")),
            _ => out.push_str(&format!("<!-- {id} --> {text}\n\n")),
        }
    }
    out
}

pub fn prepare_bound_document(
    source: &Path,
    id: &str,
    destination: &Path,
    kind: FormatKind,
    expected_sha: Option<&str>,
) -> Result<Value> {
    let mut doc = read_bound_document(source, id, kind, expected_sha)?;
    ensure!(
        !destination.exists() && !destination.is_symlink(),
        "Keep existing source evidence directory: {}",
        destination.display()
    );
    fs::create_dir_all(destination)?;
    let dir = files::canonical(destination)?;

    let mut plain = String::new();
    for b in doc["native_blocks"].as_array().into_iter().flatten() {
        plain.push_str(&format!(
            "[{} | {}]\n{}\n\n",
            b["id"].as_str().unwrap_or(""),
            b["kind"].as_str().unwrap_or("paragraph"),
            b["text"].as_str().unwrap_or("")
        ));
    }
    let clean_md = render_clean_markdown(&doc);
    doc["native_text"] = write_atomic_new(&dir, "native-blocks.txt", plain.as_bytes())?;
    doc["clean_markdown"] = write_atomic_new(&dir, "clean-source.md", clean_md.as_bytes())?;
    let mut json_bytes = serde_json::to_vec_pretty(&doc)?;
    json_bytes.push(b'\n');
    doc["native_document"] = write_atomic_new(&dir, "native-document.json", &json_bytes)?;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    #[test]
    fn reads_html_epub_odt_and_gbk_text_pure_rust() {
        let dir = tempfile::tempdir().unwrap();

        // 1. HTML with entities, script stripping, and table
        let html_path = dir.path().join("paper.html");
        fs::write(
            &html_path,
            "<html><head><title>Ignored</title><style>body{}</style></head><body>\
             <h1>Ocean Dynamics</h1><script>alert(1)</script>\
             <p>Internal waves &amp; stratification &le; 10 m.</p>\
             <table><tr><th>Metric</th><th>Value</th></tr><tr><td>Depth</td><td>500 m</td></tr></table>\
             </body></html>",
        )
        .unwrap();
        let html_doc = prepare_bound_document(&html_path, "h1", &dir.path().join("out-html"), FormatKind::Html, None).unwrap();
        assert_eq!(html_doc["format"], "html");
        assert_eq!(html_doc["mojibake_audit"]["is_garbled"], false);
        let blocks = html_doc["native_blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 3, "{blocks:?}");
        assert_eq!(blocks[0]["kind"], "heading");
        assert!(blocks[1]["text"].as_str().unwrap().contains("≤ 10 m"));
        assert!(!blocks[1]["text"].as_str().unwrap().contains("alert"));
        assert_eq!(blocks[2]["kind"], "table");

        // 2. EPUB archive
        let epub_path = dir.path().join("book.epub");
        {
            let f = fs::File::create(&epub_path).unwrap();
            let mut z = zip::ZipWriter::new(f);
            let opts = SimpleFileOptions::default();
            z.start_file("META-INF/container.xml", opts).unwrap();
            z.write_all(br#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#).unwrap();
            z.start_file("OEBPS/content.opf", opts).unwrap();
            z.write_all(br#"<package><manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="ch1"/></spine></package>"#).unwrap();
            z.start_file("OEBPS/ch1.xhtml", opts).unwrap();
            z.write_all(b"<html><body><h2>Chapter 1</h2><p>Pure Rust EPUB extraction.</p></body></html>").unwrap();
            z.finish().unwrap();
        }
        let epub_doc = read_bound_document(&epub_path, "e1", FormatKind::Epub, None).unwrap();
        assert_eq!(epub_doc["format"], "epub");
        assert_eq!(epub_doc["native_blocks"].as_array().unwrap().len(), 2);

        // 3. ODT archive
        let odt_path = dir.path().join("notes.odt");
        {
            let f = fs::File::create(&odt_path).unwrap();
            let mut z = zip::ZipWriter::new(f);
            let opts = SimpleFileOptions::default();
            z.start_file("content.xml", opts).unwrap();
            z.write_all(br#"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"><office:body><office:text><text:h>ODT Heading</text:h><text:p>Hello<text:s text:c="1"/>ODT world.</text:p></office:text></office:body></office:document-content>"#).unwrap();
            z.finish().unwrap();
        }
        let odt_doc = read_bound_document(&odt_path, "o1", FormatKind::Odt, None).unwrap();
        assert_eq!(odt_doc["format"], "odt");
        assert_eq!(odt_doc["native_blocks"][0]["text"], "ODT Heading");
        assert_eq!(odt_doc["native_blocks"][1]["text"], "Hello ODT world.");

        // 4. GBK encoded TXT
        let txt_path = dir.path().join("gbk.txt");
        let (encoded, _, _) = encoding_rs::GB18030.encode("# 海洋内波研究\n\n这是 GBK 编码的学术摘要正文。");
        fs::write(&txt_path, encoded).unwrap();
        let txt_doc = read_bound_document(&txt_path, "t1", FormatKind::Text, None).unwrap();
        assert_eq!(txt_doc["mojibake_audit"]["is_garbled"], false);
        assert_eq!(txt_doc["native_blocks"][0]["text"], "海洋内波研究");
    }
}
