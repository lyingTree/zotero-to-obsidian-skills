//! Zero-Mojibake text decoding and corruption detection firewall.
//! Shared across PDF preflight, layout extraction, TeX/multi-format readers,
//! OCR verification, and final Markdown note publish gates.
use anyhow::{bail, ensure, Result};
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::OnceLock;

fn compile_rx(s: &str) -> Regex {
    Regex::new(s).expect("mojibake regex")
}
macro_rules! rx {
    ($s:expr) => {{
        static R: OnceLock<Regex> = OnceLock::new();
        R.get_or_init(|| compile_rx($s))
    }};
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedText {
    pub text: String,
    pub encoding: String,
    pub had_bom: bool,
    pub replaced_count: usize,
}

/// Smart encoding decoder for TeX, HTML, TXT, Markdown and XML sources.
/// Order: reject PDF/binary -> UTF-8/UTF-16 BOM -> strict UTF-8 ->
/// `chardetng` + `encoding_rs` transcoding with zero-replacement fallback.
pub fn decode_text_bytes(bytes: &[u8], label: &str) -> Result<DecodedText> {
    ensure!(
        !bytes.starts_with(b"%PDF-"),
        "Input is a PDF binary rather than a text document: {label}"
    );
    if bytes.is_empty() {
        return Ok(DecodedText {
            text: String::new(),
            encoding: "utf-8".into(),
            had_bom: false,
            replaced_count: 0,
        });
    }

    // 1. UTF-8 BOM
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        if let Ok(s) = std::str::from_utf8(rest) {
            ensure!(!s.contains('\0'), "Text stream contains NUL bytes: {label}");
            return Ok(DecodedText {
                text: s.to_owned(),
                encoding: "utf-8".into(),
                had_bom: true,
                replaced_count: 0,
            });
        }
    }

    // 2. UTF-16LE BOM (avoiding UTF-32LE 0xFF 0xFE 0x00 0x00)
    if bytes.len() >= 2
        && bytes[0] == 0xFF
        && bytes[1] == 0xFE
        && !(bytes.len() >= 4 && bytes[2] == 0x00 && bytes[3] == 0x00)
    {
        let (cow, had_errors) = encoding_rs::UTF_16LE.decode_without_bom_handling(&bytes[2..]);
        let text = cow.into_owned();
        let replaced = text.chars().filter(|&c| c == '\u{FFFD}').count();
        ensure!(
            !had_errors && replaced == 0 && !text.contains('\0'),
            "Corrupted UTF-16LE text stream: {label}"
        );
        return Ok(DecodedText {
            text,
            encoding: "utf-16le".into(),
            had_bom: true,
            replaced_count: 0,
        });
    }

    // 3. UTF-16BE BOM
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let (cow, had_errors) = encoding_rs::UTF_16BE.decode_without_bom_handling(&bytes[2..]);
        let text = cow.into_owned();
        let replaced = text.chars().filter(|&c| c == '\u{FFFD}').count();
        ensure!(
            !had_errors && replaced == 0 && !text.contains('\0'),
            "Corrupted UTF-16BE text stream: {label}"
        );
        return Ok(DecodedText {
            text,
            encoding: "utf-16be".into(),
            had_bom: true,
            replaced_count: 0,
        });
    }

    // 4. Reject NUL bytes in non-UTF-16 streams (binary archive/image/object)
    ensure!(
        !bytes.contains(&0),
        "Stream is binary or corrupted (contains NUL byte): {label}"
    );

    // 5. Strict UTF-8 (fast path)
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok(DecodedText {
            text: s.to_owned(),
            encoding: "utf-8".into(),
            had_bom: false,
            replaced_count: 0,
        });
    }

    // 6. Statistical encoding detection via chardetng + encoding_rs
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let guessed = detector.guess(None, false);

    let candidates = [
        guessed,
        encoding_rs::GB18030,
        encoding_rs::SHIFT_JIS,
        encoding_rs::EUC_KR,
        encoding_rs::BIG5,
        encoding_rs::WINDOWS_1252,
    ];
    for enc in candidates {
        let (cow, had_errors) = enc.decode_without_bom_handling(bytes);
        if !had_errors && !cow.contains('\u{FFFD}') && !cow.contains('\0') {
            return Ok(DecodedText {
                text: cow.into_owned(),
                encoding: enc.name().to_ascii_lowercase(),
                had_bom: false,
                replaced_count: 0,
            });
        }
    }

    bail!("Unable to cleanly decode non-UTF-8 text without replacement characters: {label}")
}

#[inline]
pub fn is_pua_char(c: char) -> bool {
    ('\u{E000}'..='\u{F8FF}').contains(&c)
        || ('\u{F0000}'..='\u{FFFFD}').contains(&c)
        || ('\u{100000}'..='\u{10FFFD}').contains(&c)
}

#[inline]
pub fn is_disallowed_control_char(c: char) -> bool {
    c == '\0' || (c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
}

/// Count unmistakable UTF-8-decoded-as-Latin1/CP1252 sequences (e.g. `Ã©`, `â€”`, `Â°`, `ï»¿`).
/// Designed for zero false positives on legitimate Portuguese/Nordic/European academic text.
fn count_utf8_as_latin1_mojibake(text: &str) -> usize {
    let mut hits = 0usize;
    hits += text.matches("ï»¿").count();
    hits += rx!(r"â€[\u{0080}-\u{00BF}\u{2018}-\u{2026}\u{2122}\x22]")
        .find_iter(text)
        .count();
    hits += rx!(r"Ã[\u{0080}-\u{00BF}]").find_iter(text).count();
    hits += rx!(r"Â[\u{00A0}¡¢£¤¥¦§¨©ª«¬\u{00AD}®¯°±²³´µ¶·¸¹º»¼½¾¿]")
        .find_iter(text)
        .count();
    hits
}

fn count_gbk_mojibake(text: &str) -> usize {
    let mut hits = 0usize;
    // `锟斤拷` is the classic UTF-8 U+FFFD x2 decoded as GBK signature.
    hits += text.matches("锟斤拷").count();
    // `锘?` is UTF-8 BOM decoded as GBK before ASCII/punctuation.
    hits += text.matches("锘?").count();
    hits
}

fn count_cid_markers(text: &str) -> usize {
    rx!(r"\(cid:\d+\)").find_iter(text).count()
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TextQualityReport {
    pub replacement_count: usize,
    pub null_count: usize,
    pub control_count: usize,
    pub pua_count: usize,
    pub cid_marker_count: usize,
    pub utf8_as_latin1_hits: usize,
    pub gbk_mojibake_hits: usize,
    pub is_garbled: bool,
    pub reasons: Vec<String>,
}

impl TextQualityReport {
    pub fn to_json(&self) -> Value {
        json!({
            "replacement_count": self.replacement_count,
            "null_count": self.null_count,
            "control_count": self.control_count,
            "pua_count": self.pua_count,
            "cid_marker_count": self.cid_marker_count,
            "utf8_as_latin1_hits": self.utf8_as_latin1_hits,
            "gbk_mojibake_hits": self.gbk_mojibake_hits,
            "is_garbled": self.is_garbled,
            "reasons": self.reasons,
        })
    }
}

/// Layer-4 Unicode & encoding Mojibake detector for extracted PDF text,
/// OCR hypotheses, and multi-format document blocks.
pub fn analyze_text(text: &str) -> TextQualityReport {
    let mut r = TextQualityReport::default();
    for c in text.chars() {
        if c == '\u{FFFD}' {
            r.replacement_count += 1;
        } else if c == '\0' {
            r.null_count += 1;
        } else if is_disallowed_control_char(c) {
            r.control_count += 1;
        } else if is_pua_char(c) {
            r.pua_count += 1;
        }
    }
    r.cid_marker_count = count_cid_markers(text);
    r.utf8_as_latin1_hits = count_utf8_as_latin1_mojibake(text);
    r.gbk_mojibake_hits = count_gbk_mojibake(text);

    if r.replacement_count > 0 {
        r.reasons.push("replacement_char".into());
    }
    if r.null_count > 0 {
        r.reasons.push("null_byte".into());
    }
    if r.control_count > 0 {
        r.reasons.push("disallowed_control_char".into());
    }
    if r.pua_count > 0 {
        r.reasons.push("private_use_area_glyph".into());
    }
    if r.cid_marker_count > 0 {
        r.reasons.push("unresolved_cid_token".into());
    }
    if r.utf8_as_latin1_hits > 0 {
        r.reasons.push("utf8_latin1_mojibake".into());
    }
    if r.gbk_mojibake_hits > 0 {
        r.reasons.push("gbk_mojibake".into());
    }
    r.is_garbled = !r.reasons.is_empty();
    r
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteMojibakeHit {
    pub code: &'static str,
    pub line: usize,
    pub message: String,
}

/// Hard publish-gate scanner for final Obsidian Markdown notes.
/// Every returned hit is promoted to an error-level finding in `validate.rs`,
/// blocking `validate_note`, `accept_note`, and `vault_publish` commits.
pub fn scan_note_lines(lines: &[&str]) -> Vec<NoteMojibakeHit> {
    let mut hits = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        if line.contains('\u{FFFD}') {
            hits.push(NoteMojibakeHit {
                code: "MOJIBAKE_REPLACEMENT_CHAR",
                line: line_no,
                message: "检测到乱码替换字符 U+FFFD（），禁止发布；请通过高清视觉核对或重新 OCR 修正。".into(),
            });
        }
        if line.chars().any(is_disallowed_control_char) {
            hits.push(NoteMojibakeHit {
                code: "MOJIBAKE_CONTROL_CHAR",
                line: line_no,
                message: "检测到非法控制字符或 NUL 字节，疑似编码损坏，禁止发布。".into(),
            });
        }
        if line.chars().any(is_pua_char) {
            hits.push(NoteMojibakeHit {
                code: "MOJIBAKE_PUA_GLYPH",
                line: line_no,
                message: "检测到私有区字符（PUA，U+E000–U+F8FF 等，常见于 PDF ToUnicode CMap 缺失导致的字形乱码），禁止发布；请对照渲染图修正。".into(),
            });
        }
        if count_cid_markers(line) > 0
            || count_utf8_as_latin1_mojibake(line) > 0
            || count_gbk_mojibake(line) > 0
        {
            hits.push(NoteMojibakeHit {
                code: "MOJIBAKE_ENCODING_ARTIFACT",
                line: line_no,
                message: "检测到典型编码乱码特征（(cid:N) / UTF-8-Latin1 错解 / 锟斤拷），禁止发布。".into(),
            });
        }
        if line.contains("<!-- VISUAL_REQUIRED:") {
            hits.push(NoteMojibakeHit {
                code: "UNRESOLVED_VISUAL_REQUIRED",
                line: line_no,
                message: "存在尚未完成视觉核对的 <!-- VISUAL_REQUIRED: ... --> 占位标记，请对照 ocr-verify-sheet.png 补全并移除该标记后再发布。".into(),
            });
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_bom_utf16_gbk_shiftjis_and_windows1252_without_mojibake() {
        // UTF-8 BOM
        let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
        utf8_bom.extend_from_slice("海洋重力波 α+β".as_bytes());
        let d = decode_text_bytes(&utf8_bom, "bom.tex").unwrap();
        assert_eq!(d.text, "海洋重力波 α+β");
        assert!(d.had_bom);
        assert_eq!(d.encoding, "utf-8");

        // UTF-16LE BOM
        let mut utf16le = vec![0xFF, 0xFE];
        for u in "公式 E=mc² 海洋".encode_utf16() {
            utf16le.extend_from_slice(&u.to_le_bytes());
        }
        let d = decode_text_bytes(&utf16le, "utf16.txt").unwrap();
        assert_eq!(d.text, "公式 E=mc² 海洋");
        assert_eq!(d.encoding, "utf-16le");

        // GBK / GB18030 Chinese academic TeX
        let src_zh = "\\section{海洋内波与非线性动力学的数值模拟研究进展}\n本文系统讨论了不同层结条件下内孤立波的传播演化特征与能量耗散机制。";
        let (gbk_bytes, _, had_err) = encoding_rs::GB18030.encode(src_zh);
        assert!(!had_err);
        let d = decode_text_bytes(&gbk_bytes, "zh-gbk.tex").unwrap();
        assert_eq!(d.text, src_zh);
        assert!(!analyze_text(&d.text).is_garbled);

        // Shift-JIS Japanese
        let src_ja = "海洋重力波の数値シミュレーションと境界条件の解析手法について検討する。";
        let (sjis_bytes, _, had_err) = encoding_rs::SHIFT_JIS.encode(src_ja);
        assert!(!had_err);
        let d = decode_text_bytes(&sjis_bytes, "ja-sjis.txt").unwrap();
        assert_eq!(d.text, src_ja);

        // Windows-1252 European accents
        let src_eu = "Schröder, Müller, Poincaré, and L'Hôpital studied waves.";
        let (w1252, _, had_err) = encoding_rs::WINDOWS_1252.encode(src_eu);
        assert!(!had_err);
        let d = decode_text_bytes(&w1252, "eu.tex").unwrap();
        assert_eq!(d.text, src_eu);

        // Reject binary & PDF
        assert!(decode_text_bytes(b"%PDF-1.7\n...", "bad.tex").is_err());
        assert!(decode_text_bytes(b"abc\0def", "bin.tex").is_err());
    }

    #[test]
    fn detects_mojibake_without_false_positives_on_clean_academic_text() {
        let clean = "Ocean internal waves (海洋内波) — Ångström, São Paulo, Müller, Poincaré, α + β ≤ γ, E = mc².";
        let r = analyze_text(clean);
        assert!(!r.is_garbled, "{r:?}");

        assert!(analyze_text("broken \u{FFFD} glyph").is_garbled);
        assert!(analyze_text("pua \u{E001} math").is_garbled);
        assert!(analyze_text("font (cid:42) missing").is_garbled);
        assert!(analyze_text("SchrÃ¶der and PoincarÃ©").is_garbled);
        assert!(analyze_text("出现锟斤拷乱码").is_garbled);
        assert!(analyze_text("bad\x07control").is_garbled);

        let lines = vec![
            "# Clean Title",
            "Line with \u{FFFD} char",
            "<!-- VISUAL_REQUIRED: main-p1-b3 | reason=garbled_glyphs | page=1 | bbox=[10,20,100,40] -->",
            "Line with \u{E012} PUA",
        ];
        let hits = scan_note_lines(&lines);
        let codes: Vec<_> = hits.iter().map(|h| h.code).collect();
        assert_eq!(
            codes,
            vec![
                "MOJIBAKE_REPLACEMENT_CHAR",
                "UNRESOLVED_VISUAL_REQUIRED",
                "MOJIBAKE_PUA_GLYPH"
            ]
        );
    }
}
