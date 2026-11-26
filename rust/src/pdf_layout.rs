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
    s.chars().any(|c|c=='\u{FFFD}'||c=='\0'||('\u{E000}'..='\u{F8FF}').contains(&c))
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

pub(crate) fn analyze_page(source_id:&str,page:u64,bounds:Box4,native:&Value,structure:&Value,in_references:&mut bool)->Value{
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

    let mut blocks=Vec::with_capacity(drafts.len());
    let mut figure_candidates=Vec::new();
    let mut has_figure_caption=false;
    let mut has_table_caption=false;
    let mut prose_lines_set=BTreeSet::<u64>::new();
    for d in &drafts{
        if d.kind!="caption" && (d.text.len()>=45 || d.line_indices.len()>=2 || d.kind=="heading"){
            prose_lines_set.extend(d.line_indices.iter().copied());
        }
    }
    let page_w=(max_x-min_x).max(1.0);
    let mid_x=min_x+0.5*page_w;

    for (i,d) in drafts.iter().enumerate(){
        let id=format!("{source_id}-p{page}-b{}",i+1);
        if d.kind=="caption"{
            if rx!(r"^(?:Table|Tbl\.|表)").is_match(d.text.trim()){
                has_table_caption=true;
                let region=[bounds[0],(d.bbox[1]-18.0).max(bounds[1]),bounds[2],(d.bbox[3]+220.0).min(bounds[3])];
                visual_regions.push(json!({"page":page,"block_id":id,"kind":"table_region","bbox":region,"reason":"table caption detected"}));
            }else{
                has_figure_caption=true;
                let region=[bounds[0],(d.bbox[1]-220.0).max(bounds[1]),bounds[2],(d.bbox[3]+20.0).min(bounds[3])];
                visual_regions.push(json!({"page":page,"block_id":id,"kind":"figure_region","bbox":region,"reason":"figure caption detected"}));

                let (col_x0,col_x1)=if two_column&&d.col==-1{
                    (bounds[0],(mid_x+0.04*page_w).min(bounds[2]))
                }else if two_column&&d.col==1{
                    ((mid_x-0.04*page_w).max(bounds[0]),bounds[2])
                }else{(bounds[0],bounds[2])};

                let prev_bottom=drafts[..i].iter()
                    .filter(|p|p.bbox[3]<=d.bbox[1]+2.0
                        &&(p.col==d.col||p.col==0||d.col==0)
                        &&(p.kind=="heading"||p.kind=="caption"||p.text.len()>=45||p.line_indices.len()>=2))
                    .map(|p|p.bbox[3])
                    .reduce(f64::max);
                let search_top=prev_bottom.unwrap_or_else(||bounds[1]+0.04*(bounds[3]-bounds[1])).min(d.bbox[1]-8.0).max(bounds[1]);

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

                let cap_top=(d.bbox[1]-1.0).max(bounds[1]+8.0);
                let suggested_bbox=if let Some(u)=fig_union{
                    let top=(u[1]-8.0).max(bounds[1]).min(cap_top-4.0);
                    let bottom=(u[3]+6.0).min(cap_top).max(top+4.0);
                    [(u[0]-8.0).max(bounds[0]),top,(u[2]+8.0).min(bounds[2]).max((u[0]-8.0).max(bounds[0])+4.0),bottom]
                }else{
                    let top=(d.bbox[1]-220.0).max(search_top).min(cap_top-4.0);
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
        }else if d.garbled{
            visual_regions.push(json!({"page":page,"block_id":id,"kind":"garbled_glyphs","bbox":d.bbox,"reason":"replacement or private-use unicode glyph detected"}));
        }else if d.kind=="display_math" && d.line_indices.len()>=2{
            visual_regions.push(json!({"page":page,"block_id":id,"kind":"multiline_formula","bbox":d.bbox,"reason":"multi-line display math block"}));
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
        }));
    }

    let image_count=structure["occurrences"].as_array().map_or(0,Vec::len);
    let classification=ocr_pdf::classify(native,structure);
    json!({
        "page":page,
        "two_column":two_column,
        "raw_line_count":raw_lines.len(),
        "margin_lines":margin_lines,
        "blocks":blocks,
        "visual_review_regions":visual_regions,
        "figure_candidates":figure_candidates,
        "has_images":image_count>0,
        "has_figure_caption":has_figure_caption,
        "has_table_caption":has_table_caption,
        "ocr_whole_page":classification["ocr_whole_page"],
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

fn render_clean_layout_markdown(pages:&[Value])->String{
    let mut out=String::from("<!-- Derived from pure-Rust PDF geometric layout clustering. Verify tables, complex formulas & cropped figures against rendered page/region images. -->\n\n");
    for p in pages{
        let page_num=p["page"].as_u64().unwrap_or(0);
        out.push_str(&format!("<!-- page {page_num} -->\n\n"));
        for b in p["blocks"].as_array().into_iter().flatten(){
            let id=b["id"].as_str().unwrap_or("");
            let kind=b["kind"].as_str().unwrap_or("paragraph");
            let text=b["text"].as_str().unwrap_or("").trim();
            if text.is_empty(){continue;}
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

    let mut backend=Backend::open(source_pdf,expected_sha)?;
    let source_binding=backend.source_binding().clone();
    if render_pages!="none"{backend.preflight_whole_pages(dpi)?;}

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
    let mut exported_objects=Vec::new();
    let mut seen_objects=BTreeSet::<(u32,u16)>::new();

    for n in 1..=total_pages{
        let page=n as u64;
        let (_,_,bounds,_)=backend.geometry(page)?;
        let native=backend.text(page).unwrap_or_else(|e|json!({
            "schema_version":1,"status":"unavailable","error":format!("{e:#}"),
            "page":page,"plain_text":"","glyphs":[],"warnings":[],"truncated":true
        }));
        let structure=backend.structure(page)?;
        let mut page_doc=analyze_page(source_id,page,bounds,&native,&structure,&mut in_references);
        raw_line_total+=page_doc["raw_line_count"].as_u64().unwrap_or(0) as usize;
        block_total+=page_doc["blocks"].as_array().map_or(0,Vec::len);
        let has_images=page_doc["has_images"]==true;
        let has_fig=page_doc["has_figure_caption"]==true;
        let has_tbl=page_doc["has_table_caption"]==true;
        let ocr_needed=page_doc["ocr_whole_page"]==true;
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
            "selective"=>page==1 || has_images || has_fig || has_tbl || ocr_needed,
            _=>false,
        };
        if should_render{
            let dest=pages_dir.join(format!("page-{n:04}.png"));
            let r=backend.render(page,None,dpi,&dest)?;
            page_doc["render"]=r["asset"].clone();
            rendered_pages.push(json!({"page":page,"path":r["asset"]["path"],"sha256":r["asset"]["sha256"]}));
        }else{
            page_doc["render"]=Value::Null;
        }
        analyzed.push(page_doc);
    }
    backend.verify()?;

    let clean_md=render_clean_layout_markdown(&analyzed);
    let clean_binding=write_atomic_new(&clean_file,clean_md.as_bytes())?;
    let layout_doc=json!({
        "schema_version":1,"backend":"pdfcraft-layout-v1","source_file":source_binding,"source_id":source_id,
        "page_count":total_pages,"render_mode":render_pages,"dpi":dpi,
        "counts":{"raw_lines":raw_line_total,"clustered_blocks":block_total,"visual_review_regions":all_regions.len(),"rendered_pages":rendered_pages.len(),"exported_image_objects":exported_objects.len()},
        "image_objects":exported_objects,"pages":analyzed,"boundary":BOUNDARY
    });
    let mut layout_bytes=serde_json::to_vec_pretty(&layout_doc)?;layout_bytes.push(b'\n');
    let layout_binding=write_atomic_new(&blocks_file,&layout_bytes)?;

    let regions_doc=json!({
        "schema_version":1,"source_file":source_binding,"regions":all_regions,
        "figure_candidates":all_figure_candidates,
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
        "counts":layout_doc["counts"],
        "figure_pages":figure_pages,
        "table_pages":table_pages,
        "image_pages":image_pages,
        "rendered_pages":rendered_pages,
        "figure_candidates":all_figure_candidates,
        "clean_markdown":clean_binding,
        "layout_blocks":layout_binding,
        "visual_review_regions":region_binding,
        "next_action":"Read clean-layout.md; inspect only rendered Page 1 and figure/table/visual_review_regions pages instead of prose pages.",
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

fn load_cached_layout(layout_dir:&Path,pdf_path:&Path,expected_sha:Option<&str>)->Result<Option<Value>>{
    let blocks_file=layout_dir.join("layout-blocks.json");
    let regions_file=layout_dir.join("visual-review-regions.json");
    let clean_file=layout_dir.join("clean-layout.md");
    if !(blocks_file.is_file()&&regions_file.is_file()&&clean_file.is_file()){
        return Ok(None);
    }
    let pdf_snap=files::snapshot(pdf_path)?;
    let pdf_sha=pdf_snap["sha256"].as_str().context("pdf sha256")?;
    if let Some(exp)=expected_sha{
        ensure!(pdf_sha==exp,"Source PDF SHA-256 mismatch against expected_pdf_sha256");
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

pub fn prepare_paper(opt:PreparePaperOptions<'_>)->Result<Value>{
    ensure!(opt.pdf_path.is_some()||opt.tex_target.is_some(),"Provide at least one of pdf_path or tex_target");
    ensure!(matches!(opt.render_pages,"auto"|"selective"|"all"|"none"),"render_pages must be auto, selective, all or none");
    ensure!((72..=600).contains(&opt.dpi),"dpi must be between 72 and 600");
    ensure!(!opt.work_dir.is_symlink(),"Refusing a symlink work directory");
    pipeline::ensure_outside_vault(&opt.work_dir.join("probe"))?;
    fs::create_dir_all(opt.work_dir)?;
    let work=files::canonical(opt.work_dir)?;
    let evidence=work.join("evidence");
    let tex_dir=evidence.join("tex");
    let layout_dir=evidence.join("pdf-layout");
    let matched_glossary_file=evidence.join("matched-glossary.md");
    fs::create_dir_all(&evidence)?;

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
            let r=tex_source::prepare(Path::new(raw_path),"main",&tex_dir,None)?;
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

    let mut pdf_summary=Value::Null;
    let mut figure_candidates=Value::Array(vec![]);
    if let Some(pdf)=opt.pdf_path{
        let eff_render=if opt.render_pages=="auto"{"selective"}else{opt.render_pages};
        let r=if let Some(cached)=load_cached_layout(&layout_dir,pdf,opt.expected_pdf_sha256)?{
            cached
        }else{
            prepare_pdf_layout(pdf,&layout_dir,"main",eff_render,opt.dpi,opt.expected_pdf_sha256)?
        };
        figure_candidates=r["figure_candidates"].clone();
        pdf_summary=r;
    }

    let mut matched_glossary=Value::Null;
    if let Some(gp)=opt.glossary_path{
        if gp.is_file(){
            let mut corpus=String::new();
            let tex_clean=tex_dir.join("clean-source.md");
            if tex_clean.is_file(){corpus.push_str(&fs::read_to_string(&tex_clean)?);corpus.push('\n');}
            let pdf_clean=layout_dir.join("clean-layout.md");
            if pdf_clean.is_file(){corpus.push_str(&fs::read_to_string(&pdf_clean)?);}
            if !corpus.trim().is_empty(){
                let gl=glossary::Glossary::new(gp)?;
                let sug=gl.suggest(&corpus,6)?;
                let mut rows=BTreeMap::<String,(String,String,String,String,u64)>::new();
                for m in sug["matches"].as_array().into_iter().flatten(){
                    let e=&m["entry"];
                    let Some(id)=e["id"].as_str() else{continue;};
                    let kinds:Vec<&str>=m["match_types"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                    let en=e["english"].as_str().unwrap_or("").replace('|',"\\|");
                    let zh=e["translation"].as_str().unwrap_or("").replace('|',"\\|");
                    let ctx=e["context"].as_str().unwrap_or("").replace('|',"\\|");
                    let line=e["line"].as_u64().unwrap_or(0);
                    rows.entry(id.to_owned()).or_insert((en,zh,ctx,kinds.join(", "),line));
                }
                let mut md=format!("# 本篇命中术语表（matched-glossary）\n\n来源术语表：`{}`（sha256: `{}`）\n命中词条数：{}\n\n| ID | 英文术语 | 译名 | 语境/备注 | 匹配方式 | 术语表行号 |\n|---|---|---|---|---|---|\n",
                    gl.binding["path"].as_str().unwrap_or(""),
                    gl.binding["sha256"].as_str().unwrap_or(""),
                    rows.len());
                for (id,(en,zh,ctx,kind,line)) in &rows{
                    md.push_str(&format!("| {id} | {en} | {zh} | {ctx} | {kind} | {line} |\n"));
                }
                pipeline::publish_replace(&matched_glossary_file,md.as_bytes())?;
                matched_glossary=json!({
                    "file":check_ledger::bind_file(&matched_glossary_file)?,
                    "matched_count":rows.len(),
                    "glossary_source":gl.binding,
                });
            }
        }
    }

    Ok(json!({
        "schema_version":1,
        "status":"ok",
        "work_dir":path_text(&work)?,
        "tex_succeeded":tex_ok,
        "tex":tex_summary,
        "pdf_layout":pdf_summary,
        "figure_candidates":figure_candidates,
        "matched_glossary":matched_glossary,
        "source_fidelity":"not_assessed",
        "next_action":"Read clean-source.md (if TeX succeeded) / clean-layout.md and matched-glossary.md; pass figure_candidates to pdf_image_objects.render_page_regions_batch for crop verification.",
    }))
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
        fs::write(&gl_path,"| ID | 英文术语 | 中文译名 | 备注 |\n| --- | --- | --- | --- |\n| T_HOM | homology | 同调 | 代数拓扑 |\n| T_MISSING | zeta function | ζ函数 | 未出现 |\n").unwrap();
        let tex_path=tmp.path().join("paper.tex");
        fs::write(&tex_path,"\\documentclass{article}\n\\title{Persistent Homology}\n\\begin{document}\nWe compute homology groups cleanly.\n\\end{document}\n").unwrap();
        let paper_work=tmp.path().join("paper-work");
        let tex_val=json!(tex_path.to_string_lossy().to_string());
        let prep=prepare_paper(PreparePaperOptions{
            pdf_path:Some(&pdf),tex_target:Some(&tex_val),work_dir:&paper_work,
            glossary_path:Some(&gl_path),render_pages:"auto",dpi:72,expected_pdf_sha256:None,arxiv_timeout_seconds:5,
        }).unwrap();
        assert_eq!(prep["tex_succeeded"],true);
        assert_eq!(prep["matched_glossary"]["matched_count"],1);
        let mg=fs::read_to_string(prep["matched_glossary"]["file"]["path"].as_str().unwrap()).unwrap();
        assert!(mg.contains("T_HOM")&&!mg.contains("T_MISSING"),"{mg}");
        let prep_cached=prepare_paper(PreparePaperOptions{
            pdf_path:Some(&pdf),tex_target:Some(&tex_val),work_dir:&paper_work,
            glossary_path:Some(&gl_path),render_pages:"auto",dpi:72,expected_pdf_sha256:None,arxiv_timeout_seconds:5,
        }).unwrap();
        assert_eq!(prep_cached["pdf_layout"]["status"],"cached");
    }
}
