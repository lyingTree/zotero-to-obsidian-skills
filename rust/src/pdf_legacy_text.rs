//! Read-only crosswalk of historic PDF text selectors, not an acceptance gate.
use crate::{check_ledger, check_write, files, parse_json, path, pdf_inventory, pdf_legacy_images::LegacyImages};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, io::Read, path::Path};

const MAX: u64 = 32 * 1024 * 1024;
type Bounds = [f64;4];
fn open(path: &Path, expected: &str) -> Result<(Value,Value)> {
    ensure!(expected.len()==64 && expected.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "Provide an independently trusted inventory SHA-256");
    check_write::no_link(path)?;
    let actual=files::canonical(path)?;
    let file=fs::File::open(&actual)?;
    let meta=file.metadata()?;
    ensure!(meta.is_file() && meta.len()<=MAX,"Inventory exceeds read budget");
    let mut data=Vec::with_capacity(meta.len() as usize);
    file.take(MAX+1).read_to_end(&mut data)?;
    ensure!(data.len() as u64<=MAX && files::digest(&data)==expected,"PDF crosswalk inventory hash mismatch");
    let binding=check_ledger::bind_file(path)?;
    ensure!(binding["sha256"]==expected,"PDF crosswalk inventory changed");
    let inv=parse_json(std::str::from_utf8(&data)?)?;
    ensure!(inv["schema_version"].as_u64()==Some(1),"Expected source inventory schema 1");
    Ok((inv,binding))
}
fn source<'a>(inv:&'a Value,id:&str)->Result<&'a Value>{
    let mut rows=inv["sources"].as_array().context("Inventory sources required")?.iter().filter(|r|r["id"]==id);
    let row=rows.next().context("Source ID absent from inventory")?;
    ensure!(rows.next().is_none(),"Duplicate source ID");
    Ok(row)
}
fn rect(v:&Value)->Option<Bounds>{
    let a=v.as_array().filter(|a|a.len()==4)?;
    let mut b=[0.;4];
    for(i,x)in a.iter().enumerate(){b[i]=x.as_f64().filter(|x|x.is_finite())?;}
    (b[0]<b[2]&&b[1]<b[3]).then_some(b)
}
fn overlaps(a:Bounds,b:Bounds)->bool{
    a[0].max(b[0])<a[2].min(b[2])&&a[1].max(b[1])<a[3].min(b[3])
}
const PT_ROUNDING:f64=0.01;
struct Block<'a>{ id:&'a str, tokens:Vec<&'a str>, bbox:Bounds }
fn blocks<'a>(p:&'a Value,prefix:&str,frame:Bounds,native:bool)->Option<Vec<Block<'a>>>{
    let mut ids=BTreeSet::new();let mut values=BTreeSet::new();let mut result=vec![];
    for row in p["blocks"].as_array()? {
        let id=row["id"].as_str()?;
        let suffix=id.strip_prefix(prefix)?;
        if suffix.is_empty() || !suffix.bytes().all(|b|b.is_ascii_digit()) || !ids.insert(id){return None;}
        if native&&(row["basis"]!="pdfcraft_native_line"||
                   !row["unlocated_glyph_indices"].as_array().is_some_and(Vec::is_empty)){return None;}
        // Only layout separators may vary. No case, punctuation, ligature,
        // Unicode, or hyphen normalization. Original bytes remain accessible.
        let tokens=row["text"].as_str()?.split([' ','\t','\r','\n']).filter(|x|!x.is_empty()).collect::<Vec<_>>();
        if tokens.is_empty()||!values.insert(tokens.clone()){return None;}
        let b=rect(&row["bbox"])?;
        if b[0]<frame[0]-PT_ROUNDING||b[1]<frame[1]-PT_ROUNDING
            ||b[2]>frame[2]+PT_ROUNDING||b[3]>frame[3]+PT_ROUNDING{return None;}
        result.push(Block{id,tokens,bbox:b});
    }
    Some(result)
}
fn map_page(legacy:&Value,native:&Value,id:&str,n:u64)->Option<Vec<Value>>{
    let a=&legacy["image_index"];let b=&native["image_index"];
    let frame=rect(&b["page_bounds"])?;let old_frame=rect(&a["page_bounds"])?;
    let coords=json!({"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"});
    if a["coordinate_system"]!=coords||b["coordinate_system"]!=coords
        ||!a["page_rotation"].is_u64()||a["page_rotation"]!=b["page_rotation"]
        ||old_frame.iter().zip(frame).any(|(x,y)|(x-y).abs()>PT_ROUNDING){return None;}
    let old=blocks(legacy,&format!("{id}-p{n}-b"),frame,false)?;
    let new=blocks(native,&format!("{id}-p{n}-pc-line-"),frame,true)?;
    if old.is_empty()||new.is_empty()||
        !old.iter().flat_map(|b|&b.tokens).eq(new.iter().flat_map(|b|&b.tokens)){return None;}
    let (mut j,mut k,mut offset)=(0,0,0);let mut groups=vec![];let mut group_text=BTreeSet::new();
    while j<old.len()&&k<new.len(){
        let (js,ks)=(j,k);let (mut x,mut y)=(old[j].tokens.len(),new[k].tokens.len());
        j+=1;k+=1;
        while x!=y{
            if x<y{x+=old.get(j)?.tokens.len();j+=1;}
            else{y+=new.get(k)?.tokens.len();k+=1;}
            if j-js>1&&k-ks>1{return None;}
        }
        let (a,b)=(&old[js..j],&new[ks..k]);
        if !a.iter().all(|i|b.iter().all(|o|overlaps(i.bbox,o.bbox)))
            ||!group_text.insert(a.iter().flat_map(|i|i.tokens.iter().copied()).collect::<Vec<_>>()){return None;}
        groups.push(json!({"old_block_ids":a.iter().map(|i|i.id).collect::<Vec<_>>(),
            "native_block_ids":b.iter().map(|i|i.id).collect::<Vec<_>>(),"page_token_range":[offset,offset+x]}));
        offset+=x;
    }
    (j==old.len()&&k==new.len()&&!groups.is_empty()).then_some(groups)
}
fn report_page(old:&Value,new:&Value,id:&str,n:u64)->Value{
    let ev=&new["pdf_evidence"];
    let (status,reason,groups)=if ev["ocr"]["status"]!="native_text_candidate"||
        ev["classification"]["region_review_required"]!=false||
        !ev["unlocated_native_lines"].as_array().is_some_and(Vec::is_empty){
        ("pending","ocr_or_regional_review_required",vec![])
    }else if let Some(g)=map_page(old,new,id,n){
        ("mechanical_text_correspondence","unique_monotone_whole_block_groups",g)
    }else{("pending","unavailable_or_disputed_text_geometry_or_identity",vec![])};
    json!({"page":n,"text_status":status,"reason":reason,"groups":groups,
        "visual":"pending","reading_order":"pending","human_review":"not_transferred"})
}
pub fn crosswalk(old_path:&Path,old_sha:&str,new_path:&Path,new_sha:&str,id:&str,
    progress:impl FnMut(usize,usize,&str))->Result<Value>{
    ensure!(old_sha!=new_sha,"Provide distinct historical and native inventories");
    let (old,old_binding)=open(old_path,old_sha)?;let (new,new_binding)=open(new_path,new_sha)?;
    let (old,new)=(source(&old,id)?,source(&new,id)?);
    ensure!(old["inventory_backend"].is_null()&&(old["format"].is_null()||old["format"]=="pdf")&&
        new["format"]=="pdf"&&new["inventory_backend"]==pdf_inventory::BACKEND,
        "Expected legacy PDF source and native PDF inventory");
    ensure!(old["sha256"]==new["sha256"]&&old["role"].is_string()&&old["role"]==new["role"],
        "Legacy PDF source hash or role differs from native source");
    let pdf=path(&new["path"])?;
    check_write::no_link(&pdf)?;
    ensure!(fs::metadata(&pdf)?.len()<=512*1024*1024,"PDF exceeds crosswalk read budget");
    let original=files::snapshot(&pdf)?;
    ensure!(original["sha256"]==new["sha256"],"PDF source bytes differ");
    let replay=pdf_inventory::replay(new,&pdf,progress)?;
    let (a,b)=(old["pages"].as_array().context("Legacy pages required")?,
               new["pages"].as_array().context("Native pages required")?);
    ensure!(a.len()==b.len()&&old["page_count"].as_u64()==Some(a.len() as u64),"Legacy PDF page count differs");
    let images=LegacyImages::new(old,a);
    let mut pages=Vec::with_capacity(a.len());
    for(i,(a,b))in a.iter().zip(b).enumerate(){
        let n=(i+1) as u64;
        ensure!(a["page"].as_u64()==Some(n)&&b["page"].as_u64()==Some(n),"Page order differs");
        let mut page=report_page(a,b,id,n);
        page["image_correspondence"]=images.report(a,b);
        pages.push(page);
    }
    ensure!(check_ledger::bind_file(old_path)?==old_binding&&check_ledger::bind_file(new_path)?==new_binding
        &&files::snapshot(&pdf)?==original,"Crosswalk inputs changed");
    Ok(json!({"kind":"pdf-legacy-text-crosswalk/v1","schema_version":1,"source_file":original,
        "source_id":id,"source_role":new["role"],"legacy_inventory":old_binding,"native_inventory":new_binding,
        "native_backend_revision":new["backend_revision"],"native_replay":replay,"pages":pages,
        "text_policy":"exact_tokens_ascii_layout_whitespace_only","range_policy":"zero_based_half_open_page_tokens",
        "legacy_engine_provenance":"not_verified","legacy_raster_provenance":"not_assessed",
        "source_fidelity":"not_assessed","source_checks_complete":false,
        "delivery_ready":false,"evidence_boundary":"Mechanical selectors only. No legacy raster, visual/OCR, reading order, human review or acceptance is established."}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(prefix: &str, texts: &[&str], native: bool) -> Value {
        let coords=json!({"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"});
        json!({"image_index":{"coordinate_system":coords,"page_rotation":0,"page_bounds":[0,0,200,100]},
            "blocks":texts.iter().enumerate().map(|(i,t)|json!({
                "id":format!("main-p1-{prefix}{i}"),"bbox":[1,1,10,10],"text":t,
                "basis":if native {"pdfcraft_native_line"}else{"legacy"},
                "unlocated_glyph_indices":[]
            })).collect::<Vec<_>>()})
    }

    #[test]
    fn refuses_ambiguity_and_preserves_meaningful_codepoints() {
        let legacy=page("b",&["A  B\tC"],false);
        let mut native=page("pc-line-",&["A","B C"],true);
        assert!(map_page(&legacy,&native,"main",1).is_some());
        native["blocks"][0]["bbox"]=json!([100,40,150,41]);
        assert!(map_page(&legacy,&native,"main",1).is_none());
        for (old,new) in [
            (vec!["A","A"],vec!["A","A"]),
            (vec!["A B","C"],vec!["A","B C"]),
            (vec!["A-B"],vec!["A B"]),
            (vec!["A\u{a0}B"],vec!["A B"]),
            (vec!["é"],vec!["e\u{301}"]),
        ] {
            assert!(map_page(&page("b",&old,false),&page("pc-line-",&new,true),"main",1).is_none());
        }
    }
}
