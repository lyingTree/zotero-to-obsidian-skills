//! Pure-Rust PDF geometric layout analyzer and selective page renderer.
//! Groups native PdfCraft glyph lines into reading-order paragraphs, headings,
//! display formulas, captions, and bibliography entries, emitting targeted
//! visual-review bounding boxes instead of blindly rasterizing every prose page.
use anyhow::{anyhow,ensure,Context,Result};
use regex::Regex;
use serde_json::{json,Value};
use std::{
    collections::{BTreeMap,BTreeSet},
    fs,
    io::Write,
    path::Path,
    sync::OnceLock,
};
use crate::{check_ledger,check_write,files,glossary,ocr_pdf,path_text,pdf_inventory,pdfcraft_backend::Backend,pipeline,tex_source};

const BOUNDARY:&str="Heuristic pure-Rust layout clustering from native PDF glyph geometry. Verify complex tables, multi-line equations, garbled math glyphs, and figure boundaries against rendered PDF images.";
type Box4=[f64;4];

#[derive(Clone,Debug)]
struct LineItem{
    line_index:u64,
    text:String,
    bbox:Box4,
    height:f64,
    garbled:bool,
    col:i8, // -1 left, 0 span/single, 1 right
}

#[derive(Clone,Debug,PartialEq,Eq)]
enum Role{
    Heading,
    Caption,
    DisplayMath,
    ReferenceStart,
    Prose,
}

fn compile_rx(s:&str)->Regex{Regex::new(s).expect("pdf_layout regex")}
macro_rules! rx{($s:expr)=>{{static R:OnceLock<Regex>=OnceLock::new();R.get_or_init(||compile_rx($s))}}}

fn box4(v:&Value)->Option<Box4>{
    let a=v.as_array().filter(|a|a.len()==4)?;
    let mut b=[0.;4];
    for (i,x) in a.iter().enumerate(){b[i]=x.as_f64().filter(|n|n.is_finite())?;}
    (b[0]<b[2]&&b[1]<b[3]).then_some(b)
}
fn intersect(a:Box4,b:Box4)->Option<Box4>{
    let c=[a[0].max(b[0]),a[1].max(b[1]),a[2].min(b[2]),a[3].min(b[3])];
    (c[0]<c[2]&&c[1]<c[3]).then_some(c)
}
fn union(a:Box4,b:Box4)->Box4{[a[0].min(b[0]),a[1].min(b[1]),a[2].max(b[2]),a[3].max(b[3])]}

fn is_garbled_str(s:&str)->bool{
    crate::mojibake::analyze_text(s).is_garbled
}

fn is_page1_affiliation_block(page:u64,kind:&str,text:&str,bbox:Box4,bounds:Box4)->bool{
    if page!=1 || kind!="paragraph"{return false;}
    let trimmed=text.trim();
    if trimmed.is_empty() || trimmed.len()>260{return false;}
    let page_h=(bounds[3]-bounds[1]).max(1.0);
    if bbox[1]>bounds[1]+0.45*page_h{return false;}
    trimmed.contains('@')
        || rx!(r"(?i)\b(?:University|Institute|Department|Laboratory|Academy|College|School|Center|Centre)\b|(?:大学|学院|研究所|实验室)").is_match(trimmed)
}

fn extract_lines(native:&Value,bounds:Box4)->Vec<LineItem>{
    let Some(glyphs)=native["glyphs"].as_array() else{return vec![];};
    let mut grouped=BTreeMap::<u64,Vec<&Value>>::new();
    for g in glyphs{
        if let Some(idx)=g["line_index"].as_u64(){grouped.entry(idx).or_default().push(g);}
    }
    let mut out=Vec::with_capacity(grouped.len());
    for (line_index,rows) in grouped{
        let mut text=String::new();
        let mut b:Option<Box4>=None;
        for g in rows{
            let reading=g["text"].as_str().unwrap_or("");
            if g["space_before"]==true&&!text.is_empty(){text.push(' ');}
            text.push_str(reading);
            if let Some(c)=box4(&g["bbox"]).and_then(|gb|intersect(bounds,gb)){
                b=Some(b.map_or(c,|a|union(a,c)));
            }
        }
        let trimmed=text.trim();
        let Some(bbox)=b else{continue;};
        if trimmed.is_empty(){continue;}
        let height=(bbox[3]-bbox[1]).max(1.0);
        let garbled=is_garbled_str(trimmed);
        out.push(LineItem{line_index,text:trimmed.to_owned(),bbox,height,garbled,col:0});
    }
    out.sort_by(|a,b|a.bbox[1].total_cmp(&b.bbox[1]).then(a.bbox[0].total_cmp(&b.bbox[0])));
    out
}

fn median(vals:&mut [f64],fallback:f64)->f64{
    if vals.is_empty(){return fallback;}
    vals.sort_by(|a,b|a.total_cmp(b));
    vals[vals.len()/2]
}

fn order_columns(lines:&mut Vec<LineItem>,bounds:Box4)->bool{
    if lines.len()<6{return false;}
    let min_x=lines.iter().map(|l|l.bbox[0]).fold(bounds[2],f64::min);
    let max_x=lines.iter().map(|l|l.bbox[2]).fold(bounds[0],f64::max);
    let cw=(max_x-min_x).max(1.0);
    if cw<120.0{return false;}
    let mid_x=min_x+0.5*cw;
    for l in lines.iter_mut(){
        let w=l.bbox[2]-l.bbox[0];
        if l.bbox[2]<=mid_x+0.03*cw && w<0.58*cw{l.col=-1;}
        else if l.bbox[0]>=mid_x-0.03*cw && w<0.58*cw{l.col=1;}
        else{l.col=0;}
    }
    // Check substantive overlap so right-aligned equation numbers like "(1)" on
    // a single-column page never trigger two-column splitting.
    let mut overlap_pairs=0usize;
    for r in lines.iter().filter(|l|l.col==1&&l.text.len()>=18){
        if lines.iter().any(|l|l.col==-1&&l.text.len()>=18&&l.bbox[3].min(r.bbox[3])>l.bbox[1].max(r.bbox[1])-2.0){
            overlap_pairs+=1;
        }
    }
    if overlap_pairs<4{
        for l in lines.iter_mut(){l.col=0;}
        return false;
    }
    let mut ordered=Vec::with_capacity(lines.len());
    let mut left_buf=Vec::new();
    let mut right_buf=Vec::new();
    let flush=|l:&mut Vec<LineItem>,r:&mut Vec<LineItem>,out:&mut Vec<LineItem>|{
        l.sort_by(|a,b|a.bbox[1].total_cmp(&b.bbox[1]).then(a.bbox[0].total_cmp(&b.bbox[0])));
        r.sort_by(|a,b|a.bbox[1].total_cmp(&b.bbox[1]).then(a.bbox[0].total_cmp(&b.bbox[0])));
        out.append(l);
        out.append(r);
    };
    for l in std::mem::take(lines){
        match l.col{
            -1=>left_buf.push(l),
            1=>right_buf.push(l),
            _=>{
                flush(&mut left_buf,&mut right_buf,&mut ordered);
                ordered.push(l);
            }
        }
    }
    flush(&mut left_buf,&mut right_buf,&mut ordered);
    *lines=ordered;
    true
}

fn is_margin_line(line:&LineItem,idx:usize,total:usize,lines:&[LineItem],page:u64,bounds:Box4,med_h:f64)->bool{
    let page_h=(bounds[3]-bounds[1]).max(1.0);
    let in_top=line.bbox[3]<=bounds[1]+0.07*page_h;
    let in_bottom=line.bbox[1]>=bounds[3]-0.07*page_h;
    if !(in_top||in_bottom){return false;}
    let t=line.text.trim();
    if rx!(r"^(?:[-–—]\s*)?\d{1,4}(?:\s*[-–—])?$").is_match(t){return true;}
    if page==1 && in_top{return false;}
    if t.len()<=75 && line.height<=1.1*med_h{
        if in_top && idx+1<total && lines[idx+1].bbox[1]-line.bbox[3]>=med_h*1.4{return true;}
        if in_bottom && idx>0 && line.bbox[1]-lines[idx-1].bbox[3]>=med_h*1.4{return true;}
    }
    false
}

fn classify_line(line:&LineItem,min_x:f64,max_x:f64,med_h:f64,in_references:bool)->Role{
    let t=line.text.trim();
    if rx!(r"^(?:Figure|Fig\.|Table|Tbl\.|图|表)\s*S?\d+(?:\.\d+)?\s*[:.：]").is_match(t){
        return Role::Caption;
    }
    if in_references && rx!(r"^(?:\[\d+\]|\d{1,3}\.)\s+[A-Z\u{4e00}-\u{9fff}]").is_match(t){
        return Role::ReferenceStart;
    }
    if t.len()<=100 && (rx!(r"^(?:\d+(?:\.\d+)*\.?\s+[A-Z\u{4e00}-\u{9fff}].*|(?:Abstract|Introduction|Background|Preliminaries|Conclusion|Conclusions|Acknowledgments?|References|Bibliography|Appendix(?:\s+[A-Z0-9]+)?|Code\s+Availability|Data\s+Availability)\.?)$").is_match(t)
        || (line.height>=1.14*med_h && !t.ends_with('.') && !t.ends_with(','))){
        return Role::Heading;
    }
    let cw=(max_x-min_x).max(1.0);
    let centered=line.col==0 && line.bbox[0]>min_x+0.11*cw && line.bbox[2]<max_x-0.08*cw && t.len()<130;
    let math_ops=t.chars().filter(|c|matches!(*c,'='|'∑'|'∫'|'∂'|'→'|'↦'|'≤'|'≥'|'∈'|'⊆'|'≃'|'≅'|'⟨'|'⟩'|'⊗'|'⊕')
        || ('\u{0391}'..='\u{03C9}').contains(c) || ('\u{1D400}'..='\u{1D7FF}').contains(c)).count();
    let has_eq_tag=rx!(r"\(\d+(?:\.\d+)?\)$").is_match(t);
    if (centered && (math_ops>=1 || has_eq_tag)) || (has_eq_tag && math_ops>=2 && t.len()<140){
        return Role::DisplayMath;
    }
    Role::Prose
}

fn join_lines(prev:&mut String,next:&str,keep_newlines:bool){
    if keep_newlines{
        prev.push('\n');
        prev.push_str(next);
        return;
    }
    if prev.ends_with('-') && rx!(r"[A-Za-z]{2}-$").is_match(prev) && next.chars().next().is_some_and(|c|c.is_ascii_lowercase()){
        prev.pop();
        prev.push_str(next);
    }else{
        prev.push(' ');
        prev.push_str(next);
    }
}

#[cfg(test)]
pub(crate) fn analyze_page(source_id:&str,page:u64,bounds:Box4,native:&Value,structure:&Value,in_references:&mut bool)->Value{
    analyze_page_with_quality(source_id,page,bounds,native,structure,in_references,None)
}

pub(crate) fn analyze_page_with_quality(
    source_id:&str,
    page:u64,
    bounds:Box4,
    native:&Value,
    structure:&Value,
    in_references:&mut bool,
    page_quality:Option<&crate::pdf_inspector_bridge::PageQuality>,
)->Value{
    let raw_lines=extract_lines(native,bounds);
    let mut heights:Vec<f64>=raw_lines.iter().filter(|l|l.text.len()>=20).map(|l|l.height).collect();
    if heights.is_empty(){heights=raw_lines.iter().map(|l|l.height).collect();}
    let med_h=median(&mut heights,10.0);

    let mut body=Vec::with_capacity(raw_lines.len());
    let mut margin_lines=Vec::new();
    for (i,l) in raw_lines.iter().enumerate(){
        if is_margin_line(l,i,raw_lines.len(),&raw_lines,page,bounds,med_h){
            margin_lines.push(json!({"line_index":l.line_index,"text":l.text,"bbox":l.bbox}));
        }else{
            body.push(l.clone());
        }
    }
    let two_column=order_columns(&mut body,bounds);
    let min_x=body.iter().map(|l|l.bbox[0]).fold(bounds[2],f64::min);
    let max_x=body.iter().map(|l|l.bbox[2]).fold(bounds[0],f64::max);

    let mut gaps=Vec::new();
    for w in body.windows(2){
        if w[0].col==w[1].col{
            let g=w[1].bbox[1]-w[0].bbox[3];
            if (0.0..=2.5*med_h).contains(&g){gaps.push(g);}
        }
    }
    let med_gap=median(&mut gaps,0.35*med_h).max(1.5);

    struct Draft{
        kind:&'static str,
        col:i8,
        bbox:Box4,
        text:String,
        line_indices:Vec<u64>,
        garbled:bool,
        last_line_bottom:f64,
        block_left:f64,
        last_line_ended_sentence:bool,
    }
    let mut drafts:Vec<Draft>=Vec::new();
    let mut visual_regions:Vec<Value>=Vec::new();

    for line in &body{
        let role=classify_line(line,min_x,max_x,med_h,*in_references);
        if role==Role::Heading && rx!(r"(?i)^(?:\d+(?:\.\d+)*\.?\s+)?(?:References|Bibliography)\.?$").is_match(line.text.trim()){
            *in_references=true;
        }
        let target_kind=match role{
            Role::Heading=>"heading",
            Role::Caption=>"caption",
            Role::DisplayMath=>"display_math",
            Role::ReferenceStart=>"reference",
            Role::Prose=>if *in_references && drafts.last().is_some_and(|d|d.kind=="reference"){"reference"}else{"paragraph"},
        };
        let split_gap=line.bbox[1]-drafts.last().map_or(line.bbox[1],|d|d.last_line_bottom)>med_gap*1.65+2.0;
        let same_block=if let Some(prev)=drafts.last(){
            prev.col==line.col
                && prev.kind==target_kind
                && prev.garbled==line.garbled
                && role!=Role::Heading
                && role!=Role::ReferenceStart
                && !split_gap
                && !(target_kind=="paragraph"
                    && role==Role::Prose
                    && prev.last_line_ended_sentence
                    && line.bbox[0]-prev.block_left>0.75*med_h)
        }else{false};

        let ended_sentence=line.text.trim_end().ends_with(['.',':','?','!','。','：','？','！']);
        if same_block{
            let prev=drafts.last_mut().unwrap();
            join_lines(&mut prev.text,&line.text,target_kind=="display_math");
            prev.bbox=union(prev.bbox,line.bbox);
            prev.line_indices.push(line.line_index);
            prev.garbled|=line.garbled;
            prev.last_line_bottom=line.bbox[3];
            prev.last_line_ended_sentence=ended_sentence;
        }else{
            drafts.push(Draft{
                kind:target_kind,
                col:line.col,
                bbox:line.bbox,
                text:line.text.clone(),
                line_indices:vec![line.line_index],
                garbled:line.garbled,
                last_line_bottom:line.bbox[3],
                block_left:line.bbox[0],
                last_line_ended_sentence:ended_sentence,
            });
        }
    }

fn extract_fig_id(caption:&str)->Option<String>{
    let cap=rx!(r"(?i)^(?:Figure|Fig\.?|图)\s*(S?\d+(?:[.-]\d+)?[a-zA-Z]?)").captures(caption.trim())?;
    let raw=cap.get(1)?.as_str().replace('.',"-");
    if raw.is_empty(){return None;}
    let mut out=String::from("fig");
    if raw.starts_with('s')||raw.starts_with('S'){
        out.push('S');
        out.push_str(&raw[1..].to_ascii_lowercase());
    }else{
        out.push_str(&raw.to_ascii_lowercase());
    }
    Some(out)
}

    let page_w=(max_x-min_x).max(1.0);
    let mid_x=min_x+0.5*page_w;
    let min_prose_w=if two_column{0.35*page_w}else{0.50*page_w};
    let is_subfigure_label=|p:&Draft|->bool{
        if p.kind!="paragraph"{return false;}
        let t=p.text.trim();
        rx!(r"^(?:\([a-zA-Z0-9]\)|[a-zA-Z]\))\s+.*(?:\([b-zB-Z0-9]\)|\b[b-zB-Z]\))").is_match(t)
            ||(p.line_indices.len()==1
                &&!p.last_line_ended_sentence
                &&rx!(r"^(?:\([a-zA-Z0-9]\)|[a-zA-Z]\))\s+").is_match(t))
    };
    let is_prose_barrier=|p:&Draft|->bool{
        if is_subfigure_label(p){return false;}
        p.kind=="heading"
            ||p.kind=="caption"
            ||(p.text.len()>=55&&(p.bbox[2]-p.bbox[0])>=min_prose_w)
    };
    let mut blocks=Vec::with_capacity(drafts.len());
    let mut figure_candidates=Vec::new();
    let mut has_figure_caption=false;
    let mut has_table_caption=false;
    let mut prose_lines_set=BTreeSet::<u64>::new();
    for d in &drafts{
        if d.kind!="caption"&&is_prose_barrier(d){
            prose_lines_set.extend(d.line_indices.iter().copied());
        }
    }

    for (i,d) in drafts.iter().enumerate(){
        let id=format!("{source_id}-p{page}-b{}",i+1);
        let mut visual_required=false;
        let mut visual_reason:Option<&str>=None;
        if d.kind=="caption"{
            if rx!(r"^(?:Table|Tbl\.|表)").is_match(d.text.trim()){
                has_table_caption=true;
                let region=[bounds[0],(d.bbox[1]-18.0).max(bounds[1]),bounds[2],(d.bbox[3]+220.0).min(bounds[3])];
                visual_regions.push(json!({"page":page,"block_id":id,"kind":"table_region","bbox":region,"reason":"table caption detected","visual_required":false}));
            }else{
                has_figure_caption=true;
                let region=[bounds[0],(d.bbox[1]-220.0).max(bounds[1]),bounds[2],(d.bbox[3]+20.0).min(bounds[3])];
                visual_regions.push(json!({"page":page,"block_id":id,"kind":"figure_region","bbox":region,"reason":"figure caption detected","visual_required":false}));

                let (col_x0,col_x1)=if two_column&&d.col==-1{
                    (bounds[0],(mid_x+0.04*page_w).min(bounds[2]))
                }else if two_column&&d.col==1{
                    ((mid_x-0.04*page_w).max(bounds[0]),bounds[2])
                }else{(bounds[0],bounds[2])};

                let prev_bottom=drafts[..i].iter()
                    .filter(|p|p.bbox[3]<=d.bbox[1]+2.0
                        &&(p.col==d.col||p.col==0||d.col==0)
                        &&!(p.kind=="paragraph"&&p.line_indices.len()==1&&!p.last_line_ended_sentence&&d.bbox[1]-p.bbox[3]<=24.0)
                        &&is_prose_barrier(p))
                    .map(|p|p.bbox[3])
                    .reduce(f64::max);
                let cap_top=(d.bbox[1]-1.0).max(bounds[1]+8.0);
                let search_top=prev_bottom
                    .map(|b|b+4.0)
                    .unwrap_or_else(||bounds[1]+0.04*(bounds[3]-bounds[1]))
                    .min(cap_top-8.0)
                    .max(bounds[1]);

                let mut matched_xrefs=Vec::new();
                let mut img_union:Option<Box4>=None;
                let mut clean_effects=true;
                for occ in structure["occurrences"].as_array().into_iter().flatten(){
                    let Some(ob)=box4(&occ["bbox"]).and_then(|r|intersect(bounds,r)) else{continue;};
                    let cx=0.5*(ob[0]+ob[2]);
                    let cy=0.5*(ob[1]+ob[3]);
                    if cx>=col_x0-15.0&&cx<=col_x1+15.0&&cy>=search_top-8.0&&cy<=d.bbox[1]+8.0{
                        if let Some(xr)=occ["image_xref"].as_u64(){matched_xrefs.push(xr);}
                        img_union=Some(img_union.map_or(ob,|u|union(u,ob)));
                        if occ["context_effects"].as_array().is_some_and(|a|!a.is_empty()){
                            clean_effects=false;
                        }
                    }
                }

                let mut fig_union=img_union;
                let mut has_surrounding_labels=false;
                for l in &body{
                    if d.line_indices.contains(&l.line_index)||prose_lines_set.contains(&l.line_index){continue;}
                    if l.text.len()>55{continue;}
                    let cx=0.5*(l.bbox[0]+l.bbox[2]);
                    if cx<col_x0-10.0||cx>col_x1+10.0{continue;}
                    if l.bbox[1]<search_top-2.0||l.bbox[3]>d.bbox[1]-1.0{continue;}
                    if let Some(iu)=img_union{
                        let near_x=l.bbox[2]>=iu[0]-52.0&&l.bbox[0]<=iu[2]+52.0;
                        let near_y=l.bbox[3]>=iu[1]-36.0&&l.bbox[1]<=iu[3]+36.0;
                        if !(near_x&&near_y){continue;}
                        let inside_img=l.bbox[0]>=iu[0]-1.0&&l.bbox[1]>=iu[1]-1.0&&l.bbox[2]<=iu[2]+1.0&&l.bbox[3]<=iu[3]+1.0;
                        if !inside_img{has_surrounding_labels=true;}
                    }
                    fig_union=Some(fig_union.map_or(l.bbox,|u|union(u,l.bbox)));
                }

                let suggested_bbox=if !matched_xrefs.is_empty(){
                    if let Some(u)=fig_union{
                        let top=(u[1]-8.0).max(bounds[1]).min(cap_top-4.0);
                        let bottom=(u[3]+6.0).min(cap_top).max(top+4.0);
                        [(u[0]-8.0).max(bounds[0]),top,(u[2]+8.0).min(bounds[2]).max((u[0]-8.0).max(bounds[0])+4.0),bottom]
                    }else{
                        let top=(cap_top-260.0).max(search_top).min(cap_top-4.0);
                        [col_x0,top,col_x1.max(col_x0+4.0),cap_top]
                    }
                }else{
                    let band_top=if prev_bottom.is_some(){
                        search_top.max(cap_top-420.0)
                    }else{
                        (cap_top-260.0).max(search_top)
                    };
                    let label_top=fig_union.map_or(band_top,|u|(u[1]-12.0).max(search_top));
                    let top=band_top.min(label_top).max(bounds[1]).min(cap_top-4.0);
                    [col_x0,top,col_x1.max(col_x0+4.0),cap_top]
                };
                let single_object_reusable=matched_xrefs.len()==1&&!has_surrounding_labels&&clean_effects;
                let confidence=if !matched_xrefs.is_empty(){"high"}else{"medium"};
                figure_candidates.push(json!({
                    "page":page,
                    "block_id":id,
                    "suggested_fig_id":extract_fig_id(&d.text),
                    "caption_text":d.text,
                    "suggested_bbox":suggested_bbox,
                    "matched_image_xrefs":matched_xrefs,
                    "single_object_reusable":single_object_reusable,
                    "confidence":confidence,
                }));
            }
        }
        if d.garbled{
            visual_required=true;
            visual_reason=Some("garbled_glyphs");
            visual_regions.push(json!({"page":page,"block_id":id,"kind":"garbled_glyphs","bbox":d.bbox,"reason":"mojibake, replacement or private-use unicode glyph detected","visual_required":true}));
        }else if d.kind=="display_math"{
            let kind=if d.line_indices.len()>=2{"multiline_formula"}else{"display_formula"};
            let reason=if d.line_indices.len()>=2{"multi-line display math block requires visual verification"}else{"2D display formula requires visual verification"};
            visual_required=true;
            visual_reason=Some(kind);
            visual_regions.push(json!({"page":page,"block_id":id,"kind":kind,"bbox":d.bbox,"reason":reason,"visual_required":true}));
        }else if is_page1_affiliation_block(page,d.kind,&d.text,d.bbox,bounds){
            visual_required=true;
            visual_reason=Some("page1_affiliation");
            visual_regions.push(json!({"page":page,"block_id":id,"kind":"page1_affiliation","bbox":d.bbox,"reason":"page 1 author/affiliation header block requires visual check","visual_required":true}));
        }
        blocks.push(json!({
            "id":id,
            "page":page,
            "kind":d.kind,
            "column":d.col,
            "bbox":d.bbox,
            "text":d.text.clone(),
            "line_indices":d.line_indices.clone(),
            "garbled":d.garbled,
            "visual_required":visual_required,
            "visual_reason":visual_reason,
        }));
    }

    let image_count=structure["occurrences"].as_array().map_or(0,Vec::len);
    let classification=ocr_pdf::classify(native,structure);
    let mut ocr_whole_page=classification["ocr_whole_page"]==true;
    let mut page_visual_required=false;
    let mut page_visual_reason:Option<String>=None;
    let mut has_detected_table=false;
    if let Some(pq)=page_quality{
        if pq.has_table{has_detected_table=true;}
        if pq.has_type3_or_unmapped_font{
            ocr_whole_page=true;
            page_visual_required=true;
            let reason=format!("unmapped_font_page({})",pq.unmapped_fonts.join(","));
            page_visual_reason=Some(reason.clone());
            visual_regions.push(json!({"page":page,"block_id":format!("{source_id}-p{page}-page"),"kind":"unmapped_font_page","bbox":bounds,"reason":reason,"visual_required":true}));
        }else if !pq.mojibake_reasons.is_empty(){
            ocr_whole_page=true;
            page_visual_required=true;
            let reason=format!("mojibake_page({})",pq.mojibake_reasons.join(","));
            page_visual_reason=Some(reason.clone());
            visual_regions.push(json!({"page":page,"block_id":format!("{source_id}-p{page}-page"),"kind":"mojibake_page","bbox":bounds,"reason":reason,"visual_required":true}));
        }else if pq.needs_ocr{
            let substantive=image_count>0||raw_lines.is_empty()||pq.reasons.iter().any(|r|{
                let rl=r.to_ascii_lowercase();
                rl.contains("encoding")||rl.contains("cmap")||rl.contains("garbl")||rl.contains("mojibake")
                    ||rl.contains("font")||rl.contains("vector")||rl.contains("invisible")
            });
            if substantive{
                ocr_whole_page=true;
                page_visual_required=true;
                let reason=if pq.reasons.is_empty(){"scanned_or_ocr_page".to_owned()}else{format!("scanned_or_ocr_page({})",pq.reasons.join(","))};
                page_visual_reason=Some(reason.clone());
                visual_regions.push(json!({"page":page,"block_id":format!("{source_id}-p{page}-page"),"kind":"scanned_or_ocr_page","bbox":bounds,"reason":reason,"visual_required":true}));
            }
        }
    }
    if ocr_whole_page && !page_visual_required{
        page_visual_required=true;
        let reason=classification["kind"].as_str().unwrap_or("scan_or_unreliable_text_layer").to_owned();
        page_visual_reason=Some(reason.clone());
        visual_regions.push(json!({"page":page,"block_id":format!("{source_id}-p{page}-page"),"kind":"scanned_or_ocr_page","bbox":bounds,"reason":reason,"visual_required":true}));
    }
    json!({
        "page":page,
        "bounds":bounds,
        "two_column":two_column,
        "raw_line_count":raw_lines.len(),
        "margin_lines":margin_lines,
        "blocks":blocks,
        "visual_review_regions":visual_regions,
        "figure_candidates":figure_candidates,
        "has_images":image_count>0,
        "has_figure_caption":has_figure_caption,
        "has_table_caption":has_table_caption,
        "has_detected_table":has_detected_table,
        "ocr_whole_page":ocr_whole_page,
        "page_visual_required":page_visual_required,
        "page_visual_reason":page_visual_reason,
        "classification":classification,
    })
}

fn write_atomic_new(p:&Path,bytes:&[u8])->Result<Value>{
    ensure!(!p.exists()&&!p.is_symlink(),"Keep existing layout artifact: {}",p.display());
    let mut t=tempfile::NamedTempFile::new_in(p.parent().context("layout parent")?)?;
    t.write_all(bytes)?;t.as_file().sync_all()?;
    t.persist_noclobber(p).map_err(|e|anyhow!("Cannot publish {}: {}",p.display(),e.error))?;
    let b=check_ledger::bind_file(p)?;
    ensure!(b["sha256"]==files::digest(bytes),"Layout artifact changed after write");
    Ok(b)
}

fn clamp_box(b:Box4,bounds:Box4)->Option<Box4>{
    let c=[
        b[0].max(bounds[0]).min(bounds[2]),
        b[1].max(bounds[1]).min(bounds[3]),
        b[2].max(bounds[0]).min(bounds[2]),
        b[3].max(bounds[1]).min(bounds[3]),
    ];
    (c[0]<c[2]&&c[1]<c[3]).then_some(c)
}

fn expand_box(b:Box4,margin:f64,bounds:Box4)->Option<Box4>{
    clamp_box([b[0]-margin,b[1]-margin,b[2]+margin,b[3]+margin],bounds)
}

fn format_bbox(v:&Value)->String{
    if let Some(b)=box4(v){
        format!("[{:.1},{:.1},{:.1},{:.1}]",b[0],b[1],b[2],b[3])
    }else{
        "[0.0,0.0,0.0,0.0]".into()
    }
}

fn sanitize_stem(id:&str)->String{
    id.chars().map(|c|if c.is_ascii_alphanumeric()||matches!(c,'_'|'-'){c}else{'_'}).collect()
}

fn render_clean_layout_markdown(source_id:&str,pages:&[Value])->String{
    let mut out=String::from("<!-- Derived from pure-Rust PDF geometric layout clustering with pdf-inspector preflight & zero-Mojibake firewall. Resolve every VISUAL_REQUIRED marker against ocr-verify-sheet.png / rendered pages before publishing. -->\n\n");
    for p in pages{
        let page_num=p["page"].as_u64().unwrap_or(0);
        out.push_str(&format!("<!-- page {page_num} -->\n\n"));
        if p["page_visual_required"]==true{
            let reason=p["page_visual_reason"].as_str().unwrap_or("page_visual_review_required");
            let bbox_str=format_bbox(&p["bounds"]);
            out.push_str(&format!("<!-- VISUAL_REQUIRED: {source_id}-p{page_num}-page | reason={reason} | page={page_num} | bbox={bbox_str} -->\n\n"));
        }
        for b in p["blocks"].as_array().into_iter().flatten(){
            let id=b["id"].as_str().unwrap_or("");
            let kind=b["kind"].as_str().unwrap_or("paragraph");
            let text=b["text"].as_str().unwrap_or("").trim();
            let garbled=b["garbled"]==true;
            let vis_req=b["visual_required"]==true;
            let vis_reason=b["visual_reason"].as_str().unwrap_or("visual_review_required");
            let bbox_str=format_bbox(&b["bbox"]);
            if garbled{
                // Zero-Mojibake Firewall: NEVER emit raw garbled/PUA/U+FFFD text into clean-layout.md.
                out.push_str(&format!("<!-- VISUAL_REQUIRED: {id} | reason={vis_reason} | page={page_num} | bbox={bbox_str} -->\n\n"));
                continue;
            }
            if text.is_empty(){continue;}
            if vis_req{
                out.push_str(&format!("<!-- VISUAL_REQUIRED: {id} | reason={vis_reason} | page={page_num} | bbox={bbox_str} -->\n"));
            }
            match kind{
                "heading"=>out.push_str(&format!("## {text} <!-- {id} -->\n\n")),
                "display_math"=>out.push_str(&format!("<!-- {id} (math) -->\n$$\n{text}\n$$\n\n")),
                "caption"=>out.push_str(&format!("<!-- {id} (caption) -->\n**{text}**\n\n")),
                "reference"=>out.push_str(&format!("- <!-- {id} --> {text}\n")),
                _=>out.push_str(&format!("<!-- {id} --> {text}\n\n")),
            }
        }
    }
    out
}

pub fn prepare_pdf_layout(source_pdf:&Path,destination:&Path,source_id:&str,render_pages:&str,dpi:u64,expected_sha:Option<&str>)->Result<Value>{
    ensure!(source_id.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)&&
        source_id.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-')),"Invalid source ID");
    ensure!(matches!(render_pages,"selective"|"all"|"none"),"render_pages must be selective, all or none");
    ensure!((72..=600).contains(&dpi),"dpi must be between 72 and 600");
    ensure!(!destination.is_symlink(),"Refusing a symlink layout directory");

    let layout_started=std::time::Instant::now();
    let mut backend=Backend::open(source_pdf,expected_sha)?;
    let source_binding=backend.source_binding().clone();
    if render_pages!="none"{backend.preflight_whole_pages(dpi)?;}
    let pdf_quality_report=crate::pdf_inspector_bridge::inspect_pdf_bytes(backend.source_bytes()).ok();

    fs::create_dir_all(destination)?;
    let root=files::canonical(destination)?;
    let blocks_file=root.join("layout-blocks.json");
    let regions_file=root.join("visual-review-regions.json");
    let clean_file=root.join("clean-layout.md");
    for p in [&blocks_file,&regions_file,&clean_file]{
        ensure!(!p.exists()&&!p.is_symlink(),"Keep existing layout evidence: {}",p.display());
    }
    let pages_dir=root.join("pages");
    let image_dir=root.join("image-objects");
    if render_pages!="none"{fs::create_dir_all(&pages_dir)?;}
    fs::create_dir_all(&image_dir)?;

    let total_pages=backend.len();
    let mut analyzed=Vec::with_capacity(total_pages);
    let mut raw_line_total=0usize;
    let mut block_total=0usize;
    let mut in_references=false;
    let mut rendered_pages=Vec::new();
    let mut figure_pages=Vec::new();
    let mut table_pages=Vec::new();
    let mut image_pages=Vec::new();
    let mut all_regions=Vec::new();
    let mut all_figure_candidates=Vec::new();
    let mut visual_required_items=Vec::new();
    let mut exported_objects=Vec::new();
    let mut seen_objects=BTreeSet::<(u32,u16)>::new();
    let mut bundled_ocr:Option<Result<crate::ocr_backend::Engine,String>>=None;

    let setup_ms=layout_started.elapsed().as_millis();
    let pages_started=std::time::Instant::now();
    let mut ocr_pages=BTreeSet::new();
    let mut ocr_attempts=0usize;
    let mut ocr_time=std::time::Duration::ZERO;
    let mut render_time=std::time::Duration::ZERO;
    for n in 1..=total_pages{
        let page=n as u64;
        let (_,_,bounds,_)=backend.geometry(page)?;
        let native=backend.text(page).unwrap_or_else(|e|json!({
            "schema_version":1,"status":"unavailable","error":format!("{e:#}"),
            "page":page,"plain_text":"","glyphs":[],"warnings":[],"truncated":true
        }));
        let structure=backend.structure(page)?;
        let pq=pdf_quality_report.as_ref().and_then(|r|r.pages.get(&page));
        let mut page_doc=analyze_page_with_quality(source_id,page,bounds,&native,&structure,&mut in_references,pq);

        let has_garbled_blocks=page_doc["blocks"].as_array().is_some_and(|arr|arr.iter().any(|b|b["garbled"]==true));
        if has_garbled_blocks && crate::ocr_backend::discover_bundled_package().is_some(){
            if bundled_ocr.is_none(){
                bundled_ocr=Some(match crate::ocr_backend::open_bundled_engine("off"){
                    Ok(Some(e))=>Ok(e),
                    Ok(None)=>Err("bundled OCR package absent".into()),
                    Err(e)=>Err(format!("{e:#}")),
                });
            }
            if let Some(Ok(engine))=bundled_ocr.as_mut(){
                let ocr_tmp=tempfile::Builder::new().prefix(".ocr-repair-").tempdir_in(&root)?;
                if let Some(blocks_arr)=page_doc["blocks"].as_array_mut(){
                    let mut repaired_ids=BTreeSet::<String>::new();
                    for (idx,blk) in blocks_arr.iter_mut().enumerate(){
                        if blk["garbled"]!=true{continue;}
                        let Some(bbox)=box4(&blk["bbox"]).and_then(|b|expand_box(b,6.0,bounds)) else{continue;};
                        let png=ocr_tmp.path().join(format!("p{page}-b{idx}.png"));
                        let Ok(raster)=backend.render(page,Some(bbox),300,&png) else{continue;};
                        let started=std::time::Instant::now();
                        ocr_pages.insert(page);
                        ocr_attempts+=1;
                        let recognition=engine.recognize_batch_image(&png,raster["asset"]["sha256"].as_str());
                        ocr_time+=started.elapsed();
                        match recognition{
                            Ok(res)=>{
                                let cand=res["plain_text"].as_str().unwrap_or("").trim().to_owned();
                                let ocr_blocks=res["blocks"].as_array();
                                let high_conf=ocr_blocks.is_some_and(|bs|!bs.is_empty()&&bs.iter().all(|ob|{
                                    ob["location_valid"]==true
                                        &&ob["text_score"].as_f64().is_some_and(|s|s>=0.92)
                                        &&ob["box_score"].as_f64().is_some_and(|s|s>=0.60)
                                }));
                                let clean=!cand.is_empty()&&!crate::mojibake::analyze_text(&cand).is_garbled;
                                if high_conf && clean{
                                    let prev=blk["text"].clone();
                                    blk["native_garbled_text"]=prev;
                                    blk["text"]=json!(cand);
                                    blk["garbled"]=json!(false);
                                    blk["ocr_repaired"]=json!(true);
                                    blk["visual_required"]=json!(false);
                                    blk["visual_reason"]=Value::Null;
                                    if let Some(id)=blk["id"].as_str(){repaired_ids.insert(id.to_owned());}
                                }else{
                                    blk["visual_required"]=json!(true);
                                    blk["visual_reason"]=json!(if !clean{"bundled_ocr_garbled_or_empty"}else{"bundled_ocr_low_confidence_below_0.92"});
                                    if !cand.is_empty(){blk["ocr_candidate"]=json!(cand);}
                                }
                            }
                            Err(_)=>{
                                blk["visual_required"]=json!(true);
                                blk["visual_reason"]=json!("bundled_ocr_failed");
                            }
                        }
                    }
                    if !repaired_ids.is_empty(){
                        if let Some(vr)=page_doc["visual_review_regions"].as_array_mut(){
                            vr.retain(|r|!(r["kind"]=="garbled_glyphs"&&r["block_id"].as_str().is_some_and(|id|repaired_ids.contains(id))));
                        }
                    }
                }
            }
        }

        if page_doc["page_visual_required"]==true{
            visual_required_items.push(json!({
                "id":format!("{source_id}-p{page}-page"),
                "page":page,
                "kind":"page_visual_required",
                "reason":page_doc["page_visual_reason"].as_str().unwrap_or("page_visual_review_required"),
                "bbox":bounds,
            }));
        }
        for blk in page_doc["blocks"].as_array().into_iter().flatten(){
            if blk["visual_required"]==true{
                if let Some(clamped)=box4(&blk["bbox"]).and_then(|b|clamp_box(b,bounds)){
                    visual_required_items.push(json!({
                        "id":blk["id"],
                        "page":page,
                        "kind":blk["kind"],
                        "reason":blk["visual_reason"].as_str().unwrap_or("visual_review_required"),
                        "bbox":clamped,
                    }));
                }
            }
        }

        raw_line_total+=page_doc["raw_line_count"].as_u64().unwrap_or(0) as usize;
        block_total+=page_doc["blocks"].as_array().map_or(0,Vec::len);
        let has_images=page_doc["has_images"]==true;
        let has_fig=page_doc["has_figure_caption"]==true;
        let has_tbl=page_doc["has_table_caption"]==true||page_doc["has_detected_table"]==true;
        let ocr_needed=page_doc["ocr_whole_page"]==true;
        let page_has_visual_required=page_doc["page_visual_required"]==true
            ||page_doc["blocks"].as_array().is_some_and(|arr|arr.iter().any(|b|b["visual_required"]==true));
        if has_images{image_pages.push(page);}
        if has_fig{figure_pages.push(page);}
        if has_tbl{table_pages.push(page);}
        for r in page_doc["visual_review_regions"].as_array().into_iter().flatten(){
            all_regions.push(r.clone());
        }
        for fc in page_doc["figure_candidates"].as_array().into_iter().flatten(){
            all_figure_candidates.push(fc.clone());
        }

        for indexed in structure["objects"].as_array().into_iter().flatten(){
            let Some(xref)=indexed["image_xref"].as_u64().and_then(|v|u32::try_from(v).ok()) else{continue;};
            let Some(generation)=indexed["generation"].as_u64().and_then(|v|u16::try_from(v).ok()) else{continue;};
            if !seen_objects.insert((xref,generation)){continue;}
            let stem=format!("xref-{xref:06}-g{generation}");
            match backend.export_indexed(page,xref,&image_dir.join(&stem),&structure){
                Ok(rec)=>{
                    exported_objects.push(json!({"page":page,"image_xref":xref,"generation":generation,"asset":rec["asset"],"method":rec["method"]}));
                }
                Err(e)=>{
                    exported_objects.push(json!({"page":page,"image_xref":xref,"generation":generation,"status":"page_render_required","reason":format!("{e:#}")}));
                }
            }
        }

        let should_render=match render_pages{
            "all"=>true,
            "selective"=>page==1 || has_images || has_fig || has_tbl || ocr_needed || page_has_visual_required,
            _=>false,
        };
        if should_render{
            let dest=pages_dir.join(format!("page-{n:04}.png"));
            let render_started=std::time::Instant::now();
            let r=backend.render(page,None,dpi,&dest)?;
            page_doc["render"]=r["asset"].clone();
            render_time+=render_started.elapsed();
            rendered_pages.push(json!({"page":page,"path":r["asset"]["path"],"sha256":r["asset"]["sha256"]}));
        }else{
            page_doc["render"]=Value::Null;
        }
        analyzed.push(page_doc);
    }
    backend.verify()?;

    if let Some(Ok(engine))=&bundled_ocr{engine.verify_batch()?;}
    let pages_ms=pages_started.elapsed().as_millis();
    let sheet_started=std::time::Instant::now();
    let verify_sheet=if !visual_required_items.is_empty() && render_pages!="none"{
        let verify_dir=root.join("verify-crops");
        fs::create_dir_all(&verify_dir)?;
        let sheet_path=root.join("ocr-verify-sheet.png");
        let mut batch_regions=Vec::new();
        for (idx,item) in visual_required_items.iter().take(32).enumerate(){
            let id=item["id"].as_str().unwrap_or("region");
            let reason=item["reason"].as_str().unwrap_or("visual_required");
            let dest=verify_dir.join(format!("verify-{:02}-{}.png",idx+1,sanitize_stem(id)));
            batch_regions.push(json!({
                "page_number":item["page"],
                "bbox":item["bbox"],
                "destination":path_text(&dest)?,
                "dpi":300,
                "margin_pt":6.0,
                "transparent":false,
                "auto_trim":false,
                "label":format!("{id} ({reason})"),
            }));
        }
        let source_sha=source_binding["sha256"].as_str();
        let rendered=crate::pdf_boundary::render_regions_with_backend(
            &mut backend,
            &Value::Array(batch_regions),
            crate::pdf_boundary::BatchRegionOptions{
                expected:source_sha,
                overwrite:false,
                contact_sheet_destination:Some(&sheet_path),
                contact_sheet_columns:None,
            },
        )?;
        json!({
            "contact_sheet":rendered["contact_sheet"],
            "cropped_count":rendered["count"],
            "total_visual_required":visual_required_items.len(),
            "truncated_at_32":visual_required_items.len()>32,
            "items":rendered["items"],
        })
    }else{Value::Null};

    let sheet_ms=sheet_started.elapsed().as_millis();
    backend.verify()?;
    let pdf_quality_json=pdf_quality_report.as_ref().map(|r|r.to_json()).unwrap_or(Value::Null);
    let clean_md=render_clean_layout_markdown(source_id,&analyzed);
    let clean_binding=write_atomic_new(&clean_file,clean_md.as_bytes())?;
    let layout_doc=json!({
        "schema_version":1,"backend":"pdfcraft-layout-v1","source_file":source_binding,"source_id":source_id,
        "page_count":total_pages,"render_mode":render_pages,"dpi":dpi,
        "pdf_quality":pdf_quality_json,
        "visual_required_count":visual_required_items.len(),
        "visual_required_items":visual_required_items,
        "ocr_verify_sheet":verify_sheet,
        "counts":{"ocr_pages":ocr_pages.len(),"ocr_calls":ocr_attempts,"raw_lines":raw_line_total,"clustered_blocks":block_total,"visual_review_regions":all_regions.len(),"visual_required":visual_required_items.len(),"rendered_pages":rendered_pages.len(),"exported_image_objects":exported_objects.len()},
        "image_objects":exported_objects,"pages":analyzed,"boundary":BOUNDARY
    });
    let mut layout_bytes=serde_json::to_vec_pretty(&layout_doc)?;layout_bytes.push(b'\n');
    let layout_binding=write_atomic_new(&blocks_file,&layout_bytes)?;

    let regions_doc=json!({
        "schema_version":1,"source_file":source_binding,"regions":all_regions,
        "figure_candidates":all_figure_candidates,
        "visual_required_items":visual_required_items,
        "ocr_verify_sheet":verify_sheet,
        "rendered_pages":rendered_pages,"figure_pages":figure_pages,"table_pages":table_pages
    });
    let mut region_bytes=serde_json::to_vec_pretty(&regions_doc)?;region_bytes.push(b'\n');
    let region_binding=write_atomic_new(&regions_file,&region_bytes)?;

    let _=pdf_inventory::BACKEND;
    let _=check_write::no_link(&clean_file);
    Ok(json!({
        "schema_version":1,
        "status":"ok",
        "backend":"pdfcraft-layout-v1",
        "source_file":source_binding,
        "page_count":total_pages,
        "render_mode":render_pages,
        "dpi":dpi,
        "pdf_quality":layout_doc["pdf_quality"],
        "visual_required_count":visual_required_items.len(),
        "visual_required_items":visual_required_items,
        "ocr_verify_sheet":verify_sheet,
        "counts":layout_doc["counts"],
        "timings_ms":{"setup":setup_ms,"pages":pages_ms,"page_png_render":render_time.as_millis(),
            "ocr_inference":ocr_time.as_millis(),"visual_review_sheet":sheet_ms},
        "figure_pages":figure_pages,
        "table_pages":table_pages,
        "image_pages":image_pages,
        "rendered_pages":rendered_pages,
        "figure_candidates":all_figure_candidates,
        "clean_markdown":clean_binding,
        "layout_blocks":layout_binding,
        "visual_review_regions":region_binding,
        "next_action":"Read clean-layout.md; resolve every VISUAL_REQUIRED marker using ocr-verify-sheet.png (when generated) and rendered Page 1 / figure / table pages.",
        "source_fidelity":"not_assessed",
        "boundary":BOUNDARY,
    }))
}

pub struct PreparePaperOptions<'a>{
    pub pdf_path:Option<&'a Path>,
    pub tex_target:Option<&'a Value>,
    pub work_dir:&'a Path,
    pub glossary_path:Option<&'a Path>,
    pub render_pages:&'a str,
    pub dpi:u64,
    pub expected_pdf_sha256:Option<&'a str>,
    pub arxiv_timeout_seconds:u64,
}

fn load_cached_layout(layout_dir:&Path,pdf_sha:&str)->Result<Option<Value>>{
    let blocks_file=layout_dir.join("layout-blocks.json");
    let regions_file=layout_dir.join("visual-review-regions.json");
    let clean_file=layout_dir.join("clean-layout.md");
    if !(blocks_file.is_file()&&regions_file.is_file()&&clean_file.is_file()){
        return Ok(None);
    }
    let blocks:Value=crate::parse_json(&fs::read_to_string(&blocks_file)?)?;
    if blocks["source_file"]["sha256"]!=pdf_sha{
        return Ok(None);
    }
    let regions:Value=crate::parse_json(&fs::read_to_string(&regions_file)?)?;
    Ok(Some(json!({
        "schema_version":1,
        "status":"cached",
        "backend":"pdfcraft-layout-v1",
        "source_file":blocks["source_file"],
        "page_count":blocks["page_count"],
        "render_mode":blocks["render_mode"],
        "dpi":blocks["dpi"],
        "pdf_quality":blocks.get("pdf_quality").cloned().unwrap_or(Value::Null),
        "visual_required_count":blocks.get("visual_required_count").cloned().unwrap_or(json!(0)),
        "visual_required_items":blocks.get("visual_required_items").cloned().unwrap_or(json!([])),
        "ocr_verify_sheet":blocks.get("ocr_verify_sheet").cloned().unwrap_or(Value::Null),
        "counts":blocks["counts"],
        "figure_pages":regions["figure_pages"],
        "table_pages":regions["table_pages"],
        "rendered_pages":regions["rendered_pages"],
        "figure_candidates":regions["figure_candidates"],
        "clean_markdown":check_ledger::bind_file(&clean_file)?,
        "layout_blocks":check_ledger::bind_file(&blocks_file)?,
        "visual_review_regions":check_ledger::bind_file(&regions_file)?,
        "boundary":BOUNDARY,
    })))
}

pub fn prepare_paper(opt:PreparePaperOptions<'_>)->Result<Value>{prepare_paper_with_output(opt,false)}
pub fn prepare_paper_with_output(opt:PreparePaperOptions<'_>,compact:bool)->Result<Value>{
    let started=std::time::Instant::now();
    ensure!(opt.pdf_path.is_some()||opt.tex_target.is_some(),"Provide at least one of pdf_path or tex_target");
    ensure!(matches!(opt.render_pages,"auto"|"selective"|"all"|"none"),"render_pages must be auto, selective, all or none");
    ensure!((72..=600).contains(&opt.dpi),"dpi must be between 72 and 600");
    ensure!(!opt.work_dir.is_symlink(),"Refusing a symlink work directory");
    pipeline::ensure_outside_vault(&opt.work_dir.join("probe"))?;
    fs::create_dir_all(opt.work_dir)?;
    let work=files::canonical(opt.work_dir)?;
    let evidence=work.join("evidence");
    // Include source identity and rendering options in each evidence namespace.
    let pdf_binding=opt.pdf_path.map(check_ledger::bind_file).transpose()?;
    let pdf_sha=pdf_binding.as_ref().and_then(|v|v["sha256"].as_str());
    if let Some(expected)=opt.expected_pdf_sha256{ensure!(pdf_sha==Some(expected),"Source PDF SHA-256 mismatch");}
    let render=if opt.render_pages=="auto"{"selective"}else{opt.render_pages};
    let ocr_package=opt.pdf_path.and_then(|_|crate::ocr_backend::discover_bundled_package()).map(|(_,sha)|sha);
    let layout_key=files::digest(&serde_json::to_vec(&json!([crate::ENGINE_VERSION,pdf_binding,render,opt.dpi,ocr_package]))?);
    let layout_dir=evidence.join("pdf-layout").join(layout_key);
    let local_tex=opt.tex_target.and_then(Value::as_str).map(str::trim).map(Path::new).filter(|p|p.is_file());
    let tex_binding=local_tex.map(check_ledger::bind_file).transpose()?;
    let tex_key=files::digest(&serde_json::to_vec(&json!([crate::ENGINE_VERSION,opt.tex_target,tex_binding,pdf_sha]))?);
    let tex_dir=evidence.join("tex").join(tex_key);
    let matched_glossary_file=evidence.join("matched-glossary.md");
    fs::create_dir_all(&evidence)?;

    let tex_started=std::time::Instant::now();
    let mut tex_summary=Value::Null;
    let mut tex_ok=false;
    if let Some(target)=opt.tex_target{
        let cached_tex_struct=tex_dir.join("tex-structure.json");
        let cached_tex_clean=tex_dir.join("clean-source.md");
        if cached_tex_struct.is_file()&&cached_tex_clean.is_file(){
            tex_ok=true;
            tex_summary=json!({
                "status":"cached",
                "tex_structure":check_ledger::bind_file(&cached_tex_struct)?,
                "clean_markdown":check_ledger::bind_file(&cached_tex_clean)?,
            });
        }else if let Some(raw_path)=target.as_str().map(str::trim).filter(|s|Path::new(s).is_file()){
            let r=tex_source::prepare(Path::new(raw_path),"main",&tex_dir,tex_binding.as_ref().and_then(|v|v["sha256"].as_str()))?;
            tex_ok=true;
            tex_summary=json!({
                "status":"ok",
                "source_type":"local_tex",
                "tex_structure":r["structure_json"],
                "clean_markdown":r["clean_markdown"],
                "counts":{
                    "blocks":r["blocks"].as_array().map_or(0,Vec::len),
                    "equations":r["equations"].as_array().map_or(0,Vec::len),
                    "tables":r["tables"].as_array().map_or(0,Vec::len),
                    "figures":r["figures"].as_array().map_or(0,Vec::len),
                    "references":r["references"].as_array().map_or(0,Vec::len),
                },
            });
        }else{
            let r=tex_source::fetch_and_prepare(target,&tex_dir,"main",opt.pdf_path,opt.arxiv_timeout_seconds,true)?;
            tex_ok=matches!(r["status"].as_str(),Some("ok"|"downloaded"|"ready"|"prepared"|"cached"));
            if r.get("clean_markdown").is_some_and(|v|v.is_object()){tex_ok=true;}
            tex_summary=r;
        }
    }

    let tex_ms=tex_started.elapsed().as_millis();
    let pdf_started=std::time::Instant::now();
    let mut pdf_summary=Value::Null;
    let mut figure_candidates=Value::Array(vec![]);
    if let Some(pdf)=opt.pdf_path{
        let r=if let Some(cached)=load_cached_layout(&layout_dir,pdf_sha.context("PDF fingerprint")?)?{
            cached
        }else{
            prepare_pdf_layout(pdf,&layout_dir,"main",render,opt.dpi,pdf_sha)?
        };
        figure_candidates=r["figure_candidates"].clone();
        pdf_summary=r;
    }

    let pdf_ms=pdf_started.elapsed().as_millis();
    let glossary_started=std::time::Instant::now();
    let mut matched_glossary=Value::Null;
    if let Some(gp)=opt.glossary_path{
        if gp.is_file(){
            let mut corpus=String::new();
            let tex_clean=tex_dir.join("clean-source.md");
            if tex_ok && tex_clean.is_file(){corpus.push_str(&fs::read_to_string(&tex_clean)?);corpus.push('\n');}
            let pdf_clean=layout_dir.join("clean-layout.md");
            if opt.pdf_path.is_some() && pdf_clean.is_file(){corpus.push_str(&fs::read_to_string(&pdf_clean)?);}
            if !corpus.trim().is_empty(){
                struct GlossaryAgg{
                    en:String,
                    zh:String,
                    ctx:String,
                    line:u64,
                    has_exact:bool,
                    family_tokens:BTreeSet<String>,
                }
                let gl=glossary::Glossary::new(gp)?;
                let sug=gl.suggest(&corpus,6)?;
                let mut aggregated=BTreeMap::<String,GlossaryAgg>::new();
                for m in sug["matches"].as_array().into_iter().flatten(){
                    let e=&m["entry"];
                    let Some(id)=e["id"].as_str() else{continue;};
                    let kinds:Vec<&str>=m["match_types"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                    let rec=aggregated.entry(id.to_owned()).or_insert_with(||GlossaryAgg{
                        en:e["english"].as_str().unwrap_or("").replace('|',"\\|"),
                        zh:e["translation"].as_str().unwrap_or("").replace('|',"\\|"),
                        ctx:e["context"].as_str().unwrap_or("").replace('|',"\\|"),
                        line:e["line"].as_u64().unwrap_or(0),
                        has_exact:false,
                        family_tokens:BTreeSet::new(),
                    });
                    if kinds.contains(&"exact_token_phrase"){rec.has_exact=true;}
                    if kinds.contains(&"shared_token_family"){
                        if let Some(q)=m["query"].as_str().map(str::trim).filter(|s|!s.is_empty()){
                            rec.family_tokens.insert(q.to_ascii_lowercase());
                        }
                    }
                }
                let total_candidates=aggregated.len();
                let mut kept:Vec<(String,GlossaryAgg)>=aggregated.into_iter()
                    .filter(|(_,r)|r.has_exact||r.family_tokens.len()>=2)
                    .collect();
                let filtered_noise_count=total_candidates.saturating_sub(kept.len());
                let exact_count=kept.iter().filter(|(_,r)|r.has_exact).count();
                let family_count=kept.len().saturating_sub(exact_count);
                kept.sort_by(|(id_a,a),(id_b,b)|b.has_exact.cmp(&a.has_exact).then_with(||id_a.cmp(id_b)));
                let mut md=format!("# 本篇命中术语表（matched-glossary）\n\n来源术语表：`{}`（sha256: `{}`）\n命中词条数：{}（精确短语：{}，多词词族：{}；已过滤单词噪声：{}）\n\n| ID | 英文术语 | 译名 | 语境/备注 | 匹配方式 | 术语表行号 |\n|---|---|---|---|---|---|\n",
                    gl.binding["path"].as_str().unwrap_or(""),
                    gl.binding["sha256"].as_str().unwrap_or(""),
                    kept.len(),
                    exact_count,
                    family_count,
                    filtered_noise_count);
                for (id,r) in &kept{
                    let kind=if r.has_exact{
                        if r.family_tokens.is_empty(){"exact_token_phrase".to_owned()}else{"exact_token_phrase (+family)".to_owned()}
                    }else{
                        let toks:Vec<&str>=r.family_tokens.iter().map(String::as_str).collect();
                        format!("shared_token_family ({} tokens: {})",r.family_tokens.len(),toks.join(", "))
                    };
                    md.push_str(&format!("| {id} | {} | {} | {} | {kind} | {} |\n",r.en,r.zh,r.ctx,r.line));
                }
                pipeline::publish_replace(&matched_glossary_file,md.as_bytes())?;
                matched_glossary=json!({
                    "file":check_ledger::bind_file(&matched_glossary_file)?,
                    "matched_count":kept.len(),
                    "exact_count":exact_count,
                    "family_count":family_count,
                    "filtered_noise_count":filtered_noise_count,
                    "glossary_source":gl.binding,
                });
            }
        }
    }

    let glossary_ms=glossary_started.elapsed().as_millis();
    let mut crop_regions = Vec::new();
    let mut labels = std::collections::BTreeMap::new();
    for candidate in figure_candidates.as_array().into_iter().flatten() {
        let Some(label) = candidate["suggested_fig_id"].as_str()
            .filter(|label| label.starts_with("fig") && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        else { continue; };
        let label_key = label.to_ascii_lowercase();
        let duplicate_index = labels.get(&label_key).copied();
        let filename = if let Some(first) = duplicate_index {
            let first: &mut Value = &mut crop_regions[first];
            first["priority_visual_review"] = json!(true);
            format!("{label}--candidate-{}.png", crop_regions.len() + 1)
        } else {
            labels.insert(label_key, crop_regions.len());
            format!("{label}.png")
        };
        crop_regions.push(json!({
            "page_number":candidate["page"],"bbox":candidate["suggested_bbox"],
            "coordinate_system":crate::pdf_boundary::coordinates(),"label":label,
            "destination":path_text(&work.join("crops").join(filename))?,
            "margin_pt":0,"dpi":300,"transparent":true,"auto_trim":true,
            "priority_visual_review":duplicate_index.is_some() || candidate["confidence"]!="high"
        }));
    }
    let cached=pdf_summary["status"]=="cached";
    let performed=|key:&str|if cached{0}else{pdf_summary["counts"][key].as_u64().unwrap_or(0)};
    let metrics=json!({"tex_cache_hit":tex_summary["status"]=="cached","layout_cache_hit":cached,
        "rendered_pages":performed("rendered_pages"),"ocr_pages":performed("ocr_pages"),"ocr_calls":performed("ocr_calls"),
        "visual_required_count":pdf_summary["visual_required_count"]});
    let timings=json!({"tex":tex_ms,"pdf_layout":pdf_ms,"pdf_detail":pdf_summary["timings_ms"],"glossary":glossary_ms,
        "prepare_total":started.elapsed().as_millis()});
    let report=json!({
        "schema_version":1,"metrics":metrics,"timings_ms":timings,
        "status":"ok",
        "work_dir":path_text(&work)?,
        "tex_succeeded":tex_ok,
        "tex":tex_summary,
        "pdf_layout":pdf_summary,
        "figure_candidates":figure_candidates,
        "crop_regions":crop_regions,
        "crop_contact_sheet_destination":path_text(&work.join("crop-contact-sheet.png"))?,
        "matched_glossary":matched_glossary,
        "source_fidelity":"not_assessed",
        "next_action":"Read the clean source and matched glossary; pass crop_regions directly to render_page_regions_batch and visually check its contact sheet.",
    });
    if !compact{return Ok(report);}
    let save=|name:&str,data:&Value|->Result<Value>{
        let bytes=serde_json::to_vec_pretty(data)?;
        let path=evidence.join(format!("{name}-{}.json",files::digest(&bytes)));
        pipeline::publish(&path,&bytes)?;
        check_ledger::bind_file(&path)
    };
    // The crop API accepts up to 32 regions per call.
    let batches=crop_regions.chunks(32).map(|rows|save("crop-regions",&json!(rows))).collect::<Result<Vec<_>>>()?;
    Ok(json!({"schema_version":1,"status":"ok","source_fidelity":"not_assessed","metrics":metrics,"timings_ms":timings,
        "tex_status":tex_summary["status"],"tex_reason":tex_summary["reason"],"tex_clean_markdown":tex_summary["clean_markdown"],
        "pdf_clean_markdown":pdf_summary["clean_markdown"],"visual_review_regions":pdf_summary["visual_review_regions"],
        "ocr_verify_sheet":pdf_summary["ocr_verify_sheet"]["contact_sheet"]["asset"],"matched_glossary":matched_glossary,
        "crop_count":crop_regions.len(),"crop_batches":batches,"crop_contact_sheet_destination":report["crop_contact_sheet_destination"],
        "receipt":save("prepare-paper",&report)?}))
}

#[cfg(test)]
mod tests{
    use super::*;
    use lopdf::{Document,Object,Stream,dictionary};

    #[test]
    fn heals_hyphenation_keeps_single_column_math_tags_and_sorts_two_columns(){
        // Verify two-column detection ignores short right-aligned math tags like (1) on single-column pages.
        let single_native=json!({"glyphs":[
            {"glyph_index":0,"line_index":0,"text":"First sentence with homol-","space_before":false,"bbox":[72.0,100.0,450.0,112.0]},
            {"glyph_index":1,"line_index":1,"text":"ogy on page one.","space_before":false,"bbox":[72.0,115.0,260.0,127.0]},
            {"glyph_index":2,"line_index":2,"text":"x = y + z","space_before":false,"bbox":[240.0,145.0,340.0,157.0]},
            {"glyph_index":3,"line_index":3,"text":"(1)","space_before":false,"bbox":[480.0,145.0,500.0,157.0]},
        ]});
        let p=analyze_page("main",1,[0.0,0.0,600.0,800.0],&single_native,&json!({"occurrences":[]}),&mut false);
        assert_eq!(p["two_column"],false);
        assert_eq!(p["blocks"][0]["text"],"First sentence with homology on page one.");

        // Real two-column ordering: left column completely before right column.
        let mut two_col_glyphs=Vec::new();
        for i in 0..5{
            let y=100.0+(i as f64)*15.0;
            two_col_glyphs.push(json!({"glyph_index":i*2,"line_index":i*2,"text":format!("Left column line {i} content"),"space_before":false,"bbox":[50.0,y,270.0,y+11.0]}));
            two_col_glyphs.push(json!({"glyph_index":i*2+1,"line_index":i*2+1,"text":format!("Right column line {i} content"),"space_before":false,"bbox":[320.0,y,550.0,y+11.0]}));
        }
        let p2=analyze_page("main",2,[0.0,0.0,600.0,800.0],&json!({"glyphs":two_col_glyphs}),&json!({"occurrences":[]}),&mut false);
        assert_eq!(p2["two_column"],true);
        let texts:Vec<&str>=p2["blocks"].as_array().unwrap().iter().map(|b|b["text"].as_str().unwrap()).collect();
        assert!(texts[0].starts_with("Left column line 0"));
        assert!(texts[1].starts_with("Right column line 0"));
    }

    #[test]
    fn selective_pdf_layout_skips_prose_only_page_raster(){
        let tmp=tempfile::tempdir().unwrap();
        let pdf=tmp.path().join("paper.pdf");
        let mut doc=Document::with_version("1.5");
        let pages=doc.new_object_id();
        let font=doc.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
        let res_text=dictionary!{"Font"=>dictionary!{"F1"=>font}};
        let img=doc.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>1,"Height"=>1,"BitsPerComponent"=>8,"ColorSpace"=>"DeviceRGB"},vec![10,20,30]));
        let res_fig=dictionary!{"Font"=>dictionary!{"F1"=>font},"XObject"=>dictionary!{"Im"=>img}};
        let c1=doc.add_object(Stream::new(dictionary!{},b"BT /F1 12 Tf 40 250 Tm (Title Page) Tj ET".to_vec()));
        let p1=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>c1,"Resources"=>res_text.clone()});
        let c2=doc.add_object(Stream::new(dictionary!{},b"BT /F1 12 Tf 40 250 Tm (Plain prose page two without images.) Tj ET".to_vec()));
        let p2=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>c2,"Resources"=>res_text});
        let c3=doc.add_object(Stream::new(dictionary!{},b"q 40 0 0 30 50 150 cm /Im Do Q BT /F1 12 Tf 40 120 Tm (Figure 1: Sample) Tj ET".to_vec()));
        let p3=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>c3,"Resources"=>res_fig});
        doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(p1),Object::Reference(p2),Object::Reference(p3)],"Count"=>3,"MediaBox"=>vec![0.into(),0.into(),300.into(),300.into()]}));
        let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});doc.trailer.set("Root",catalog);doc.save(&pdf).unwrap();

        let out=tmp.path().join("layout");
        let res=prepare_pdf_layout(&pdf,&out,"main","selective",72,None).unwrap();
        assert_eq!(res["status"],"ok");
        assert_eq!(res["counts"]["rendered_pages"],2);
        let rpages:Vec<u64>=res["rendered_pages"].as_array().unwrap().iter().map(|r|r["page"].as_u64().unwrap()).collect();
        assert_eq!(rpages,vec![1,3]);
        assert!(out.join("pages/page-0001.png").is_file());
        assert!(!out.join("pages/page-0002.png").exists());
        assert!(out.join("pages/page-0003.png").is_file());
        assert!(Path::new(res["clean_markdown"]["path"].as_str().unwrap()).is_file());
        let fc=res["figure_candidates"].as_array().unwrap();
        assert_eq!(fc.len(),1,"{res}");
        assert_eq!(fc[0]["page"],3);
        assert_eq!(fc[0]["suggested_fig_id"],"fig1");
        assert_eq!(fc[0]["single_object_reusable"],true);

        let gl_path=tmp.path().join("glossary.md");
        fs::write(&gl_path,"| ID | 英文术语 | 中文译名 | 备注 |\n| --- | --- | --- | --- |\n| T_HOM | homology | 同调 | 代数拓扑 |\n| T_FAM | persistent groups | 持续群 | 拓扑 |\n| T_NOISE | stable stratification | 稳定层结 | 大气 |\n| T_MISSING | zeta function | ζ函数 | 未出现 |\n").unwrap();
        let tex_path=tmp.path().join("paper.tex");
        fs::write(&tex_path,"\\documentclass{article}\n\\title{Persistent Homology}\n\\begin{document}\nWe compute persistent homology groups cleanly with stable behavior.\n\\end{document}\n").unwrap();
        let paper_work=tmp.path().join("paper-work");
        let tex_val=json!(tex_path.to_string_lossy().to_string());
        let prep=prepare_paper(PreparePaperOptions{
            pdf_path:Some(&pdf),tex_target:Some(&tex_val),work_dir:&paper_work,
            glossary_path:Some(&gl_path),render_pages:"auto",dpi:72,expected_pdf_sha256:None,arxiv_timeout_seconds:5,
        }).unwrap();
        assert_eq!(prep["tex_succeeded"],true);
        assert_eq!(prep["matched_glossary"]["matched_count"],2);
        assert_eq!(prep["matched_glossary"]["exact_count"],1);
        assert_eq!(prep["matched_glossary"]["family_count"],1);
        assert_eq!(prep["matched_glossary"]["filtered_noise_count"],1);
        let mg=fs::read_to_string(prep["matched_glossary"]["file"]["path"].as_str().unwrap()).unwrap();
        assert!(mg.contains("T_HOM")&&mg.contains("T_FAM")&&!mg.contains("T_NOISE")&&!mg.contains("T_MISSING"),"{mg}");
        let prep_cached=prepare_paper(PreparePaperOptions{
            pdf_path:Some(&pdf),tex_target:Some(&tex_val),work_dir:&paper_work,
            glossary_path:Some(&gl_path),render_pages:"auto",dpi:72,expected_pdf_sha256:None,arxiv_timeout_seconds:5,
        }).unwrap();
        assert_eq!(prep_cached["pdf_layout"]["status"],"cached");
        let compact=prepare_paper_with_output(PreparePaperOptions{
            pdf_path:Some(&pdf),tex_target:Some(&tex_val),work_dir:&paper_work,
            glossary_path:Some(&gl_path),render_pages:"auto",dpi:72,expected_pdf_sha256:None,arxiv_timeout_seconds:5,
        },true).unwrap();
        assert_eq!(compact["metrics"]["layout_cache_hit"],true);
        assert_eq!(compact["metrics"]["rendered_pages"],0);
        assert_eq!(compact["metrics"]["ocr_calls"],0);
        let receipt=crate::pipeline::read_json(Path::new(compact["receipt"]["path"].as_str().unwrap())).unwrap();
        assert_eq!(receipt["crop_regions"],prep["crop_regions"]);
        assert!(compact.get("figure_candidates").is_none());
        assert!(compact["pdf_clean_markdown"]["path"].is_string());
    }

    #[test]
    fn vector_figure_above_caption_not_collapsed_by_tikz_labels(){
        let glyphs=json!([
            {"glyph_index":0,"line_index":0,"text":"This is a full-width prose paragraph preceding a pure vector TikZ diagram.","space_before":false,"bbox":[72.0,100.0,520.0,112.0]},
            {"glyph_index":1,"line_index":1,"text":"It explains the boundary operator construction before Figure 1 appears below.","space_before":false,"bbox":[72.0,114.0,520.0,126.0]},
            {"glyph_index":2,"line_index":2,"text":"v0","space_before":false,"bbox":[150.0,248.0,164.0,258.0]},
            {"glyph_index":3,"line_index":3,"text":"v1","space_before":false,"bbox":[150.0,262.0,164.0,272.0]},
            {"glyph_index":4,"line_index":4,"text":"(a)","space_before":false,"bbox":[145.0,276.0,160.0,286.0]},
            {"glyph_index":5,"line_index":5,"text":"(b)","space_before":false,"bbox":[350.0,276.0,365.0,286.0]},
            {"glyph_index":6,"line_index":6,"text":"Figure 1: Directed graphs used for Mayer path homology.","space_before":false,"bbox":[100.0,295.0,490.0,307.0]},
        ]);
        let p=analyze_page("main",5,[0.0,0.0,595.0,842.0],&json!({"glyphs":glyphs}),&json!({"occurrences":[]}),&mut false);
        let fc=p["figure_candidates"].as_array().unwrap();
        assert_eq!(fc.len(),1,"{p}");
        let b=fc[0]["suggested_bbox"].as_array().unwrap();
        let h=b[3].as_f64().unwrap()-b[1].as_f64().unwrap();
        assert!(h>=120.0,"expected >=120pt height, got {h} ({b:?})");

        let glyphs2=json!([
            {"glyph_index":0,"line_index":0,"text":"The rank of H is called the Mayer Path Betti number. The following examples illustrate this construction.","space_before":false,"bbox":[72.0,620.0,523.3,634.6]},
            {"glyph_index":1,"line_index":1,"text":"(a) L1 - 4 node-feedback loop (b) L2 - Bi-parallel (c) L3 - Bi-fan","space_before":false,"bbox":[72.0,737.5,492.6,746.9]},
            {"glyph_index":2,"line_index":2,"text":"Figure 1: Digraphs","space_before":false,"bbox":[250.0,758.3,332.9,768.3]},
        ]);
        let p2=analyze_page("main",5,[0.0,0.0,595.3,842.0],&json!({"glyphs":glyphs2}),&json!({"occurrences":[]}),&mut false);
        let fc2=p2["figure_candidates"].as_array().unwrap();
        assert_eq!(fc2.len(),1,"{p2}");
        let b2=fc2[0]["suggested_bbox"].as_array().unwrap();
        let h2=b2[3].as_f64().unwrap()-b2[1].as_f64().unwrap();
        assert!(h2>=100.0,"expected >=100pt height above subfigure row, got {h2} ({b2:?})");
    }

    #[test]
    fn garbled_pua_and_display_math_emit_visual_required_and_verify_sheet(){
        // 1. Unit check: garbled PUA/U+FFFD text is NEVER emitted into clean-layout.md;
        //    instead a VISUAL_REQUIRED marker is emitted.
        let glyphs=json!([
            {"glyph_index":0,"line_index":0,"text":"Clean introductory paragraph before formula.","space_before":false,"bbox":[72.0,100.0,480.0,112.0]},
            {"glyph_index":1,"line_index":1,"text":"Corrupted \u{E001}\u{FFFD} math glyph line","space_before":false,"bbox":[72.0,140.0,320.0,152.0]},
            {"glyph_index":2,"line_index":2,"text":"x = y + z (1)","space_before":false,"bbox":[220.0,190.0,340.0,202.0]},
        ]);
        let page=analyze_page_with_quality("main",2,[0.0,0.0,600.0,800.0],&json!({"glyphs":glyphs}),&json!({"occurrences":[]}),&mut false,None);
        let md=render_clean_layout_markdown("main",&[page]);
        assert!(md.contains("Clean introductory paragraph before formula."),"{md}");
        assert!(md.contains("<!-- VISUAL_REQUIRED: main-p2-b2 | reason=garbled_glyphs"),"{md}");
        assert!(md.contains("<!-- VISUAL_REQUIRED: main-p2-b3 | reason=display_formula"),"{md}");
        assert!(!md.contains('\u{E001}')&&!md.contains('\u{FFFD}'),"garbled glyph leaked into clean-layout.md: {md}");

        // 2. End-to-end PDF check: Page 1 affiliation + display formula + Page 2 Type-3 unmapped font
        //    must generate ocr-verify-sheet.png and emit VISUAL_REQUIRED markers.
        let tmp=tempfile::tempdir().unwrap();
        let pdf=tmp.path().join("firewall.pdf");
        let mut doc=Document::with_version("1.5");
        let pages=doc.new_object_id();
        let font=doc.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
        let res_p1=dictionary!{"Font"=>dictionary!{"F1"=>font}};
        let c1=doc.add_object(Stream::new(dictionary!{},b"BT /F1 12 Tf 1 0 0 1 40 720 Tm (Department of Mathematics, Example University) Tj ET BT /F1 12 Tf 1 0 0 1 40 680 Tm (This full width sentence spans across the page margin to establish page column width cleanly.) Tj ET BT /F1 12 Tf 1 0 0 1 240 620 Tm (x = y + z \\(1\\)) Tj ET".to_vec()));
        let p1=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>c1,"Resources"=>res_p1});

        let t3=doc.add_object(dictionary!{
            "Type"=>"Font","Subtype"=>"Type3",
            "FontBBox"=>vec![0.into(),0.into(),1000.into(),1000.into()],
            "FontMatrix"=>vec![0.001.into(),0.into(),0.into(),0.001.into(),0.into(),0.into()],
            "CharProcs"=>dictionary!{},"Encoding"=>"WinAnsiEncoding",
            "FirstChar"=>32,"LastChar"=>32,"Widths"=>vec![500.into()],
        });
        let c2=doc.add_object(Stream::new(dictionary!{},b"BT /F3 12 Tf 40 700 Tm ( ) Tj ET".to_vec()));
        let p2=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>c2,"Resources"=>dictionary!{"Font"=>dictionary!{"F3"=>t3}}});
        doc.objects.insert(pages,Object::Dictionary(dictionary!{
            "Type"=>"Pages","Kids"=>vec![Object::Reference(p1),Object::Reference(p2)],
            "Count"=>2,"MediaBox"=>vec![0.into(),0.into(),600.into(),800.into()]
        }));
        let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});
        doc.trailer.set("Root",catalog);
        doc.save(&pdf).unwrap();

        let out=tmp.path().join("layout");
        let res=prepare_pdf_layout(&pdf,&out,"main","selective",72,None).unwrap();
        assert!(res["visual_required_count"].as_u64().unwrap()>=2,"{res}");
        assert!(out.join("ocr-verify-sheet.png").is_file(),"expected ocr-verify-sheet.png");
        assert!(res["ocr_verify_sheet"]["contact_sheet"]["asset"]["path"].as_str().unwrap().ends_with("ocr-verify-sheet.png"));
        assert!(res["pdf_quality"]["type3_or_unmapped_font_pages"].as_array().unwrap().iter().any(|v|v==2),"{res}");
        let clean=fs::read_to_string(res["clean_markdown"]["path"].as_str().unwrap()).unwrap();
        assert!(clean.contains("<!-- VISUAL_REQUIRED:"),"{clean}");
        assert!(!clean.contains('\u{FFFD}')&&!clean.contains('\u{E001}'),"{clean}");
    }
}
