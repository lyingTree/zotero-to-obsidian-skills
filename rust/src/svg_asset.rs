//! Offline validation of an explicitly bounded static SVG subset.
use anyhow::{ensure, Context, Result};
use resvg::{tiny_skia, usvg};
use svg_xml::{Document, Node, ParsingOptions};
use std::{collections::{HashMap, HashSet}, sync::OnceLock};

pub(crate) const MAX_BYTES: usize = 1024 * 1024;
const MAX_NODES: u32 = 8_192;
const MAX_ELEMENTS: usize = 2_048;
const MAX_DEPTH: usize = 32;
const MAX_GEOMETRY: usize = 160_000;
const MAX_USES: usize = 512;
const MAX_STOPS: usize = 64;
const MAX_SIDE: f32 = 20_000.;
const MAX_PIXELS: f32 = 40_000_000.;
const RENDER_SIDE: f32 = 384.;
const SVG: &str = "http://www.w3.org/2000/svg";
const XLINK: &str = "http://www.w3.org/1999/xlink";

#[derive(Clone, Copy)]
enum Reference {
    Paint,
    Clip,
    Shape,
}

fn is_shape(tag: &str) -> bool {
    matches!(tag, "path" | "rect" | "circle" | "ellipse" | "line" | "polygon" | "polyline")
}
fn definition(node: Node<'_, '_>) -> bool {
    node.parent_element().is_some_and(|p| p.tag_name().name() == "defs")
}
fn fragment(value: &str) -> Result<&str> {
    let id = value.strip_prefix('#').context("FIGURE_SVG_EXTERNAL_REFERENCE: fragment required")?;
    ensure!(valid_id(id), "FIGURE_SVG_EXTERNAL_REFERENCE: invalid fragment");
    Ok(id)
}
fn valid_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() <= 96
        && b.first().is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(c))
}
fn presentation(name: &str) -> bool {
    matches!(name, "fill" | "fill-opacity" | "fill-rule" | "stroke" | "stroke-width" |
        "stroke-opacity" | "stroke-linecap" | "stroke-linejoin" | "stroke-miterlimit" |
        "opacity" | "display" | "visibility" | "color" | "clip-path" | "clip-rule")
}
fn paint_property(tag: &str, name: &str) -> bool {
    if tag == "stop" {
        return matches!(name, "stop-color" | "stop-opacity");
    }
    (is_shape(tag) || matches!(tag, "svg" | "g" | "use"))
        && presentation(name)
        && (name != "opacity" || is_shape(tag) || tag == "use")
}
fn geometry_attribute(tag: &str, name: &str) -> bool {
    match tag {
        "svg" => matches!(name, "width" | "height" | "viewBox" | "preserveAspectRatio" | "version"),
        "linearGradient" => matches!(name,"x1" | "x2" | "y1" | "y2" | "gradientUnits" | "gradientTransform"),
        "radialGradient" => matches!(name,"cx" | "cy" | "r" | "fx" | "fy" | "gradientUnits" | "gradientTransform"),
        "stop" => name == "offset",
        "clipPath" => matches!(name, "clipPathUnits" | "transform"),
        "g" => name == "transform",
        "use" => matches!(name, "x" | "y" | "transform"),
        "path" => matches!(name,"d" | "transform"),
        "line" => matches!(name,"x1" | "x2" | "y1" | "y2" | "transform"),
        "circle" => matches!(name,"cx" | "cy" | "r" | "transform"),
        "ellipse" => matches!(name,"cx" | "cy" | "rx" | "ry" | "transform"),
        "rect" => matches!(name,"x" | "y" | "width" | "height" | "rx" | "ry" | "transform"),
        "polygon" | "polyline" => matches!(name,"points" | "transform"),
        _ => false,
    }
}
fn geometry_value(name: &str, value: &str) -> Result<()> {
    let limit = if matches!(name, "d" | "points") { 32_768 } else { 512 };
    ensure!(value.len() <= limit, "FIGURE_SVG_BUDGET: geometry attribute limit");
    if matches!(name, "version" | "clipPathUnits" | "gradientUnits" | "preserveAspectRatio") {
        return Ok(());
    }
    static NUMBERS: OnceLock<regex::Regex> = OnceLock::new();
    let re = NUMBERS.get_or_init(|| regex::Regex::new(r"(?i)[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:e[+-]?[0-9]+)?").unwrap());
    ensure!(re.find_iter(value).all(|token|token.as_str().parse::<f64>().is_ok_and(|v|v.is_finite()&&v.abs()<=1_000_000.)),
        "FIGURE_SVG_BUDGET: geometry coefficient exceeds bounds");
    let valid = match name {
        "d" => !value.trim().is_empty() && svgtypes::PathParser::from(value).all(|v| v.is_ok()),
        "points" => {
            let mut count = 0usize;
            svgtypes::NumberListParser::from(value).all(|v|{count+=1;v.is_ok()}) && count >= 4 && count.is_multiple_of(2)
        },
        "transform" | "gradientTransform" => svgtypes::TransformListParser::from(value).all(|v|v.is_ok()),
        "viewBox" => value.parse::<svgtypes::ViewBox>().is_ok(),
        _ => value.parse::<svgtypes::Length>().is_ok(),
    };
    ensure!(valid, "FIGURE_SVG_XML: malformed geometry value in {name}");
    Ok(())
}
fn clipped_definition(node: Node<'_, '_>) -> bool {
    definition(node) || node.parent_element().is_some_and(|p| p.tag_name().name() == "clipPath")
}
fn property<'a>(node: Node<'a, '_>, name: &str, value: &'a str, refs: &mut Vec<(Reference, &'a str)>) -> Result<()> {
    let value = value.trim();
    ensure!(!value.is_empty() && value.len() <= 512 && !value.contains(['\\', '@', '"', '\''])
        && !value.contains("/*") && !value.contains("!important"), "FIGURE_SVG_PROPERTY: invalid {name}");
    let lower = value.to_ascii_lowercase();
    if lower.contains("url") {
        let inner = value.strip_prefix("url(").and_then(|v| v.strip_suffix(')'))
            .context("FIGURE_SVG_EXTERNAL_REFERENCE: only url(#id) is supported")?;
        let kind = match name {
            "fill" | "stroke" => Reference::Paint,
            "clip-path" if !clipped_definition(node) => Reference::Clip,
            _ => anyhow::bail!("FIGURE_SVG_EXTERNAL_REFERENCE: unsupported reference in {name}"),
        };
        refs.push((kind, fragment(inner.trim())?));
    } else if name == "clip-path" {
        ensure!(value == "none", "FIGURE_SVG_PROPERTY: clip-path requires local fragment or none");
    }
    Ok(())
}
fn allow_child(tag: &str, parent: Option<&str>) -> bool {
    match tag {
        "svg" => parent.is_none(),
        "g" | "defs" | "use" => matches!(parent, Some("svg" | "g")),
        "title" | "desc" => matches!(parent, Some("svg" | "g")) || parent.is_some_and(is_shape),
        "linearGradient" | "radialGradient" | "clipPath" => parent == Some("defs"),
        "stop" => matches!(parent, Some("linearGradient" | "radialGradient")),
        _ => is_shape(tag) && matches!(parent, Some("svg" | "g" | "defs" | "clipPath")),
    }
}
fn accept_svg(doc: &Document<'_>) -> Result<()> {
    let root = doc.root_element();
    ensure!(root.tag_name().name() == "svg" && root.tag_name().namespace() == Some(SVG),
        "FIGURE_SVG_XML: expected an SVG namespace root");
    let mut ids = HashMap::new();
    let mut refs = Vec::new();
    let mut elements = 0usize;
    let mut geometry = 0usize;
    let mut uses = 0usize;
    let mut clip_nodes = HashSet::new();
    for node in doc.descendants() {
        ensure!(!node.is_pi(), "FIGURE_SVG_ACTIVE: processing instructions are not allowed");
        if node.is_text() {
            let in_description = node.parent_element().is_some_and(|p| matches!(p.tag_name().name(), "title" | "desc"));
            ensure!(in_description || node.text().is_none_or(|s| s.trim().is_empty()),
                "FIGURE_SVG_UNSUPPORTED: unexpected text outside title or desc");
        }
        if !node.is_element() { continue; }
        elements += 1;
        ensure!(elements <= MAX_ELEMENTS && node.ancestors().take(MAX_DEPTH + 1).count() <= MAX_DEPTH,
            "FIGURE_SVG_BUDGET: element or depth limit exceeded");
        let tag = node.tag_name().name();
        let parent = node.parent_element().map(|p| p.tag_name().name());
        ensure!(node.tag_name().namespace() == Some(SVG) && allow_child(tag, parent),
            "FIGURE_SVG_UNSUPPORTED: unsupported element or nesting: {tag}");
        if matches!(tag, "linearGradient" | "radialGradient") {
            ensure!(node.children().filter(Node::is_element).take(MAX_STOPS + 1).count() <= MAX_STOPS,
                "FIGURE_SVG_BUDGET: gradient stop limit exceeded");
        }
        let mut seen_properties = HashSet::new();
        let mut href_seen = false;
        let before = refs.len();
        for attr in node.attributes() {
            let name = attr.name();
            let value = attr.value();
            ensure!(attr.namespace().is_none() || (attr.namespace() == Some(XLINK) && name == "href"),
                "FIGURE_SVG_UNSUPPORTED: attribute namespace");
            match name {
                "id" => {
                    ensure!(valid_id(value) && ids.insert(value,node).is_none(), "FIGURE_SVG_XML: invalid or duplicate ID");
                }
                "href" => {
                    ensure!(tag == "use" && !href_seen, "FIGURE_SVG_EXTERNAL_REFERENCE: only one local shape href on use is supported");
                    refs.push((Reference::Shape, fragment(value)?));
                    href_seen = true;
                }
                "style" => {
                    ensure!(value.len() <= 2_048, "FIGURE_SVG_BUDGET: style length");
                    for part in value.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                        let (key, val) = part.split_once(':').context("FIGURE_SVG_PROPERTY: invalid style declaration")?;
                        let key = key.trim();
                        ensure!(paint_property(tag,key) && seen_properties.insert(key),
                            "FIGURE_SVG_UNSUPPORTED: unsupported or repeated style property {key}");
                        property(node,key,val,&mut refs)?;
                    }
                }
                _ if paint_property(tag,name) => property(node,name,value,&mut refs)?,
                _ if geometry_attribute(tag,name) => {
                    geometry_value(name,value)?;
                    if matches!(name, "d" | "points") {
                        geometry += value.len();
                        ensure!(geometry <= MAX_GEOMETRY, "FIGURE_SVG_BUDGET: geometry work limit");
                    }
                }
                _ => anyhow::bail!("FIGURE_SVG_UNSUPPORTED: attribute {tag}.{name}"),
            }
        }
        if refs[before..].iter().any(|(kind,_)| matches!(kind,Reference::Clip)) {
            ensure!(node.ancestors().skip(1).filter(|n|clip_nodes.contains(&n.id())).take(3).count()<3,
                "FIGURE_SVG_BUDGET: nested clip depth");
            clip_nodes.insert(node.id());
        }
        if tag == "use" {
            uses += 1;
            ensure!(href_seen,"FIGURE_SVG_EXTERNAL_REFERENCE: use has no local href");
            ensure!(uses <= MAX_USES,"FIGURE_SVG_BUDGET: use limit exceeded");
        }
    }
    for (kind,id) in refs {
        let target = *ids.get(id).context("FIGURE_SVG_EXTERNAL_REFERENCE: unresolved fragment")?;
        let tag = target.tag_name().name();
        let valid = definition(target) && match kind {
            Reference::Clip => tag == "clipPath",
            Reference::Paint => matches!(tag,"linearGradient" | "radialGradient"),
            Reference::Shape => is_shape(tag),
        };
        ensure!(valid, "FIGURE_SVG_EXTERNAL_REFERENCE: fragment has the wrong target type");
        if matches!(kind,Reference::Shape) {
            geometry += target.attribute("d").or_else(|| target.attribute("points")).map_or(64,str::len);
            ensure!(geometry <= MAX_GEOMETRY, "FIGURE_SVG_BUDGET: expanded geometry work limit");
        }
    }
    Ok(())
}

pub(crate) fn validate(bytes: &[u8]) -> Result<()> {
    ensure!(!bytes.is_empty() && bytes.len() <= MAX_BYTES, "FIGURE_SVG_BUDGET: byte limit exceeded");
    let xml = std::str::from_utf8(bytes).context("FIGURE_SVG_XML: UTF-8 required")?;
    let xml = xml.strip_prefix('\u{feff}').unwrap_or(xml);
    let doc = Document::parse_with_options(xml, ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_NODES,
        ..ParsingOptions::default()
    }).context("FIGURE_SVG_XML: malformed, DTD or node budget exceeded")?;
    accept_svg(&doc)?;
    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_,_,_| None),
            resolve_string: Box::new(|_,_| None),
        },
        max_depth: MAX_DEPTH as u32,
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_xmltree(&doc, &options).context("FIGURE_SVG_RENDER: cannot parse bounded vector image")?;
    let (w,h) = (tree.size().width(),tree.size().height());
    ensure!(w > 0. && h > 0. && w <= MAX_SIDE && h <= MAX_SIDE && (w*h) <= MAX_PIXELS,
        "FIGURE_SVG_BUDGET: intrinsic canvas size exceeded");
    let scale = (RENDER_SIDE / w.max(h)).min(1.);
    let mut pixmap = tiny_skia::Pixmap::new((w*scale).ceil().max(1.) as u32,(h*scale).ceil().max(1.) as u32)
        .context("FIGURE_SVG_RENDER: invalid bounded canvas")?;
    resvg::render(&tree,tiny_skia::Transform::from_scale(scale,scale),&mut pixmap.as_mut());
    ensure!(pixmap.data().as_chunks::<4>().0.iter().any(|px|px[3]!=0), "FIGURE_SVG_RENDER: no visible vector pixels");
    Ok(())
}
