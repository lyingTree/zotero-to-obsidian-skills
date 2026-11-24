//! Explicit text-layer/page-image conflict records, shared by future mapping
//! validation and the native record writer. No OCR or visual truth inference.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::{HashMap,HashSet},path::{Path,PathBuf}};
use crate::{check_ledger,check_write,files,path,path_text,pdf_structure::Pdf};

fn nonempty(v:&Value)->bool{v.as_str().is_some_and(|s|!s.trim().is_empty())}
fn coordinates()->Value{json!({"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"})}
fn index<'a>(v:&'a Value,label:&str)->Result<HashMap<&'a str,&'a Value>>{
    let mut m=HashMap::new();
    for row in v.as_array().with_context(||format!("{label} must be an array"))?{
        ensure!(row.is_object(),"{label} must contain objects");
        let id=row["id"].as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("{label} needs an actual ID"))?;
        ensure!(m.insert(id,row).is_none(),"Duplicate {label} ID: {id}");
    }
    Ok(m)
}
fn bound(v:&Value,base:&Path)->Result<Value>{
    ensure!(nonempty(&v["path"]),"MAP_CONFLICT_FILE: file path required");
    let p=base.join(path(&v["path"])?);
    let b=check_ledger::bind_file(&p)?;
    ensure!(b["sha256"]==v["sha256"],"MAP_BINDING_HASH: {}",p.display());
    Ok(b)
}
fn matched(record:&Value,expected:&Value,base:&Path)->Result<Value>{
    ensure!(record.as_object().is_some_and(|o|o.len()==2&&o.contains_key("path")&&o.contains_key("sha256")),
        "MAP_CONFLICT_FILE: minimal path/sha256 record required");
    let a=bound(record,base)?;let b=bound(expected,base)?;
    ensure!(a==b,"MAP_BINDING_TARGET: conflict evidence does not match registered page/source");
    Ok(a)
}

// Python splitlines(keepends=True) also recognizes the Unicode separators.
fn lines(text:&str)->Vec<&str>{
    let mut out=Vec::new();
    let mut start=0;
    let mut iter=text.char_indices().peekable();
    while let Some((i,c))=iter.next(){
        if matches!(c,'\n'|'\r'|'\u{b}'|'\u{c}'|'\u{1c}'|'\u{1d}'|'\u{1e}'|'\u{85}'|'\u{2028}'|'\u{2029}'){
            let mut end=i+c.len_utf8();
            if c=='\r'&&iter.peek().is_some_and(|(_,next)|*next=='\n'){
                end=iter.next().unwrap().0+1;
            }
            out.push(&text[start..end]);start=end;
        }
    }
    if start<text.len(){out.push(&text[start..]);}out
}
pub fn text_matches(text:&str,locator:&Value,reading:&Value)->bool{
    let Some(reading)=reading.as_str().filter(|s|!s.is_empty()) else{return false;};
    let Some(start)=locator["start"].as_u64().and_then(|n|usize::try_from(n).ok()) else{return false;};
    let Some(end)=locator["end"].as_u64().and_then(|n|usize::try_from(n).ok()) else{return false;};
    match locator["kind"].as_str(){
        Some("characters")=>{
            if locator["unit"]!="unicode_code_points"||locator["end_exclusive"]!=true||start>=end{return false;}
            let mut bounds:Vec<_>=text.char_indices().map(|(i,_)|i).collect();
            bounds.push(text.len());
            if end>=bounds.len(){return false;}
            &text[bounds[start]..bounds[end]]==reading
        }
        Some("lines")=>{
            let lines=lines(text);
            start>=1&&start<=end&&end<=lines.len()&&lines[start-1..end].concat().contains(reading)
        }
        _=>false,
    }
}
fn page_index(source:&Value)->Result<HashMap<u64,&Value>>{
    let mut pages=HashMap::new();
    for row in source["pages"].as_array().context("MAP_CONFLICT_PAGE: source page inventory required")?{
        let n=row["page"].as_u64().filter(|n|*n>0).context("MAP_CONFLICT_PAGE: actual page number required")?;
        ensure!(pages.insert(n,row).is_none(),"MAP_CONFLICT_PAGE: duplicate page");
    }
    Ok(pages)
}
pub(crate) fn validate_records(unit:&Value,source:&Value,source_file:&Value,base:&Path,
                    pdf:&Pdf,records:&Value,require_resolved:bool)->Result<Vec<Value>>{
    let mut checked=Vec::new();
    let records=index(records,"text_visual_conflicts")?;
    let pages=page_index(source)?;
    let unit_pages:HashSet<_>=unit["evidence"].as_array().context("MAP_CONFLICT_PAGE: unit evidence required")?
        .iter().filter_map(|v|v["page"].as_u64()).collect();
    for record in records.values(){
        if let Some(b)=record.get("source_file"){checked.push(matched(b,source,base)?);}
        let (layer,visible,decision)=(&record["text_layer"],&record["visible_page"],&record["decision"]);
        ensure!(layer.is_object()&&visible.is_object()&&decision.is_object(),
            "MAP_CONFLICT_RECORD: text_layer, visible_page and explicit decision required");
        let page=visible["page"].as_u64().filter(|n|unit_pages.contains(n)&&pages.contains_key(n))
            .context("MAP_CONFLICT_PAGE: conflict must belong to an actual page of the unit")?;
        let page_row=pages[&page];
        let text_binding=matched(&layer["file"],&page_row["text"],base)?;
        checked.push(matched(&visible["file"],&page_row["render"],base)?);
        let (data,actual)=files::read_bound(&path(&text_binding["path"])?)?;
        ensure!(actual==text_binding,"MAP_BINDING_HASH: page text changed");
        ensure!(text_matches(std::str::from_utf8(&data)?,&layer["locator"],&layer["text"]),
            "MAP_CONFLICT_TEXT: reading must occur at the exact recorded file range");
        checked.push(text_binding);
        ensure!(pdf.contains_bbox(page,&visible["bbox"])?,"MAP_CONFLICT_LOCATION: bbox must be inside actual PDF page");
        ensure!(visible.get("coordinate_system").is_none()||visible["coordinate_system"]==coordinates(),
            "MAP_CONFLICT_LOCATION: invalid unrotated PDF-point coordinate system");
        let status=decision["status"].as_str().filter(|s|matches!(*s,"pending"|"resolved"|"ambiguous"))
            .context("MAP_CONFLICT_DECISION: explicit pending, ambiguous, or resolved decision required")?;
        ensure!(nonempty(&decision["reason"]),"MAP_CONFLICT_REASON: explicit reason required");
        if status=="resolved"{
            ensure!(visible["text"].is_string()&&decision["selected_text"].is_string()&&
                (decision["selected_text"]==layer["text"]||decision["selected_text"]==visible["text"]),
                "MAP_CONFLICT_READING: resolved decision must retain and choose one explicit reading");
            ensure!(decision.get("observed").is_none()||decision["observed"]==true,
                "MAP_CONFLICT_DECISION: a resolved reading cannot declare not observed");
        }else{
            ensure!(visible["text"].is_null()||visible["text"].is_string(),
                "MAP_CONFLICT_READING: visible reading must be text or null");
            ensure!(!require_resolved,"MAP_CONFLICT_UNRESOLVED: unresolved explicit decision");
        }
    }
    checked.push(source_file.clone());
    Ok(checked)
}

pub struct Conflict<'a>{
    pub conflict_id:&'a str,pub page_number:u64,pub text_locator:&'a Value,
    pub text_layer_text:&'a Value,pub visible_text:&'a Value,pub bbox:&'a Value,pub decision:&'a Value
}
pub fn record(inventory_path:&Path,unit_id:&str,opt:Conflict<'_>)->Result<Value>{
    check_write::no_link(inventory_path)?;
    let p=files::canonical(inventory_path)?;
    let _lock=check_write::lock_target(&p)?;
    let (raw,binding)=files::read_bound(&p)?;
    let mut inventory=crate::parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(inventory.is_object()&&inventory["schema_version"].as_u64()==Some(1),"Expected source inventory schema_version 1");
    ensure!(!unit_id.trim().is_empty()&&!opt.conflict_id.trim().is_empty(),"Supply actual nonempty source-unit and conflict IDs");
    let units=index(&inventory["units"],"inventory.units")?;
    let sources=index(&inventory["sources"],"inventory.sources")?;
    let unit=units.get(unit_id).context("Choose an actual unit with a registered source")?;
    let sid=unit["source"].as_str().context("Choose an actual unit with a registered source")?;
    let source=sources.get(sid).context("Choose an actual unit with a registered source")?;
    let base=p.parent().context("inventory parent")?;
    let source_file=bound(source,base)?;
    let pdf=Pdf::open(&path(&source_file["path"])?)?;
    let pages=page_index(source)?;
    let page=pages.get(&opt.page_number).context("Choose an actual page from this unit's registered source")?;
    let mut records=match unit.get("text_visual_conflicts"){
        Some(v)=>v.as_array().context("Retain text_visual_conflicts as an array of explicit records")?.clone(),
        None=>vec![],
    };
    ensure!(!records.iter().any(|v|v["id"]==opt.conflict_id),
        "This conflict ID already exists; retain its single authoritative record");
    ensure!(opt.text_locator.is_object()&&opt.decision.is_object()&&opt.bbox.is_array(),
        "Supply an explicit text locator, visible-page bbox and human decision record");
    let record=json!({"id":opt.conflict_id,"source_file":source_file,
        "text_layer":{"file":bound(&page["text"],base)?,"locator":opt.text_locator,"text":opt.text_layer_text},
        "visible_page":{"file":bound(&page["render"],base)?,"page":opt.page_number,
            "bbox":opt.bbox,"coordinate_system":coordinates(),"text":opt.visible_text},
        "decision":opt.decision});
    records.push(record.clone());let records=Value::Array(records);
    let verified=validate_records(unit,source,&source_file,base,&pdf,&records,false)?;
    for b in &verified{
        ensure!(check_ledger::bind_file(&path(&b["path"])?)?==*b,"Conflict evidence changed before publication");
    }
    let unit=inventory["units"].as_array_mut().unwrap().iter_mut()
        .find(|v|v["id"]==unit_id).context("unit disappeared")?;
    unit["text_visual_conflicts"]=records;
    let mut bytes=serde_json::to_vec_pretty(&inventory)?;bytes.push(b'\n');
    check_write::replace_bytes(&p,binding["sha256"].as_str().unwrap(),&bytes)?;
    Ok(record)
}
