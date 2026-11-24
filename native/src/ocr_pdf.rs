//! Page-scoped OCR routing and resumable, independent evidence for actual
//! scanned and hybrid PDFs. The existing legacy inventory contract is not
//! declared complete merely because OCR produced a hypothesis.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::BTreeMap,fs,io::Write,path::{Path,PathBuf}};
use crate::{check_ledger,check_write,files,ocr_backend,pdfcraft_backend::Backend,path,path_text,parse_json,pipeline};

const SCHEMA:u64=1;
const BOUNDARY:&str="Page routing and independently bound native/OCR hypotheses only. No inferred completeness, accepted translation, merged text, or source fidelity. Review scanned, hybrid, formula, table and uncertain pages against their originals.";
fn coord()->Value{json!({"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"})}
fn box4(v:&Value)->Result<[f64;4]>{
    let a=v.as_array().filter(|a|a.len()==4).context("bbox must contain four finite PDF-point numbers")?;
    let mut b=[0.;4];for (i,x) in a.iter().enumerate(){b[i]=x.as_f64().filter(|n|n.is_finite()).context("Invalid bbox number")?;}
    ensure!(b[0]<b[2]&&b[1]<b[3],"bbox must be nonempty");Ok(b)
}
fn matrix(v:&Value)->Result<[f64;6]>{
    let a=v.as_array().filter(|a|a.len()==6).context("Invalid raster transform")?;
    let mut m=[0.;6];for(i,x) in a.iter().enumerate(){m[i]=x.as_f64().filter(|x|x.is_finite()).context("Invalid raster transform number")?;}
    Ok(m)
}
fn inverse(m:[f64;6])->Result<[f64;6]>{
    let d=m[0]*m[3]-m[1]*m[2];ensure!(d.is_finite()&&d!=0.,"Singular raster transform");
    Ok([m[3]/d,-m[1]/d,-m[2]/d,m[0]/d,(m[2]*m[5]-m[3]*m[4])/d,(m[1]*m[4]-m[0]*m[5])/d])
}
fn pdf_quad(raw:&Value,m:[f64;6])->Option<(Vec<[f64;2]>,[f64;4])>{
    let a=raw.as_array().filter(|a|a.len()==4)?;
    let mut q=Vec::with_capacity(4);
    let mut b=[f64::INFINITY,f64::INFINITY,f64::NEG_INFINITY,f64::NEG_INFINITY];
    for p in a{
        let pair=p.as_array().filter(|a|a.len()==2)?;
        let (x,y)=(pair[0].as_f64()?,pair[1].as_f64()?);
        let (x,y)=(m[0]*x+m[2]*y+m[4],m[1]*x+m[3]*y+m[5]);
        if !x.is_finite()||!y.is_finite(){return None;}
        b[0]=b[0].min(x);b[1]=b[1].min(y);b[2]=b[2].max(x);b[3]=b[3].max(y);q.push([x,y]);
    }
    if b[0]>=b[2]||b[1]>=b[3]{return None;}
    Some((q,b))
}
fn add_coords(row:&mut Value,m:[f64;6]){
    let converted=pdf_quad(&row["original_pixel_quad"],m);
    if let Some((q,b))=converted{
        row["unrotated_pdf_quad"]=json!(q);row["unrotated_pdf_bbox"]=json!(b);
    }else{
        row["unrotated_pdf_quad"]=Value::Null;row["unrotated_pdf_bbox"]=Value::Null;
        row["location_valid"]=json!(false);
    }
}
pub(crate) fn bind_pdf_coordinates(ocr:&mut Value,render:&Value)->Result<()>{
    ensure!(ocr["source_image"]==render["asset"],"OCR image differs from its actual PDF raster");
    ensure!(ocr["source_image_size"]==json!([render["width"],render["height"]]),"OCR image size differs from its PDF raster");
    let m=inverse(matrix(&render["unrotated_page_to_pixel_transform"])?)?;
    for r in ocr["layout_regions"].as_array_mut().context("layout region array required")?{add_coords(r,m);}
    for r in ocr["blocks"].as_array_mut().context("OCR block array required")?{
        add_coords(r,m);
        let words=r["words"].as_array_mut().context("OCR word array required")?;
        for word in words{add_coords(word,m);}
    }
    ocr["source_pdf"]=render["source_file"].clone();
    ocr["page"]=render["page"].clone();ocr["render"]=render.clone();
    ocr["coordinate_system"]=coord();Ok(())
}

// This heuristic is intentionally conservative. A nonempty text layer is
// not proof of a digital page; a full-page raster may carry stale OCR text.
pub fn classify(native:&Value,structure:&Value)->Value{
    let text=native["plain_text"].as_str().unwrap_or("");
    let count=text.chars().filter(|c|!c.is_whitespace()&&!c.is_control()).count();
    let unknown=text.chars().filter(|c|*c=='\u{fffd}'||*c=='\0').count();
    let text_uncertain=native["status"]=="unavailable"||native["truncated"]==true||
        native["warnings"].as_array().is_some_and(|a|!a.is_empty())||unknown>0;
    let calls=structure["occurrences"].as_array();
    let structure_uncertain=structure["status"]!="structurally_indexed"||calls.is_none();
    let images=calls.map_or(0,Vec::len);
    let coverage=box4(&structure["page_bounds"]).ok().map(|page|{
        let area=(page[2]-page[0])*(page[3]-page[1]);
        calls.into_iter().flatten().filter_map(|r|box4(&r["bbox"]).ok()).map(|b|{
            let w=(b[2].min(page[2])-b[0].max(page[0])).max(0.);
            let h=(b[3].min(page[3])-b[1].max(page[1])).max(0.);
            w*h/area
        }).fold(0.,f64::max)
    });
    let (kind,ocr_whole,review)=if count==0||text_uncertain{
        ("scan_or_unreliable_text_layer",true,true)
    }else if coverage.is_some_and(|v|v>=0.85){
        ("full_page_image_with_text_layer",true,true)
    }else if images>0||structure_uncertain{
        ("hybrid_review_required",false,true)
    }else{("native_text_candidate",false,false)};
    json!({"kind":kind,"ocr_whole_page":ocr_whole,"region_review_required":review,
        "nonspace_character_count":count,"replacement_character_count":unknown,
        "native_text_uncertain":text_uncertain,"structure_uncertain":structure_uncertain,
        "image_occurrence_count":images,"largest_image_page_area_fraction":coverage,
        "classification_is_heuristic":true,"source_fidelity":"not_assessed"})
}
fn safe_root(p:&Path)->Result<PathBuf>{
    match fs::symlink_metadata(p){
        Ok(m)=>{ensure!(m.is_dir()&&!m.file_type().is_symlink(),"OCR output must be a real directory");
            #[cfg(windows)]{use std::os::windows::fs::MetadataExt;ensure!(m.file_attributes()&0x400==0,"OCR output directory is a reparse point");}},
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>fs::create_dir_all(p)?,
        Err(e)=>return Err(e.into())
    }
    files::canonical(p)
}
fn fresh(p:&Path,v:&Value)->Result<Value>{
    ensure!(!p.exists()&&!p.is_symlink(),"Keep existing OCR evidence: {}",p.display());
    let mut bytes=serde_json::to_vec_pretty(v)?;bytes.push(b'\n');
    let mut tmp=tempfile::NamedTempFile::new_in(p.parent().context("evidence parent")?)?;
    tmp.write_all(&bytes)?;tmp.as_file().sync_all()?;
    tmp.persist_noclobber(p).map_err(|e|anyhow::anyhow!("Cannot publish OCR evidence {}: {}",p.display(),e.error))?;
    let b=check_ledger::bind_file(p)?;
    ensure!(b["sha256"]==files::digest(&bytes),"OCR evidence changed after publication");
    Ok(b)
}
fn read_bound_json(p:&Path)->Result<(Value,Value)>{
    check_write::no_link(p)?;
    ensure!(fs::metadata(p)?.len()<=128*1024*1024,"OCR evidence JSON too large");
    let (raw,b)=files::read_bound(p)?;
    let v=parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(check_ledger::bind_file(p)?==b,"OCR evidence changed during read");Ok((v,b))
}
fn recheck_record(record:&Value,root:&Path,job:&Value,page:u64)->Result<()>{
    ensure!(record["schema_version"].as_u64()==Some(SCHEMA)&&record["job"]==*job&&record["page"].as_u64()==Some(page),
        "Existing OCR page record belongs to another job");
    ensure!(matches!(record["status"].as_str(),Some("native_text_candidate"|"hybrid_review_required"|"ocr_candidate_review_required")),
        "Invalid completed OCR page status");
    ensure!(record["acceptance_ready"]==false&&record["source_fidelity"]=="not_assessed","Do not promote OCR evidence to acceptance");
    let files=record["files"].as_array().filter(|a|a.len()>=2).context("Page must retain bound text and object evidence")?;
    for b in files{
        let p=path(&b["path"])?;check_write::no_link(&p)?;
        let p=files::canonical(&p)?;
        ensure!(p.starts_with(root)&&check_ledger::bind_file(&p)?==*b,"Saved OCR page evidence changed or escapes this job");
    }
    for key in ["native_text","image_structure"]{
        ensure!(files.contains(&record[key]),"Completed page lost mandatory evidence");
    }
    let regions=record["ocr_regions"].as_array().context("Retain OCR regions")?;
    for r in regions{
        for key in ["render","recognition"]{ensure!(files.contains(&r[key]),"Completed page lost region evidence");}
    }
    if record["status"]=="ocr_candidate_review_required"{ensure!(!regions.is_empty(),"Completed OCR page lacks actual recognition evidence");}
    if record["status"]=="hybrid_review_required"{ensure!(files.contains(&record["review_render"]),"Hybrid page lost its review raster");}
    Ok(())
}
pub struct Options<'a>{
    pub package_manifest:Option<&'a Path>,pub package_sha:Option<&'a str>,
    pub profile:&'a str,pub layout:&'a str,pub mode:&'a str,pub dpi:u64,
    pub expected_source_sha:Option<&'a str>,pub regions:&'a Value,pub coordinate_system:&'a Value,
}
fn regions(v:&Value,coord_system:&Value,pages:usize)->Result<BTreeMap<u64,Vec<[f64;4]>>>{
    let mut out=BTreeMap::new();
    let values=v.as_object().context("regions must map physical page numbers to arrays of bboxes")?;
    if !values.is_empty(){ensure!(*coord_system==coord(),"Declare unrotated PDF-point coordinates for OCR regions");}
    for (key,list) in values{
        let page=key.parse::<u64>()?;
        ensure!(page>=1&&page<=pages as u64&&key==&page.to_string(),"Unknown or ambiguous physical page number");
        let list=list.as_array().filter(|a|!a.is_empty()&&a.len()<=128).context("Provide 1 to 128 explicit OCR regions per selected page")?;
        let mut boxes=Vec::with_capacity(list.len());
        for v in list{let b=box4(v)?;ensure!(!boxes.contains(&b),"Duplicate OCR region");boxes.push(b);}
        out.insert(page,boxes);
    }Ok(out)
}

struct Lazy<'a>{
    options:&'a Options<'a>,engine:Option<ocr_backend::Engine>,init_error:Option<String>,
}
impl Lazy<'_>{
    fn get(&mut self)->Result<&mut ocr_backend::Engine>{
        if let Some(e)=self.init_error.as_ref(){anyhow::bail!("{e}");}
        if self.engine.is_none(){
            let result=(||->Result<ocr_backend::Engine>{
                let package=self.options.package_manifest.context("OCR_RESOURCES_REQUIRED: install the verified CPU OCR package")?;
                let hash=self.options.package_sha.context("OCR package hash is required")?;
                let p=ocr_backend::Package::load(package,hash,self.options.profile,self.options.layout)?;
                ocr_backend::Engine::open(p)
            })();
            match result{
                Ok(e)=>self.engine=Some(e),
                Err(e)=>{let message=format!("{e:#}");self.init_error=Some(message.clone());anyhow::bail!("{message}");}
            }
        }
        self.engine.as_mut().context("OCR engine unavailable")
    }
}

pub fn prepare(source:&Path,output:&Path,opt:Options<'_>,mut progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    ensure!(matches!(opt.mode,"auto"|"all"),"mode must be auto or all");
    ensure!(matches!(opt.profile,"auto"|"tiny"|"small"|"medium"),"invalid OCR profile");
    ensure!(matches!(opt.layout,"auto"|"on"|"off"),"invalid layout policy");
    ensure!((72..=600).contains(&opt.dpi),"OCR dpi must be between 72 and 600");
    ensure!(opt.package_manifest.is_some()==opt.package_sha.is_some(),"Supply both the verified package manifest and its trusted SHA-256, or neither");
    let mut pdf=Backend::open(source,opt.expected_source_sha)?;
    let selected=regions(opt.regions,opt.coordinate_system,pdf.len())?;
    let package=if let (Some(p),Some(hash))=(opt.package_manifest,opt.package_sha){
        check_write::no_link(p)?;
        let b=check_ledger::bind_file(p)?;
        ensure!(b["sha256"]==hash,"OCR_PACKAGE_MANIFEST_HASH_MISMATCH");b
    }else{Value::Null};
    let root=safe_root(output)?;
    let job_path=root.join("job.json");
    let _lock=check_write::lock_target(&job_path)?;
    let specification=json!({"schema_version":SCHEMA,"source_file":pdf.source_binding(),"page_count":pdf.len(),
        "package_manifest":package,"profile":opt.profile,"layout":opt.layout,"mode":opt.mode,"dpi":opt.dpi,
        "regions":opt.regions,"coordinate_system":coord(),"pdf_backend_revision":crate::pdfcraft_backend::PDFCRAFT_REV,
        "ocr_backend_revision":ocr_backend::OCR_REV,"source_fidelity":"not_assessed","boundary":BOUNDARY});
    let job=if job_path.exists()||job_path.is_symlink(){
        let (old,b)=read_bound_json(&job_path)?;
        ensure!(old==specification,"OCR job input/configuration changed; preserve this directory and start a new job");b
    }else{fresh(&job_path,&specification)?};
    let pages=pdf.len();
    let mut lazy=Lazy{options:&opt,engine:None,init_error:None};
    let mut results=Vec::with_capacity(pages);
    for n in 1..=pages{
        let page=n as u64;
        let done=root.join(format!("page-{n:06}.json"));
        if done.exists()||done.is_symlink(){
            let (record,b)=read_bound_json(&done)?;
            recheck_record(&record,&root,&job,page)?;
            results.push(json!({"page":page,"record":b,"status":record["status"],"resumed":true}));
            progress(n,pages,"reuse hash-verified OCR page evidence");continue;
        }
        // A failed attempt never replaces a previous attempt. A later call
        // retries the incomplete page after the resource or input issue is fixed.
        let attempt=tempfile::Builder::new().prefix(&format!("page-{n:06}-attempt-")).tempdir_in(&root)?.keep();
        let result=(||->Result<Value>{
            let native=pdf.text(page).unwrap_or_else(|e|json!({"status":"unavailable","error":format!("{e:#}"),"plain_text":""}));
            let structure=pdf.structure(page).unwrap_or_else(|e|json!({"status":"unavailable","error":format!("{e:#}")}));
            let classification=classify(&native,&structure);
            let native_file=fresh(&attempt.join("native-text.json"),&native)?;
            let object_file=fresh(&attempt.join("image-structure.json"),&structure)?;
            let mut files=vec![native_file.clone(),object_file.clone()];
            let wanted:Vec<Option<[f64;4]>>=if let Some(boxes)=selected.get(&page){
                boxes.iter().copied().map(Some).collect()
            }else if opt.mode=="all"||classification["ocr_whole_page"]==true{vec![None]}else{vec![]};
            let mut ocr_regions=Vec::new();let mut review_render=Value::Null;
            let status=if wanted.is_empty(){
                if classification["kind"]=="hybrid_review_required"{
                    let r=pdf.render(page,None,opt.dpi,&attempt.join("review.png"))?;
                    review_render=r["asset"].clone();files.push(review_render.clone());
                    "hybrid_review_required"
                }else{"native_text_candidate"}
            }else{
                for (i,region) in wanted.into_iter().enumerate(){
                    let png=attempt.join(format!("region-{i:03}.png"));
                    let raster=pdf.render(page,region,opt.dpi,&png)?;
                    let render_file=fresh(&attempt.join(format!("region-{i:03}-render.json")),&raster)?;
                    files.push(raster["asset"].clone());files.push(render_file.clone());
                    let mut result=lazy.get()?.recognize_png(&png,raster["asset"]["sha256"].as_str())?;
                    bind_pdf_coordinates(&mut result,&raster)?;
                    let ocr_file=fresh(&attempt.join(format!("region-{i:03}-ocr.json")),&result)?;
                    files.push(ocr_file.clone());
                    ocr_regions.push(json!({"index":i,"requested_bbox":region,"render":raster["asset"],
                        "render_metadata":render_file,"recognition":ocr_file,"status":result["status"]}));
                }
                "ocr_candidate_review_required"
            };
            pdf.verify()?;
            let record=json!({"schema_version":SCHEMA,"job":job,"page":page,"status":status,
                "classification":classification,"native_text":native_file,"image_structure":object_file,
                "review_render":review_render,"ocr_regions":ocr_regions,"files":files,
                "source_fidelity":"not_assessed","acceptance_ready":false,"boundary":BOUNDARY});
            recheck_record(&record,&root,&job,page)?;
            let b=fresh(&done,&record)?;
            Ok(json!({"page":page,"record":b,"status":status,"resumed":false}))
        })();
        match result{
            Ok(v)=>results.push(v),
            Err(e)=>{
                // Preserve any source render or OCR bytes already created.
                // A blocked page is never a completed page.
                lazy.engine=None;
                let v=json!({"schema_version":SCHEMA,"job":job,"page":page,"status":"blocked","reason":format!("{e:#}"),
                    "attempt_directory":path_text(&attempt)?,"source_fidelity":"not_assessed","acceptance_ready":false});
                let b=fresh(&attempt.join("blocked.json"),&v)?;
                results.push(json!({"page":page,"record":b,"status":"blocked","resumed":false}));
            }
        }
        progress(n,pages,"prepare native/scan/hybrid OCR evidence");
    }
    pdf.verify()?;
    ensure!(check_ledger::bind_file(&job_path)?==job,"OCR job record changed");
    if let Some(p)=opt.package_manifest{
        ensure!(check_ledger::bind_file(p)?==package,"OCR package manifest changed during job");
    }
    let blocked=results.iter().filter(|r|r["status"]=="blocked").count();
    let report=json!({"schema_version":SCHEMA,"job":job,"source_file":pdf.source_binding(),"pages":results,
        "page_count":pages,"blocked_page_count":blocked,"processing_finished":blocked==0,
        "source_inventory_ready":false,"acceptance_ready":false,"source_fidelity":"not_assessed","boundary":BOUNDARY});
    let receipt=tempfile::Builder::new().prefix("ocr-run-").tempdir_in(&root)?.keep().join("report.json");
    fresh(&receipt,&report)?;Ok(receipt)
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn hybrid_and_invisible_full_page_ocr_never_assumed_complete(){
        let native=json!({"plain_text":"some native text","warnings":[],"truncated":false});
        let empty=json!({"plain_text":"","warnings":[],"truncated":false});
        let base=json!({"status":"structurally_indexed","page_bounds":[0,0,100,100],"occurrences":[]});
        assert_eq!(classify(&native,&base)["kind"],"native_text_candidate");
        assert_eq!(classify(&empty,&base)["ocr_whole_page"],true);
        let mut hybrid=base.clone();hybrid["occurrences"]=json!([{"bbox":[10,10,40,50]}]);
        assert_eq!(classify(&native,&hybrid)["kind"],"hybrid_review_required");
        hybrid["occurrences"]=json!([{"bbox":[0,0,100,100]}]);
        assert_eq!(classify(&native,&hybrid)["kind"],"full_page_image_with_text_layer");
        hybrid["status"]=json!("partial");hybrid["occurrences"]=json!([]);
        assert_eq!(classify(&native,&hybrid)["kind"],"hybrid_review_required");
    }
    #[test]
    fn ocr_polygon_maps_back_from_rotated_page_pixels(){
        let mut ocr=json!({"source_image":{"path":"source.png","sha256":"test"},"source_image_size":[200,400],
            "layout_regions":[],"blocks":[{"original_pixel_quad":[[80,20],[100,20],[100,60],[80,60]],"words":[]} ]});
        let render=json!({"asset":ocr["source_image"],"width":200,"height":400,
            "unrotated_page_to_pixel_transform":[0,2,-2,0,200,0],"source_file":{"path":"source.pdf","sha256":"test"},"page":1});
        bind_pdf_coordinates(&mut ocr,&render).unwrap();
        assert_eq!(ocr["blocks"][0]["unrotated_pdf_bbox"],json!([10.0,50.0,30.0,60.0]));
    }
}
