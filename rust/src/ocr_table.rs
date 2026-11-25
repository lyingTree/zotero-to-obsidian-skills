//! Independent table structure and geometric cell evidence.
//! Model output is never executable HTML or an accepted Rich Table.
use anyhow::{bail,ensure,Context,Result};
use serde_json::{json,Value};
use std::collections::BTreeSet;
const MAX_SLOTS:usize=65_536;
const MAX_DIM:usize=512;
const MAX_CELLS:usize=4096;
const MAX_HTML:usize=512*1024;
pub(crate) const BOUNDARY:&str="Unverified table hypotheses. Structure tokens, geometric cell detections and OCR readings are independent. Do not infer missing cells, match unrelated boxes by index, drop unassigned readings, or treat this record as a final Rich Table.";

fn spans(mut attrs:&str)->Result<(usize,usize)>{
    let mut rs=None;let mut cs=None;
    loop{
        let before=attrs;
        attrs=attrs.trim_start();
        if attrs.is_empty(){break;}
        ensure!(before.len()!=attrs.len(),"Attributes require whitespace");
        let (name,rest)=attrs.split_once('=').context("Invalid table attribute")?;
        let dest=match name.trim_end(){"rowspan"=>&mut rs,"colspan"=>&mut cs,_=>bail!("Unexpected table attribute")};
        ensure!(dest.is_none(),"Duplicate table span");
        let rest=rest.trim_start();
        let quote=rest.chars().next().filter(|c|matches!(c,'\''|'"')).context("Table span must be quoted")?;
        let rest=&rest[quote.len_utf8()..];
        let (number,remaining)=rest.split_once(quote).context("Unclosed table span")?;
        ensure!(!number.is_empty()&&number.bytes().all(|b|b.is_ascii_digit()),"Span must be a positive integer");
        let n=number.parse::<usize>()?;
        ensure!((1..=MAX_DIM).contains(&n),"Table span outside bounds");
        *dest=Some(n);attrs=remaining;
    }
    Ok((rs.unwrap_or(1),cs.unwrap_or(1)))
}

/// Parse the actual model token stream into explicit cells. The pinned
/// upstream decoder may stop before emitting the closing body tag. Record that
/// omission only for an already closed row/cell, never synthesize data or rows.
pub fn parse_structure(raw:&str)->Result<Value>{
    ensure!(!raw.trim().is_empty()&&raw.len()<=MAX_HTML,"Empty or oversized table token stream");
    let mut input=raw.trim();
    let mut stack:Vec<&str>=Vec::new();
    let mut slots=BTreeSet::<(usize,usize)>::new();
    let mut rows=0usize;let mut cols=0usize;let mut column=1usize;
    let mut cells=Vec::new();
    let mut tables=0usize;
    while !input.is_empty(){
        ensure!(input.starts_with('<'),"Unexpected text outside structure tags");
        let end=input.find('>').context("Unclosed table tag")?;
        let tag=&input[1..end];input=input[end+1..].trim_start();
        if let Some(name)=tag.strip_prefix('/'){
            ensure!(stack.pop()==Some(name),"Mismatched closing table tag");
            continue;
        }
        let split=tag.find(char::is_whitespace).unwrap_or(tag.len());
        let (name,attrs)=tag.split_at(split);
        let parent=stack.last().copied().unwrap_or("");
        match name{
            "html"=>ensure!(parent.is_empty()&&rows==0,"Unexpected html container"),
            "body"=>ensure!(parent=="html","Unexpected body container"),
            "table"=>{ensure!(matches!(parent,""|"body")&&rows==0&&tables==0,"Multiple or nested tables");tables+=1;},
            "thead"|"tbody"|"tfoot"=>ensure!(matches!(parent,""|"table"),"Unexpected table section"),
            "tr"=>{
                ensure!(matches!(parent,""|"table"|"thead"|"tbody"|"tfoot"),"Unexpected table row");
                rows+=1;column=1;ensure!(rows<=MAX_DIM,"Table row limit");
            },
            "td"|"th"=>{
                ensure!(parent=="tr"&&cells.len()<MAX_CELLS,"Unexpected cell or cell budget exceeded");
                let (rs,cs)=spans(attrs)?;
                while slots.contains(&(rows,column)){column+=1;}
                let bottom=rows.checked_add(rs-1).context("Row span overflow")?;
                let right=column.checked_add(cs-1).context("Column span overflow")?;
                ensure!(bottom<=MAX_DIM&&right<=MAX_DIM,"Table grid exceeds bounds");
                for r in rows..=bottom{
                    for c in column..=right{
                        ensure!(slots.len()<MAX_SLOTS,"Table slot budget exceeded");
                        ensure!(slots.insert((r,c)),"Overlapping spans in model structure");
                    }
                }
                cells.push(json!({"index":cells.len(),"row":rows,"column":column,
                    "rowspan":rs,"colspan":cs,"role":if name=="th"{"header"}else{"cell"}}));
                cols=cols.max(right);column=right+1;
            },
            _=>bail!("Unexpected model tag: {name}"),
        }
        if !matches!(name,"td"|"th"){ensure!(attrs.trim().is_empty(),"Unexpected attributes on container");}
        stack.push(name);
    }
    ensure!(rows>0&&!cells.is_empty(),"No actual table cells in token stream");
    ensure!(stack.iter().all(|v|matches!(*v,"html"|"body"|"table"|"thead"|"tbody"|"tfoot")),
        "Truncated row or cell; do not invent a missing closure");
    ensure!(slots.iter().all(|(r,_)|*r<=rows),"Span extends beyond actual model rows");
    let holes=rows.checked_mul(cols).context("Table dimensions overflow")?.saturating_sub(slots.len());
    let omitted:Vec<_>=stack.iter().rev().copied().collect();
    Ok(json!({"status":"structure_candidate","rows":rows,"columns":cols,"cells":cells,
        "unoccupied_slot_count":holes,"omitted_terminal_container_closures":omitted,
        "complete_token_stream":stack.is_empty(),"box_identity_verified":false,
        "acceptance_ready":false,"boundary":BOUNDARY}))
}
fn quad_box(v:&Value)->Option<[f64;4]>{
    let pts=v.as_array().filter(|a|a.len()==4)?;
    let mut b=[f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY];
    for pt in pts{
        let p=pt.as_array().filter(|p|p.len()==2)?;
        let (x,y)=(p[0].as_f64()?,p[1].as_f64()?);
        if !x.is_finite()||!y.is_finite(){return None;}
        b[0]=b[0].min(x);b[1]=b[1].min(y);b[2]=b[2].max(x);b[3]=b[3].max(y);
    }
    (b[0]<b[2]&&b[1]<b[3]).then_some(b)
}
fn contains(a:[f64;4],b:[f64;4])->bool{
    a[0]<=b[0]&&a[1]<=b[1]&&a[2]>=b[2]&&a[3]>=b[3]
}
fn overlaps(a:[f64;4],b:[f64;4])->bool{
    a[0]<b[2]&&a[2]>b[0]&&a[1]<b[3]&&a[3]>b[1]
}
/// Geometric association only. No text is discarded or distributed by an
/// assumed column count. A line crossing cells stays unresolved.
pub fn associate_readings(cells:&[Value],blocks:&[Value])->Result<Value>{
    ensure!(cells.len()<=MAX_CELLS&&blocks.len()<=20_000,"Table association budget exceeded");
    let boxes:Vec<_>=cells.iter().map(|v|quad_box(&v["original_pixel_quad"])).collect();
    let mut assigned=vec![Vec::<usize>::new();cells.len()];
    let mut unresolved=Vec::new();
    for (i,block) in blocks.iter().enumerate(){
        let Some(b)=quad_box(&block["original_pixel_quad"]) else{
            unresolved.push(json!({"block_index":i,"reason":"invalid_location"}));continue;
        };
        let mut candidate=None;
        let mut count=0usize;
        let mut intersecting=Vec::new();
        for (j,cell) in boxes.iter().enumerate(){
            let Some(cell)=*cell else{continue;};
            if overlaps(cell,b){
                intersecting.push(j);
                if contains(cell,b){candidate=Some(j);count+=1;}
            }
        }
        if count==1&&intersecting.len()==1{
            assigned[candidate.unwrap()].push(i);
        }else{
            unresolved.push(json!({"block_index":i,"reason":if intersecting.is_empty(){"unassigned"}else{"ambiguous_or_cross_cell"},
                "intersecting_cells":intersecting}));
        }
    }
    let out:Vec<_>=assigned.into_iter().enumerate().map(|(i,ids)|json!({"cell_index":i,
        "block_indices":ids,"status":if ids.is_empty(){"no_reading_assigned"}else{"geometric_candidate"}})).collect();
    Ok(json!({"cell_readings":out,"unresolved_blocks":unresolved,
        "no_reading_discarded":true,"acceptance_ready":false,"boundary":BOUNDARY}))
}
pub(crate) fn cell_quad(b:[f64;4],w:u32,h:u32)->Value{
    let ok=b.iter().all(|x|x.is_finite())&&b[0]>=0.&&b[1]>=0.&&b[0]<b[2]&&b[1]<b[3]&&
        b[2]<=f64::from(w)&&b[3]<=f64::from(h);
    if ok{json!([[b[0],b[1]],[b[2],b[1]],[b[2],b[3]],[b[0],b[3]]])}else{Value::Null}
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn merged_cells_and_missing_terminal_closure_are_explicit(){
        let raw="<tbody><tr><td rowspan=\"2\"></td><td colspan=\"2\"></td></tr><tr><td></td><td></td></tr>";
        let v=parse_structure(raw).unwrap();assert_eq!(v["rows"],2);assert_eq!(v["columns"],3);
        assert_eq!(v["unoccupied_slot_count"],0);assert_eq!(v["cells"][3]["column"],3);
        assert_eq!(v["omitted_terminal_container_closures"],json!(["tbody"]));
        assert_eq!(v["complete_token_stream"],false);
        assert_eq!(v["box_identity_verified"],false);
    }
    #[test]
    fn malformed_table_tokens_never_invent_cells(){
        for s in ["<tbody><tr><td>", "<tr><td rowspan=\"2\"></td></tr>", "<tr><td onclick=\"x\"></td></tr>",
            "<tr><td colspan=\"0\"></td></tr>","<tr><td colspan=\"999999\"></td></tr>","<tr><td></tr></td>","<tr><td>text</td></tr>"]{
            assert!(parse_structure(s).is_err(),"{s}");
        }
    }
    #[test]
    fn crossing_and_outside_readings_are_retained(){
        let cells=vec![json!({"original_pixel_quad":cell_quad([0.,0.,50.,50.],100,100)}),
            json!({"original_pixel_quad":cell_quad([50.,0.,100.,50.],100,100)})];
        let blocks=vec![json!({"original_pixel_quad":cell_quad([10.,10.,40.,20.],100,100)}),
            json!({"original_pixel_quad":cell_quad([40.,20.,60.,30.],100,100)}),
            json!({"original_pixel_quad":cell_quad([5.,60.,20.,80.],100,100)}),json!({"original_pixel_quad":null})];
        let v=associate_readings(&cells,&blocks).unwrap();
        assert_eq!(v["cell_readings"][0]["block_indices"],json!([0]));
        assert_eq!(v["cell_readings"][1]["status"],"no_reading_assigned");
        assert_eq!(v["unresolved_blocks"].as_array().unwrap().len(),3);
    }
}
