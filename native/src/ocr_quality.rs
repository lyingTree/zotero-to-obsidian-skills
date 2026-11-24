//! Audit real, hash-bound OCR hypotheses. Never assert semantic correctness,
//! modify source text or infer that a model score is a calibrated probability.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,HashSet},path::{Path,PathBuf},fs};
use crate::{files,check_ledger,check_write,path,parse_json};

const MAX_JSON:u64=128*1024*1024;
const MAX_BLOCKS:usize=4096;
const BOUNDARY:&str="Deterministic triage of independently bound OCR hypotheses only. Even matching readings and high model scores cannot establish source correctness, coverage, formula equality, or acceptance. Every text block remains pending comparison with the original image.";
type Box4=[f64;4];
fn is_hash(v:&str)->bool{v.len()==64&&v.bytes().all(|b|b.is_ascii_hexdigit()&&!b.is_ascii_uppercase())}
fn bind(v:&Value)->Result<Value>{
    let p=path(&v["path"])?;check_write::no_link(&p)?;
    let hash=v["sha256"].as_str().filter(|s|is_hash(s)).context("Actual SHA-256 required")?;
    let actual=check_ledger::bind_file(&p)?;
    ensure!(actual["sha256"]==hash,"OCR_REVIEW_SOURCE_CHANGED: {}",p.display());Ok(actual)
}
fn load(p:&Path,hash:&str)->Result<(Value,Value)>{
    ensure!(is_hash(hash),"Explicit independently selected evidence SHA-256 required");
    check_write::no_link(p)?;
    ensure!(fs::metadata(p)?.len()<=MAX_JSON,"OCR review input too large");
    let (bytes,binding)=files::read_bound(p)?;
    ensure!(bytes.len() as u64<=MAX_JSON&&binding["sha256"]==hash,"OCR_REVIEW_RECORD_HASH_MISMATCH");
    let v=parse_json(std::str::from_utf8(&bytes)?)?;
    ensure!(check_ledger::bind_file(p)?==binding,"OCR review record changed while reading");
    Ok((v,binding))
}
fn quad(v:&Value,w:f64,h:f64)->Option<Box4>{
    let a=v.as_array().filter(|a|a.len()==4)?;
    let mut b=[f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY];
    for p in a{
        let p=p.as_array().filter(|a|a.len()==2)?;
        let (x,y)=(p[0].as_f64()?,p[1].as_f64()?);
        if !x.is_finite()||!y.is_finite()||x<0.||y<0.||x>w||y>h{return None;}
        b[0]=b[0].min(x);b[1]=b[1].min(y);b[2]=b[2].max(x);b[3]=b[3].max(y);
    }
    (b[0]<b[2]&&b[1]<b[3]).then_some(b)
}
fn overlap(a:Box4,b:Box4)->f64{
    let intersection=(a[2].min(b[2])-a[0].max(b[0])).max(0.)*(a[3].min(b[3])-a[1].max(b[1])).max(0.);
    let union=(a[2]-a[0])*(a[3]-a[1])+(b[2]-b[0])*(b[3]-b[1])-intersection;
    if union>0.{intersection/union}else{0.}
}
fn score(v:&Value)->Option<f64>{v.as_f64().filter(|s|s.is_finite()&&(0. ..=1.).contains(s))}
fn scientific(s:&str)->bool{
    s.chars().any(|c|c.is_numeric()||matches!(c,'\u{00b0}'|'\u{00b1}'|'\u{00d7}'|'\u{00f7}'|'\u{2212}'|'\u{221e}'|'\u{2260}'|'\u{2264}'|'\u{2265}'|'\u{00b5}'|'%'|'='|'+'|'^'|'_'|'\\'|'±')||
        ('\u{0370}'..='\u{03ff}').contains(&c)||('\u{2070}'..='\u{209f}').contains(&c)||('\u{2200}'..='\u{22ff}').contains(&c))
}
fn whitespace_only(s:&str)->String{s.split_whitespace().collect::<Vec<_>>().join(" ")}
fn engine_identity(v:&Value)->Value{
    // Compare the actual bound recognition assets, not a display label or
    // merely a different path. A distinct checkpoint is still not proof of
    // independent observations.
    let package=&v["model_package"];
    json!({"backend":package["backend"],"revision":package["backend_revision"],
        "detector":package["resources"]["detector"]["sha256"],"recognizer":package["resources"]["recognizer"]["sha256"],
        "dictionary":package["resources"]["dictionary"]["sha256"]})
}
struct Evidence{
    value:Value,
    binding:Value,
    image:Value,
    size:[u64;2],
    boxes:Vec<Option<Box4>>,
    bindings:Vec<Value>,
}
impl Evidence{
    fn open(p:&Path,hash:&str)->Result<Self>{
        let (value,binding)=load(p,hash)?;
        ensure!(value["schema_version"].as_u64()==Some(1)&&value["acceptance_ready"]==false&&value["source_fidelity"]=="not_assessed",
            "Expected a pending native OCR evidence record");
        ensure!(matches!(value["status"].as_str(),Some("ocr_candidate"|"no_text_detected_review_required"|"table_candidate_review_required")),
            "Not a native OCR hypothesis");
        let image=bind(&value["source_image"])?;
        let png=path(&image["path"])?;
        ensure!(fs::metadata(&png)?.len()<=128*1024*1024,"Review raster exceeds input budget");
        let (w,h)=image::ImageReader::open(&png)?.with_guessed_format()?.into_dimensions()?;
        ensure!(w>0&&h>0&&u64::from(w)*u64::from(h)<=20_000_000,"Invalid or oversized review raster");
        let size=[u64::from(w),u64::from(h)];
        ensure!(value["source_image_size"]==json!(size),"OCR raster geometry no longer matches its record");
        let mut bindings=vec![binding.clone(),image.clone()];
        let package=&value["model_package"];
        ensure!(package["backend"].as_str().is_some_and(|s|!s.is_empty())&&package["backend_revision"].as_str().is_some_and(|s|!s.is_empty()),
            "Missing actual backend identity");
        bindings.push(bind(&package["manifest"])?);
        let resources=package["resources"].as_object().context("Missing model resource evidence")?;
        for required in ["runtime","detector","recognizer","dictionary"]{ensure!(resources.contains_key(required),"Missing OCR resource {required}");}
        for resource in resources.values(){bindings.push(bind(resource)?);}
        if let Some(source)=value.get("source_pdf"){
            let source=bind(source)?;
            ensure!(value["render"]["source_file"]==source&&value["render"]["asset"]==image&&
                value["page"].as_u64().is_some_and(|n|n>0)&&value["render"]["page"]==value["page"],
                "PDF source or page does not match the recorded OCR raster");
            bindings.push(source);
        }
        let blocks=value["blocks"].as_array().filter(|a|a.len()<=MAX_BLOCKS).context("OCR block budget exceeded")?;
        let mut boxes=Vec::with_capacity(blocks.len());
        for (i,b) in blocks.iter().enumerate(){
            ensure!(b["index"].as_u64()==Some(i as u64)&&b["text"].is_string(),"Invalid OCR block index or reading");
            let words=b["words"].as_array().context("Missing word-level evidence")?;
            ensure!(words.len()<=8192,"OCR word budget exceeded");
            let b=if b["location_valid"]==true{quad(&b["original_pixel_quad"],w as f64,h as f64)}else{None};
            boxes.push(b);
        }
        if let Some(text)=value.get("plain_text"){
            let joined=blocks.iter().map(|b|b["text"].as_str().unwrap()).collect::<Vec<_>>().join("\n");
            ensure!(text.as_str()==Some(joined.as_str()),"OCR plain text does not match its retained block readings");
        }
        let layout=value["layout_regions"].as_array().filter(|a|a.len()<=MAX_BLOCKS).context("OCR layout region budget exceeded")?;
        for (i,r) in layout.iter().enumerate(){ensure!(r["index"].as_u64()==Some(i as u64),"Invalid layout region identity");}
        let e=Self{value,binding,image,size,boxes,bindings};e.verify()?;Ok(e)
    }
    fn verify(&self)->Result<()>{
        // Avoid repeated full hashes for duplicate records.
        let mut seen=HashSet::new();
        for b in &self.bindings{
            let p=b["path"].as_str().context("Missing binding path")?;
            if seen.insert(p){ensure!(bind(b)?==*b,"OCR audit input changed");}
        }
        Ok(())
    }
}
fn candidates(a:&[Option<Box4>],b:&[Option<Box4>])->(Vec<Vec<usize>>,Vec<Vec<usize>>){
    // Spatial sweep: sorted left edges, early stop; no dense N×M allocation.
    let mut right:Vec<_>=b.iter().enumerate().filter_map(|(j,v)|v.map(|r|(j,r))).collect();
    right.sort_by(|a,b|a.1[0].total_cmp(&b.1[0]));
    let mut forward=vec![vec![];a.len()];let mut reverse=vec![vec![];b.len()];
    for (i,aa) in a.iter().enumerate(){
        let Some(aa)=*aa else{continue;};
        for (j,bb) in &right{
            if bb[0]>=aa[2]{break;}
            if bb[2]<=aa[0]||bb[3]<=aa[1]||bb[1]>=aa[3]{continue;}
            if overlap(aa,*bb)>=0.5{forward[i].push(*j);reverse[*j].push(i);}
        }
    }
    (forward,reverse)
}
fn triage(a:&Evidence,b:Option<&Evidence>)->Result<Value>{
    if let Some(b)=b{
        ensure!(a.image==b.image&&a.size==b.size,"Independent OCR readings must refer to the very same actual raster bytes and dimensions");
        ensure!(a.binding!=b.binding,"Do not compare an OCR record with itself");
    }
    let (forward,reverse)=if let Some(b)=b{candidates(&a.boxes,&b.boxes)}else{(vec![vec![];a.boxes.len()],vec![])};
    let mut tasks=Vec::new();let mut used=HashSet::new();let mut counts=BTreeMap::<String,usize>::new();
    let primary=a.value["blocks"].as_array().unwrap();
    for (i,row) in primary.iter().enumerate(){
        let text=row["text"].as_str().unwrap();
        let mut reasons=Vec::<&str>::new();
        if a.boxes[i].is_none(){reasons.push("invalid_or_missing_location");}
        if score(&row["text_score"]).is_none_or(|s|s<0.90)||score(&row["box_score"]).is_none_or(|s|s<0.60){reasons.push("low_or_unknown_engine_score");}
        if scientific(text){reasons.push("scientific_symbols_or_numbers");}
        if text.trim().is_empty()||text.contains('\u{fffd}')||text.contains('\0'){reasons.push("empty_or_unknown_character");}
        if row["words"].as_array().unwrap().iter().any(|w|score(&w["score"]).is_none_or(|s|s<0.90)||w["location_valid"]!=true){reasons.push("word_evidence_uncertain");}
        let (other,comparison)=if let Some(b)=b{
            if forward[i].len()==1&&reverse[forward[i][0]].len()==1{
                let j=forward[i][0];used.insert(j);
                let other=&b.value["blocks"][j];
                let reading=other["text"].as_str().unwrap();
                let status=if reading==text{"exact_text_agreement"}
                    else if whitespace_only(reading)==whitespace_only(text){"whitespace_only_difference"}
                    else{reasons.push("independent_reading_difference");"different_text"};
                (json!({"block_index":j,"text":reading,"original_pixel_quad":other["original_pixel_quad"],"text_score":other["text_score"]}),status)
            }else{reasons.push("unmatched_or_ambiguous_reading");(Value::Null,"ambiguous_or_unmatched")}
        }else{(Value::Null,"no_comparison_supplied")};
        *counts.entry(comparison.to_owned()).or_default()+=1;
        let urgent=!reasons.is_empty();
        tasks.push(json!({"id":format!("ocr-block-{i:06}"),"kind":"text_reading","priority":if urgent{"high"}else{"normal"},"status":"pending",
            "primary_block_index":i,"primary_reading":text,"source_pixel_quad":row["original_pixel_quad"],
            "primary_score":row["text_score"],"comparison":comparison,"other_reading":other,
            "candidate_other_block_indices":forward[i],"reasons":reasons,"automatic_pass":false}));
    }
    if let Some(b)=b{
        for (j,row) in b.value["blocks"].as_array().unwrap().iter().enumerate(){
            if used.contains(&j){continue;}
            tasks.push(json!({"id":format!("ocr-other-{j:06}"),"kind":"unmatched_other_reading","priority":"high",
                "status":"pending","other_block_index":j,"other_reading":row["text"],"source_pixel_quad":row["original_pixel_quad"],
                "candidate_primary_block_indices":reverse[j],"reasons":["potential_omission_or_segmentation_difference"],"automatic_pass":false}));
        }
    }
    // Layout candidates are separate even if no ordinary OCR block intersects
    // the region. Scientific tables/formulae cannot be approved by word OCR.
    for (i,r) in a.value["layout_regions"].as_array().unwrap().iter().enumerate(){
        let class=r["class"].as_str().unwrap_or("");
        if !matches!(class,"Equation"|"Table"|"Figure"){continue;}
        tasks.push(json!({"id":format!("ocr-layout-{i:06}"),"kind":"specialized_region","priority":"high","status":"pending",
            "layout_index":i,"class":class,"source_pixel_quad":r["original_pixel_quad"],
            "reasons":["requires_specialized_or_independent_visual_review"],"automatic_pass":false}));
    }
    if primary.is_empty(){
        tasks.push(json!({"id":"ocr-empty-page","kind":"coverage","priority":"high","status":"pending",
            "reasons":["no_text_detected_does_not_prove_blank_page"],"automatic_pass":false}));
    }
    let identity=engine_identity(&a.value);
    let other_identity=b.map(|e|engine_identity(&e.value));
    let same_models=other_identity.as_ref().map(|v|v["detector"]==identity["detector"]&&v["recognizer"]==identity["recognizer"]&&v["dictionary"]==identity["dictionary"]);
    Ok(json!({"schema_version":1,"primary_record":a.binding,"comparison_record":b.map(|e|&e.binding),
        "source_image":a.image,"source_image_size":a.size,"primary_engine":identity,"comparison_engine":other_identity,
        "same_recognition_assets":same_models,"independence_established":false,"primary_block_count":primary.len(),
        "comparison_block_count":b.map(|e|e.boxes.len()),"comparison_counts":counts,"tasks":tasks,
        "status":"review_required","source_fidelity":"not_assessed","acceptance_ready":false,"evidence_boundary":BOUNDARY}))
}

/// The caller independently identifies both records by path and hash. No
/// external model is invoked, no comparison is silently selected, and no
/// machine-generated review can establish source fidelity.
pub fn audit(primary:&Path,primary_sha:&str,comparison:Option<(&Path,&str)>)->Result<Value>{
    let a=Evidence::open(primary,primary_sha)?;
    let b=match comparison{Some((p,h))=>Some(Evidence::open(p,h)?),None=>None};
    let result=triage(&a,b.as_ref())?;
    a.verify()?;if let Some(b)=&b{b.verify()?;}Ok(result)
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn preserve_numbers_greek_signs_and_superscripts(){
        for t in ["10 m","−1","Ω","u²","a=b","1.2×10⁻³"]{assert!(scientific(t),"{t}");}
        assert!(!scientific("Ocean waves"));assert_ne!(whitespace_only("−1"),whitespace_only("-1"));
    }
    #[test]
    fn reciprocal_matching_rejects_ambiguous_segmentation(){
        let a=[Some([0.,0.,20.,10.]),Some([10.,0.,30.,10.]),None];
        let b=[Some([0.,0.,20.,10.]),Some([5.,0.,25.,10.])];
        let (f,r)=candidates(&a,&b);
        assert_eq!(f[0],vec![0,1]);assert_eq!(f[1],vec![1]);assert_eq!(r[1],vec![0,1]);assert!(f[2].is_empty());
    }
}
