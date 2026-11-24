//! Mixed-source inventory bridge for the pinned PdfCraft runtime.
//! Native text and OCR hypotheses are independent. No legacy PyMuPDF
//! block identities, semantic grouping, completeness or acceptance are inferred.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,BTreeSet},fs,io::Write,path::{Path,PathBuf}};
use crate::{check_ledger,check_write,files,ocr_backend,ocr_pdf,path,pdfcraft_backend::{Backend,PDFCRAFT_REV},pipeline};

pub const BACKEND:&str="pdfcraft-native-v1";
const BOUNDARY:&str="Actual PDF page rasters, native glyph/line records and invoked indirect image streams. Native line IDs are not PyMuPDF block IDs. OCR text is a separate hypothesis; page order, additional content, figure membership and source fidelity remain subject to independent review.";
const MAX_STREAM:usize=128*1024*1024;
const MAX_JSON:u64=128*1024*1024;
type Box4=[f64;4];
#[derive(Clone)]
pub struct OcrSettings{pub manifest:PathBuf,pub sha256:String,pub profile:String,pub layout:String,pub accelerator:Option<crate::ocr_acceleration::Settings>}
impl OcrSettings{
    pub fn from_value(v:&Value)->Result<Option<Self>>{
        if v.is_null(){return Ok(None);}
        let m=v.as_object().context("ocr must be an explicit configuration object")?;
        ensure!(m.keys().all(|s|matches!(s.as_str(),"package_manifest"|"expected_package_sha256"|"profile"|"layout"|"accelerator")),"Unknown OCR inventory configuration");
        let manifest=files::canonical(&path(&v["package_manifest"])?)?;
        let sha256=v["expected_package_sha256"].as_str().filter(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))).context("Trusted OCR package SHA-256 required")?.to_owned();
        let profile=v.get("profile").map(|v|v.as_str().context("OCR profile must be text")).transpose()?.unwrap_or("auto").to_owned();
        let layout=v.get("layout").map(|v|v.as_str().context("OCR layout must be text")).transpose()?.unwrap_or("auto").to_owned();
        ensure!(matches!(profile.as_str(),"auto"|"tiny"|"small"|"medium")&&matches!(layout.as_str(),"auto"|"on"|"off"),"Invalid OCR profile or layout");
        check_write::no_link(&manifest)?;
        ensure!(check_ledger::bind_file(&manifest)?["sha256"]==sha256,"OCR_PACKAGE_MANIFEST_HASH_MISMATCH");
        let accelerator=crate::ocr_acceleration::Settings::from_value(&v["accelerator"])?;
        Ok(Some(Self{manifest,sha256,profile,layout,accelerator}))
    }
    pub fn binding(&self)->Result<Value>{
        let b=check_ledger::bind_file(&self.manifest)?;
        ensure!(b["sha256"]==self.sha256,"OCR package manifest changed");
        if let Some(cfg)=&self.accelerator{cfg.binding()?;}
        Ok(b)
    }
}
fn box4(v:&Value)->Option<Box4>{
    let a=v.as_array().filter(|a|a.len()==4)?;
    let mut b=[0.;4];for(i,v)in a.iter().enumerate(){b[i]=v.as_f64().filter(|v|v.is_finite())?;}
    (b[0]<b[2]&&b[1]<b[3]).then_some(b)
}
fn intersect(a:Box4,b:Box4)->Option<Box4>{
    let c=[a[0].max(b[0]),a[1].max(b[1]),a[2].min(b[2]),a[3].min(b[3])];
    (c[0]<c[2]&&c[1]<c[3]).then_some(c)
}
fn union(a:Box4,b:Box4)->Box4{[a[0].min(b[0]),a[1].min(b[1]),a[2].max(b[2]),a[3].max(b[3])]}
pub(crate) fn line_blocks(id:&str,page:u64,text:&Value,bounds:Box4)->Result<(Vec<Value>,Vec<Value>)>{
    let glyphs=text["glyphs"].as_array().context("Missing native glyph rows")?;
    let mut lines=BTreeMap::<u64,Vec<&Value>>::new();
    for(i,g)in glyphs.iter().enumerate(){
        ensure!(g["glyph_index"].as_u64()==Some(i as u64),"Invalid glyph identity");
        lines.entry(g["line_index"].as_u64().context("Missing native line identity")?).or_default().push(g);
    }
    let mut blocks=Vec::new();let mut unlocated=Vec::new();
    for (line,rows) in lines{
        let mut text=String::new();let mut b=None;let mut missing=Vec::new();
        let mut indices=Vec::with_capacity(rows.len());
        for g in rows{
            let reading=g["text"].as_str().context("Missing glyph reading")?;
            if g["space_before"]==true&&!text.is_empty(){text.push(' ');}
            text.push_str(reading);
            indices.push(g["glyph_index"].clone());
            if let Some(c)=box4(&g["bbox"]).and_then(|b|intersect(bounds,b)){b=Some(b.map_or(c,|a|union(a,c)));}
            else if !reading.trim().is_empty(){missing.push(g["glyph_index"].clone());}
        }
        if text.trim().is_empty(){continue;}
        let identity=format!("{id}-p{page}-pc-line-{line}");
        let row=json!({"id":identity,"text":text,"bbox":b,"basis":"pdfcraft_native_line",
            "native_line_index":line,"glyph_indices":indices,"unlocated_glyph_indices":missing});
        if b.is_some(){blocks.push(row);}else{unlocated.push(row);}
    }
    Ok((blocks,unlocated))
}
fn publish(p:&Path,bytes:&[u8])->Result<Value>{
    ensure!(!p.exists()&&!p.is_symlink(),"Keep existing source evidence: {}",p.display());
    let mut t=tempfile::NamedTempFile::new_in(p.parent().context("Evidence parent")?)?;
    t.write_all(bytes)?;t.as_file().sync_all()?;
    t.persist_noclobber(p).map_err(|e|anyhow::anyhow!("Cannot publish {}: {}",p.display(),e.error))?;
    let b=check_ledger::bind_file(p)?;
    ensure!(b["sha256"]==files::digest(bytes),"Evidence changed during publication");Ok(b)
}
fn write_json(p:&Path,v:&Value)->Result<Value>{let mut bytes=serde_json::to_vec_pretty(v)?;bytes.push(b'\n');publish(p,&bytes)}
fn validate_binding(v:&Value)->Result<PathBuf>{
    let p=path(&v["path"])?;check_write::no_link(&p)?;
    ensure!(check_ledger::bind_file(&p)?["sha256"]==v["sha256"],"Changed PDF inventory evidence: {}",p.display());
    Ok(p)
}
fn read_json(v:&Value)->Result<Value>{
    let p=validate_binding(v)?;
    ensure!(fs::metadata(&p)?.len()<=MAX_JSON,"PDF inventory JSON exceeds size budget");
    let (raw,b)=files::read_bound(&p)?;
    ensure!(b["sha256"]==v["sha256"]&&raw.len() as u64<=MAX_JSON,"Changed PDF evidence JSON");
    crate::parse_json(std::str::from_utf8(&raw)?)
}
/// The queue rechecks the selected page evidence before presenting/recording
/// its source review, rather than hashing all page rasters at every operation.
pub(crate) fn verify_page_files(page:&Value)->Result<()>{
    if page["pdf_evidence"].is_null(){return Ok(());}
    validate_binding(&page["render"])?;validate_binding(&page["text"])?;
    let ev=&page["pdf_evidence"];
    for key in ["glyphs","image_structure"]{validate_binding(&ev[key])?;}
    if let Some(record)=ev["ocr"].get("record"){validate_binding(record)?;}
    Ok(())
}
struct LazyOcr<'a>{options:Option<&'a OcrSettings>,engine:Option<ocr_backend::Engine>,error:Option<String>,accelerated:crate::ocr_acceleration::JobState}
impl LazyOcr<'_>{
    fn run(&mut self,png:&Path,render:&Value)->Result<Value>{
        if let Some(o)=self.options{
            if let Some(cfg)=&o.accelerator{
                let mut result=self.accelerated.run(png,render["asset"]["sha256"].as_str().context("Raster digest required")?,
                    &o.manifest,&o.sha256,&o.profile,&o.layout,cfg)?;
                ocr_pdf::bind_pdf_coordinates(&mut result,render)?;return Ok(result);
            }
        }
        if let Some(e)=self.error.as_ref(){anyhow::bail!("{e}");}
        if self.engine.is_none(){
            let result=(||->Result<_>{
                let o=self.options.context("OCR_RESOURCES_REQUIRED: no trusted CPU model package supplied")?;
                o.binding()?;
                let p=ocr_backend::Package::load(&o.manifest,&o.sha256,&o.profile,&o.layout)?;
                ocr_backend::Engine::open(p)
            })();
            match result{Ok(e)=>self.engine=Some(e),Err(e)=>{self.error=Some(format!("{e:#}"));return Err(e);}}
        }
        let mut result=self.engine.as_mut().context("Missing OCR engine")?.recognize_png(png,render["asset"]["sha256"].as_str())?;
        ocr_pdf::bind_pdf_coordinates(&mut result,render)?;Ok(result)
    }
}
pub fn prepare_source(source:&Path,id:&str,root:&Path,dpi:u64,ocr:Option<&OcrSettings>,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    ensure!((72..=600).contains(&dpi),"dpi must be between 72 and 600");
    ensure!(id.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)&&id.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-')),"Invalid source ID");
    let mut backend=Backend::open(source,None)?;
    let before=backend.source_binding().clone();
    let root=files::canonical(root)?;ensure!(root.is_dir(),"Source evidence directory required");
    let image_dir=root.join("image-objects");fs::create_dir(&image_dir)?;
    let mut pages=Vec::with_capacity(backend.len());
    let mut objects=BTreeMap::<(u32,u16),Value>::new();
    let mut lazy=LazyOcr{options:ocr,engine:None,error:None,accelerated:Default::default()};
    for n in 1..=backend.len(){
        let page=n as u64;let prefix=format!("page-{n:04}");
        let png=root.join(format!("{prefix}.png"));
        let render=backend.render(page,None,dpi,&png)?;
        let (_,_,bounds,_)=backend.geometry(page)?;
        let native=backend.text(page).unwrap_or_else(|e|json!({"schema_version":1,"status":"unavailable","error":format!("{e:#}"),
            "source_file":before,"page":page,"plain_text":"","glyphs":[],"warnings":[],"truncated":true}));
        let structure=backend.structure(page)?;
        let (blocks,unlocated)=line_blocks(id,page,&native,bounds)?;
        let text=publish(&root.join(format!("{prefix}.txt")),native["plain_text"].as_str().context("Missing native text")?.as_bytes())?;
        let glyphs=write_json(&root.join(format!("{prefix}-glyphs.json")),&native)?;
        let index_file=write_json(&root.join(format!("{prefix}-objects.json")),&structure)?;
        let classification=ocr_pdf::classify(&native,&structure);
        let ocr=if classification["ocr_whole_page"]==true{
            if lazy.options.is_none(){
                json!({"status":"ocr_resources_required","review_required":true})
            }else{
                match lazy.run(&png,&render){
                    Ok(result)=>{let record=write_json(&root.join(format!("{prefix}-ocr.json")),&result)?;
                        json!({"status":result["status"],"record":record,"review_required":true})},
                    Err(e)=>{lazy.engine=None;json!({"status":"blocked","reason":format!("{e:#}"),"review_required":true})},
                }
            }
        }else if classification["region_review_required"]==true{json!({"status":"region_review_required","review_required":true})}
        else{json!({"status":"native_text_candidate","review_required":true})};
        for indexed in structure["objects"].as_array().context("Missing image object index")?{
            let xref=u32::try_from(indexed["image_xref"].as_u64().context("Image XRef required")?)?;
            let generation=u16::try_from(indexed["generation"].as_u64().context("Image generation required")?)?;
            if objects.contains_key(&(xref,generation)){continue;}
            let mut row=indexed.clone();
            let bytes=backend.image_stream(xref,generation)?;
            ensure!(files::digest(bytes)==indexed["encoded_stream_sha256"],"Actual PDF stream differs from index");
            let stem=format!("xref-{xref:06}-g{generation}");
            if bytes.len()<=MAX_STREAM{
                row["encoded_stream"]=publish(&image_dir.join(format!("{stem}.bin")),bytes)?;
            }else{row["encoded_stream_status"]=json!("blocked_budget");}
            // Raw stream identity is retained even if a display-dependent
            // image cannot safely be exported as a standalone figure asset.
            match backend.export_indexed(page,xref,&image_dir.join(&stem),&structure){
                Ok(record)=>{row["raw_asset"]=record["asset"].clone();row["extraction"]=record;},
                Err(e)=>{row["extraction"]=json!({"status":"page_render_required","reason":format!("{e:#}")});}
            }
            row["first_invoked_page"]=json!(page);
            objects.insert((xref,generation),row);
        }
        let mut render_record=render["asset"].clone();
        for key in ["dpi","width","height","pixel_sha256","pixel_format"]{render_record[key]=render[key].clone();}
        pages.push(json!({"page":page,"render":render_record,"text":text,"blocks":blocks,"image_index":structure,
            "pdf_evidence":{"backend":BACKEND,"backend_revision":PDFCRAFT_REV,"glyphs":glyphs,"image_structure":index_file,
                "classification":classification,"ocr":ocr,"unlocated_native_lines":unlocated,"source_fidelity":"not_assessed",
                "reading_order_review":"pending","additional_object_review":"pending","render_metadata":render}}));
        progress(n,backend.len(),"native PDF inventory pages");
    }
    backend.verify()?;if let Some(o)=lazy.options{o.binding()?;}
    Ok(json!({"format":"pdf","inventory_backend":BACKEND,"backend_revision":PDFCRAFT_REV,"page_count":backend.len(),
        "pages":pages,"image_objects":objects.into_values().collect::<Vec<_>>(),"source_fidelity":"not_assessed",
        "source_inventory_ready":false,"evidence_boundary":BOUNDARY}))
}
/// Independently replay source parsing/rasterization and compare actual
/// source-derived records. This deliberately does not rerun OCR or establish
/// visual fidelity against a separate engine; full delivery stays blocked.
pub fn replay(source:&Value,p:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    ensure!(source["inventory_backend"]==BACKEND&&source["backend_revision"]==PDFCRAFT_REV,"Unknown native PDF inventory backend");
    let id=source["id"].as_str().context("Source ID required")?;
    let sha=source["sha256"].as_str().context("Source SHA-256 required")?;
    let mut backend=Backend::open(p,Some(sha))?;
    let pages=source["pages"].as_array().context("Retain all PDF pages")?;
    ensure!(pages.len()==backend.len()&&source["page_count"].as_u64()==Some(backend.len() as u64),"PDF page count changed");
    let mut actual_objects=BTreeMap::<(u32,u16),(u64,Value)>::new();
    let mut checked=0usize;let mut blocked=Vec::<Value>::new();
    for(i,row)in pages.iter().enumerate(){
        let page=(i+1) as u64;ensure!(row["page"].as_u64()==Some(page),"Page ordering changed");
        verify_page_files(row)?;
        let ev=&row["pdf_evidence"];
        ensure!(ev["backend"]==BACKEND&&ev["backend_revision"]==PDFCRAFT_REV,"Unexpected native PDF page backend");
        let glyphs=read_json(&ev["glyphs"])?;
        let text=backend.text(page);
        match text{
            Ok(actual)=>{
                ensure!(actual==glyphs,"PDF page {page} native glyphs differ from actual re-extraction");
                let (_,_,bounds,_)=backend.geometry(page)?;
                let (blocks,unlocated)=line_blocks(id,page,&actual,bounds)?;
                ensure!(row["blocks"]==json!(blocks)&&ev["unlocated_native_lines"]==json!(unlocated),"PDF page {page} native line inventory differs");
                ensure!(row["text"]["sha256"]==files::digest(actual["plain_text"].as_str().context("Actual PDF text required")?.as_bytes()),"PDF page {page} text differs");
            }
            Err(e)=>{
                ensure!(glyphs["status"]=="unavailable"&&row["blocks"]==json!([]),"PDF page {page} extraction is no longer reproducible");
                blocked.push(json!({"page":page,"reason":format!("Native text unavailable: {e:#}")}));
            }
        }
        let structure=backend.structure(page)?;
        ensure!(structure==row["image_index"]&&structure==read_json(&ev["image_structure"])?,"PDF page {page} image occurrence index differs");
        ensure!(ocr_pdf::classify(&glyphs,&structure)==ev["classification"],"PDF page {page} classification differs");
        let dpi=row["render"]["dpi"].as_u64().filter(|d|(72..=600).contains(d)).context("Valid recorded PDF DPI required")?;
        let (png,actual)=backend.raster(page,None,dpi)?;
        for key in ["width","height","dpi","pixel_sha256","pixel_format"]{
            ensure!(actual[key]==row["render"][key],"PDF page {page} rendered geometry or pixels differ: {key}");
        }
        // Require exact same pinned renderer output bytes as well as a
        // canonical pixel digest. Different engine builds are not recaptured.
        ensure!(row["render"]["sha256"]==files::digest(&png),"PDF page {page} PNG bytes differ from re-render");
        let mut expected=actual;expected["asset"]=json!({"path":row["render"]["path"],"sha256":row["render"]["sha256"]});
        ensure!(ev["render_metadata"]==expected,"PDF page {page} render provenance differs");
        if let Some(b)=ev["ocr"].get("record"){
            let ocr=read_json(b)?;
            ensure!(ocr["source_pdf"]==*backend.source_binding()&&ocr["page"].as_u64()==Some(page)&&
                ocr["source_image"]==expected["asset"]&&ocr["render"]==expected&&ocr["acceptance_ready"]==false,
                "PDF page {page} OCR candidate no longer binds the actual page");
            // OCR remains a pending hypothesis; verify its resource bytes, not
            // claim recognition accuracy or silently rerun/change the model.
            validate_binding(&ocr["model_package"]["manifest"])?;
            for asset in ocr["model_package"]["resources"].as_object().context("OCR model resource bindings")?.values(){validate_binding(asset)?;}
        }
        if ev["classification"]["region_review_required"]==true||glyphs["truncated"]==true||structure["status"]!="structurally_indexed"{
            blocked.push(json!({"page":page,"reason":"Independent page, extraction or OCR review required"}));
        }
        for obj in structure["objects"].as_array().context("Missing actual image objects")?{
            let x=u32::try_from(obj["image_xref"].as_u64().context("XRef")?)?;
            let g=u16::try_from(obj["generation"].as_u64().context("Generation")?)?;
            actual_objects.entry((x,g)).or_insert_with(||(page,obj.clone()));
        }
        checked+=1;progress(i+1,pages.len(),"independent PdfCraft source replay");
    }
    let records=source["image_objects"].as_array().context("Retain actual image objects")?;
    ensure!(records.len()==actual_objects.len(),"PDF image object inventory count differs");
    let mut seen=BTreeSet::new();
    let scratch=tempfile::tempdir()?;
    for row in records{
        let x=u32::try_from(row["image_xref"].as_u64().context("XRef")?)?;
        let g=u16::try_from(row["generation"].as_u64().context("Generation")?)?;
        ensure!(seen.insert((x,g)),"Duplicate image identity");
        let (page,actual)=actual_objects.get(&(x,g)).context("Uninvoked image object")?;
        ensure!(actual.as_object().unwrap().iter().all(|(k,v)|row[k]==*v)&&row["first_invoked_page"].as_u64()==Some(*page),"Image object metadata differs");
        let bytes=backend.image_stream(x,g)?;
        if bytes.len()<=MAX_STREAM{
            validate_binding(&row["encoded_stream"])?;
            ensure!(row["encoded_stream"]["sha256"]==files::digest(bytes),"Raw PDF stream bytes differ");
        }else{ensure!(row["encoded_stream_status"]=="blocked_budget","Missing raw stream budget state");}
        // Re-run the direct extraction under the same actual occurrence
        // contract. It may legitimately require rendering rather than a raw asset.
        let index=&pages[(*page-1) as usize]["image_index"];
        let r=backend.export_indexed(*page,x,&scratch.path().join(format!("xref-{x}-g{g}")),index);
        match r{
            Ok(actual)=>{
                validate_binding(&row["raw_asset"])?;
                ensure!(row["raw_asset"]["sha256"]==actual["asset"]["sha256"]&&row["extraction"]["source_object"]==actual["source_object"]&&
                    row["extraction"]["asset"]==row["raw_asset"],"Image extraction differs from actual samples");
            }
            Err(_)=>{ensure!(row["raw_asset"].is_null()&&row["extraction"]["status"]=="page_render_required","Unexpected standalone extraction for display-dependent image");}
        }
    }
    backend.verify()?;
    Ok(json!({"backend":BACKEND,"source_file":backend.source_binding(),"page_count":checked,
        "image_object_count":seen.len(),"same_backend_replay_complete":true,"pending_review":blocked,
        "independent_visual_parity":"not_assessed","ocr_accuracy":"not_assessed","source_fidelity":"not_assessed",
        "source_checks_complete":false,"delivery_ready":false,"boundary":BOUNDARY}))
}
