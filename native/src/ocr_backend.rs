//! CPU-first OCR, using the pinned PP-OCRv6 implementation and a trusted,
//! explicitly hash-bound ONNX Runtime package. No implicit network access,
//! GPU initialisation, Python process, or acceptance decisions.
use anyhow::{anyhow,bail,ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::{collections::HashSet,fs,io::Cursor,path::{Path,PathBuf},sync::Mutex,panic::{catch_unwind,AssertUnwindSafe}};
use ppocr_rs::{OcrLite,OcrOptions,LayoutAnalyzer,DocOrientationClassifier,Point,TextBlock,TableStructureRecognizer,CellDetector};
use crate::{check_ledger,check_write,files,path,path_text,parse_json};

pub const OCR_REV:&str="b87550b775206655acf84d7fc421ca5bdf745fd3";
const GIB:u64=1024*1024*1024;
const MAX_PIXELS:u64=20_000_000;
const MAX_PNG_BYTES:u64=128*1024*1024;
const MAX_MANIFEST_BYTES:u64=2*1024*1024;
const BOUNDARY:&str="OCR hypotheses only. Confidence is an engine score, not a calibrated correctness probability. Reading order, missing content, numbers, symbols, formulas and tables require independent comparison with the bound original. Never overwrite source text or grant acceptance.";

fn str_nonempty<'a>(v:&'a Value,label:&str)->Result<&'a str>{
    v.as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("{label} must be nonempty text"))
}
fn sha(v:&Value)->Result<&str>{
    let s=v.as_str().context("Actual SHA-256 required")?;
    ensure!(s.len()==64&&s.bytes().all(|b|b.is_ascii_hexdigit()&&!b.is_ascii_uppercase()),"Expected lowercase SHA-256");
    Ok(s)
}
fn binding(p:&Path)->Result<Value>{check_ledger::bind_file(p)}
fn read_small(p:&Path,limit:u64)->Result<(Vec<u8>,Value)>{
    check_write::no_link(p)?;
    ensure!(fs::metadata(p)?.len()<=limit,"Input exceeds evidence size budget: {}",p.display());
    let (raw,b)=files::read_bound(p)?;
    ensure!(raw.len() as u64<=limit&&binding(p)?==b,"File changed during loading");Ok((raw,b))
}
fn available()->(u64,u64,usize){
    let mut sys=sysinfo::System::new();
    sys.refresh_memory();
    (sys.total_memory(),sys.available_memory(),std::thread::available_parallelism().map(|n|n.get()).unwrap_or(1))
}
#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
enum Tier{Tiny,Small,Medium}
impl Tier{
    fn parse(v:&str)->Result<Self>{match v{"tiny"=>Ok(Self::Tiny),"small"=>Ok(Self::Small),"medium"=>Ok(Self::Medium),_=>bail!("profile must be auto, tiny, small, or medium")}}
    fn name(self)->&'static str{match self{Self::Tiny=>"tiny",Self::Small=>"small",Self::Medium=>"medium"}}
    fn max_side(self)->u32{match self{Self::Tiny=>1280,Self::Small=>1600,Self::Medium=>1920}}
    fn threads(self)->usize{match self{Self::Tiny=>2,Self::Small=>4,Self::Medium=>8}}
}
fn ceiling(total:u64,free:u64)->Tier{
    if total>=16*GIB&&free>=6*GIB{Tier::Medium}
    else if total>=8*GIB&&free>=3*GIB{Tier::Small}
    else{Tier::Tiny}
}
fn plan(total:u64,free:u64,cores:usize,requested:&str,profiles:Option<&Map<String,Value>>,layout:&str,layout_installed:bool)->Result<Value>{
    ensure!(matches!(layout,"auto"|"on"|"off"),"layout must be auto, on or off");
    let max=ceiling(total,free);
    let installed=|t:Tier|profiles.is_none_or(|m|m.contains_key(t.name()));
    let tier=if requested=="auto"{
        [Tier::Medium,Tier::Small,Tier::Tiny].into_iter().find(|t|*t<=max&&installed(*t))
            .context("No compatible installed OCR profile; install a verified model package")?
    }else{
        let t=Tier::parse(requested)?;
        ensure!(installed(t),"Requested OCR profile is not installed");
        ensure!(t<=max,"Insufficient memory for explicitly requested OCR profile; select a smaller profile or free memory");
        t
    };
    let layout_allowed=total>=8*GIB&&free>=3*GIB;
    let use_layout=match layout{
        "on"=>{ensure!(layout_allowed&&layout_installed,"Explicit layout requested but model or memory budget is unavailable");true},
        "auto"=>layout_allowed&&layout_installed,
        _=>false,
    };
    let allowed=free>=512*1024*1024;
    Ok(json!({"policy":"cpu_first","execution_provider":"CPUExecutionProvider","gpu_probe":"not_attempted",
        "gpu_required":false,"profile":tier.name(),"recommended_ceiling":max.name(),
        "threads":cores.max(1).min(tier.threads()),"max_detection_side":tier.max_side(),
        "layout_enabled":use_layout,"available_memory_bytes":free,"total_memory_bytes":total,
        "logical_parallelism":cores,"memory_budget_available":allowed,"page_parallelism":1,
        "boundary":"Conservative scheduling policy, not measured minimum system requirements. Smaller OCR models never lower acceptance requirements."}))
}
pub fn hardware_plan(requested:&str)->Result<Value>{
    let (total,free,cores)=available();
    plan(total,free,cores,requested,None,"off",false)
}

// Package paths are POSIX relative names and must not escape the trusted root,
// traverse symlinks/reparse points, or address Windows device names.
fn local(root:&Path,v:&Value)->Result<PathBuf>{
    let raw=str_nonempty(v,"package-relative path")?;
    ensure!(!raw.contains(['\\','\0',':']),"Invalid package-relative path");
    let mut p=root.to_owned();
    for part in raw.split('/'){
        ensure!(!part.is_empty()&&part!="."&&part!="..","Traversing package path");
        if cfg!(windows){
            ensure!(!part.ends_with(['.',' '])&&!part.contains(['<','>','"','|','?','*']),"Ambiguous Windows package path");
            let stem=part.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
            ensure!(!["CON","PRN","AUX","NUL","CONIN$","CONOUT$"].contains(&stem.as_str())
                &&!stem.strip_prefix("COM").or_else(||stem.strip_prefix("LPT")).is_some_and(|n|n.chars().count()==1&&"123456789¹²³".contains(n)),"Windows device name forbidden");
        }
        p.push(part);
        let meta=fs::symlink_metadata(&p)?;
        ensure!(!meta.file_type().is_symlink(),"Refusing a linked OCR resource");
        #[cfg(windows)]{
            use std::os::windows::fs::MetadataExt;
            ensure!(meta.file_attributes()&0x400==0,"Refusing a reparse-point OCR resource");
        }
    }
    check_write::no_link(&p)?;
    let p=files::canonical(&p)?;
    ensure!(p.starts_with(root),"OCR resource escapes package root");Ok(p)
}
fn resource(root:&Path,v:&Value,label:&str)->Result<Value>{
    ensure!(v.is_object(),"{label}: missing resource binding");
    let p=local(root,&v["path"])?;
    let expected=sha(&v["sha256"])?;
    let record=binding(&p)?;
    ensure!(record["sha256"]==expected,"OCR_RESOURCE_HASH_MISMATCH: {label}");
    ensure!(fs::metadata(p)?.len()>0,"Empty OCR resource: {label}");Ok(record)
}
fn package_records(data:&Value)->Result<Vec<(&str,&Value)>>{
    ensure!(data.is_object()&&data["schema_version"].as_u64()==Some(1)&&data["engine"]=="ppocr-v6",
        "Use the verified schema-1 PP-OCRv6 package manifest");
    let runtime=&data["runtime"];
    ensure!(runtime["platform"]==std::env::consts::OS&&runtime["architecture"]==std::env::consts::ARCH,
        "OCR_RUNTIME_PLATFORM_MISMATCH");
    let version=str_nonempty(&runtime["version"],"ONNX Runtime version")?;
    ensure!(version.strip_prefix("1.28.").is_some_and(|v|!v.is_empty()&&v.bytes().all(|b|b.is_ascii_digit())),
        "Package must name a qualified ONNX Runtime 1.28.x build");
    let mut result=vec![("runtime",&runtime["file"])];
    if let Some(rows)=runtime.get("dependencies"){
        let rows=rows.as_array().filter(|r|r.len()<=64).context("runtime.dependencies must be an array of at most 64 resource records")?;
        for row in rows{result.push(("runtime_dependency",row));}
    }
    let profiles=data["profiles"].as_object().filter(|m|!m.is_empty()).context("OCR profiles must be a nonempty object")?;
    for (key,profile) in profiles{
        ensure!(matches!(key.as_str(),"tiny"|"small"|"medium")&&profile.is_object(),"Invalid OCR profile");
        for name in ["detector","recognizer","dictionary"]{result.push((name,&profile[name]));}
    }
    let aux=&data["auxiliary"];
    ensure!(aux.is_null()||aux.is_object(),"auxiliary must be an object");
    if let Some(aux)=aux.as_object(){
        for (name,v) in aux{
            match name.as_str(){
                "orientation"|"layout"=>{if !v.is_null(){result.push(("auxiliary",v));}},
                "table_structure"=>{
                    if !v.is_null(){
                        result.push(("table_model",&v["model"]));
                        result.push(("table_dictionary",&v["dictionary"]));
                    }
                },
                "table_cells_wired"|"table_cells_wireless"=>{if !v.is_null(){result.push(("table_cell_detector",v));}},
                _=>bail!("Unknown OCR auxiliary model: {name}"),
            }
        }
    }
    Ok(result)
}
fn verified_manifest(p:&Path,expected:&str)->Result<(Vec<u8>,Value,Value,PathBuf)>{
    let (raw,manifest)=read_small(p,MAX_MANIFEST_BYTES)?;
    ensure!(manifest["sha256"]==sha(&json!(expected))?,"OCR_PACKAGE_MANIFEST_HASH_MISMATCH");
    let root=path(&manifest["path"])?.parent().context("package directory")?.to_owned();
    let data=parse_json(std::str::from_utf8(&raw)?)?;
    Ok((raw,manifest,data,root))
}
fn all_package_files(data:&Value,root:&Path,manifest:&Value)->Result<std::collections::BTreeMap<String,Value>>{
    let mut result=std::collections::BTreeMap::<String,Value>::new();
    let mut canonical=std::collections::BTreeMap::<PathBuf,String>::new();
    for (label,row) in package_records(data)?{
        let raw=str_nonempty(&row["path"],label)?;
        let checked=resource(root,row,label)?;
        let p=path(&checked["path"])?;
        ensure!(checked["path"]!=manifest["path"]&&raw!="package.json",
            "Model assets must not alias the package manifest");
        if let Some(previous)=canonical.get(&p){ensure!(previous==raw,"Two different paths alias an OCR resource");}
        canonical.insert(p,raw.to_owned());
        if let Some(previous)=result.get(raw){ensure!(previous==&checked,"Conflicting OCR asset bindings");}
        result.insert(raw.to_owned(),checked);
    }
    Ok(result)
}
fn real_directory(p:&Path)->Result<()>{
    let meta=fs::symlink_metadata(p)?;
    ensure!(meta.is_dir()&&!meta.file_type().is_symlink(),"OCR store must be a real directory: {}",p.display());
    #[cfg(windows)]{
        use std::os::windows::fs::MetadataExt;
        ensure!(meta.file_attributes()&0x400==0,"Refusing a reparse-point OCR directory");
    }
    Ok(())
}
fn check_installed_contents(root:&Path,expected:&std::collections::BTreeMap<String,Value>)->Result<()>{
    let mut todo=vec![root.to_owned()];
    let mut seen=HashSet::new();
    while let Some(dir)=todo.pop(){
        real_directory(&dir)?;
        for entry in fs::read_dir(dir)?{
            let entry=entry?;let p=entry.path();let ty=entry.file_type()?;
            ensure!(!ty.is_symlink(),"Unexpected symlink in OCR package");
            if ty.is_dir(){real_directory(&p)?;todo.push(p);continue;}
            check_write::no_link(&p)?;
            let relative=p.strip_prefix(root)?.to_str().context("Non-UTF-8 package path")?.replace('\\',"/");
            if relative=="package.json"{continue;}
            let b=expected.get(&relative).context("Unexpected file in installed OCR package")?;
            let current=binding(&p)?;
            ensure!(current["sha256"]==b["sha256"],"Installed OCR resource hash mismatch: {relative}");
            seen.insert(relative);
        }
    }
    ensure!(seen.len()==expected.len(),"Installed OCR package lost a resource");Ok(())
}

/// Import only explicitly declared resources from an already trusted package.
/// Does not download, execute, unpack arbitrary archives, or load an ONNX library.
/// The package's expected hash must be obtained independently of its contents.
/// Every profile is checked, even if this machine would only select Tiny.
pub fn install_package(manifest_path:&Path,expected_sha:&str,store:&Path,
    mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let (raw,manifest,data,source_root)=verified_manifest(manifest_path,expected_sha)?;
    let assets=all_package_files(&data,&source_root,&manifest)?;
    ensure!(!assets.is_empty(),"Empty OCR package");
    match fs::symlink_metadata(store){
        Ok(_)=>real_directory(store)?,
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>fs::create_dir_all(store)?,
        Err(e)=>return Err(e.into()),
    }
    real_directory(store)?;
    let store=files::canonical(store)?;
    let digest=manifest["sha256"].as_str().context("Package SHA-256")?;
    let destination=store.join(digest);
    ensure!(source_root!=destination,"Use an external package as installation input");
    let _lock=check_write::lock_target(&destination)?;
    let reused=match fs::symlink_metadata(&destination){
        Ok(_)=>{
            real_directory(&destination)?;
            let (_,_,installed,installed_root)=verified_manifest(&destination.join("package.json"),digest)?;
            all_package_files(&installed,&installed_root,&manifest)?;
            check_installed_contents(&destination,&assets)?;
            true
        },
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{
            let staging=tempfile::Builder::new().prefix(".pending-ocr-package-").tempdir_in(&store)?;
            for (i,(relative,b)) in assets.iter().enumerate(){
                let source=path(&b["path"])?;
                let dest=staging.path().join(relative);
                fs::create_dir_all(dest.parent().context("OCR resource parent")?)?;
                let mut reader=fs::File::open(&source)?;
                let mut writer=fs::OpenOptions::new().write(true).create_new(true).open(&dest)?;
                std::io::copy(&mut reader,&mut writer)?;
                writer.sync_all()?;
                ensure!(binding(&dest)?["sha256"]==b["sha256"]&&binding(&source)?==*b,
                    "OCR resource changed during installation: {relative}");
                progress(i+1,assets.len(),relative);
            }
            let p=staging.path().join("package.json");
            {
                use std::io::Write;
                let mut output=fs::OpenOptions::new().write(true).create_new(true).open(&p)?;
                output.write_all(&raw)?;output.sync_all()?;
            }
            check_installed_contents(staging.path(),&assets)?;
            ensure!(binding(manifest_path)?==manifest,"Source package manifest changed during installation");
            ensure!(!destination.exists()&&!destination.is_symlink(),"Package destination appeared during installation");
            fs::rename(staging.path(),&destination).context("Cannot publish verified OCR package")?;
            check_installed_contents(&destination,&assets)?;
            false
        },
        Err(e)=>return Err(e.into()),
    };
    let installed=binding(&destination.join("package.json"))?;
    ensure!(installed["sha256"]==digest,"Installed manifest changed");
    Ok(json!({"schema_version":1,"package_manifest":installed,"package_root":path_text(&destination)?,
        "resource_count":assets.len(),"reused_existing":reused,"execution_provider":"CPUExecutionProvider",
        "runtime_loaded":false,"qualified":false,"boundary":"Hash-verified import only. Actual CPU inference, authenticity of the independent package hash, and distribution licensing are separate qualifications."}))
}

pub struct Package{manifest:Value,resources:Map<String,Value>,policy:Value}
impl Package{
    pub fn load(manifest_path:&Path,expected_sha:&str,requested:&str,layout:&str)->Result<Self>{
        // The manifest hash is obtained independently from the trusted
        // installation/configuration. Self-reported asset hashes alone are
        // not an authenticity guarantee.
        let (raw,manifest)=read_small(manifest_path,MAX_MANIFEST_BYTES)?;
        ensure!(manifest["sha256"]==sha(&json!(expected_sha))?,"OCR_PACKAGE_MANIFEST_HASH_MISMATCH");
        let root=path(&manifest["path"])?.parent().context("package directory")?.to_owned();
        let data=parse_json(std::str::from_utf8(&raw)?)?;
        ensure!(data.is_object()&&data["schema_version"].as_u64()==Some(1)&&data["engine"]=="ppocr-v6",
            "Use the verified schema-1 PP-OCRv6 package manifest");
        let runtime=&data["runtime"];
        ensure!(runtime["platform"]==std::env::consts::OS&&runtime["architecture"]==std::env::consts::ARCH,
            "OCR_RUNTIME_PLATFORM_MISMATCH");
        let version=str_nonempty(&runtime["version"],"ONNX Runtime version")?;
        ensure!(version.strip_prefix("1.28.").is_some_and(|v|!v.is_empty()&&v.bytes().all(|b|b.is_ascii_digit())),
            "Package must name a qualified ONNX Runtime 1.28.x build");
        let profiles=data["profiles"].as_object().context("OCR profiles must be an object")?;
        ensure!(!profiles.is_empty()&&profiles.keys().all(|s|matches!(s.as_str(),"tiny"|"small"|"medium")),"Invalid OCR profile names");
        let aux=&data["auxiliary"];
        ensure!(aux.is_null()||aux.is_object(),"auxiliary must be an object");
        let (total,free,cores)=available();
        let policy=plan(total,free,cores,requested,Some(profiles),layout,!aux["layout"].is_null())?;
        let chosen=policy["profile"].as_str().unwrap();
        let profile=&profiles[chosen];
        let mut resources=Map::new();
        resources.insert("runtime".into(),resource(&root,&runtime["file"],"runtime")?);
        let mut distinct=HashSet::new();
        distinct.insert(resources["runtime"]["path"].as_str().unwrap().to_owned());
        if let Some(rows)=runtime.get("dependencies"){
            let rows=rows.as_array().filter(|r|r.len()<=64).context("Invalid runtime dependency list")?;
            for (i,row) in rows.iter().enumerate(){
                let b=resource(&root,row,"runtime dependency")?;
                ensure!(distinct.insert(b["path"].as_str().unwrap().to_owned()),"Duplicate runtime dependency");
                resources.insert(format!("runtime_dependency_{i}"),b);
            }
        }

        for name in ["detector","recognizer","dictionary"]{
            let b=resource(&root,&profile[name],name)?;
            ensure!(distinct.insert(b["path"].as_str().unwrap().to_owned()),"OCR assets must be distinct actual files");
            resources.insert(name.into(),b);
        }
        for (name,needed) in [("orientation",!aux["orientation"].is_null()),("layout",policy["layout_enabled"]==true)]{
            if needed{
                let b=resource(&root,&aux[name],name)?;
                ensure!(distinct.insert(b["path"].as_str().unwrap().to_owned()),"OCR assets must be distinct actual files");
                resources.insert(name.into(),b);
            }
        }
        let p=Self{manifest,resources,policy};p.verify()?;Ok(p)
    }
    fn for_table(manifest_path:&Path,expected_sha:&str,profile:&str,cell_mode:&str)->Result<Self>{
        ensure!(matches!(cell_mode,"off"|"wired"|"wireless"),"cell_mode must be off, wired or wireless");
        let mut p=Self::load(manifest_path,expected_sha,profile,"off")?;
        // Explicit, potentially expensive operation. No automatic table model
        // loading while processing ordinary pages.
        ensure!(available().1>=1024*1024*1024,"Insufficient available memory for table analysis");
        let (_,manifest,data,root)=verified_manifest(manifest_path,expected_sha)?;
        ensure!(manifest==p.manifest,"Table package changed during preparation");
        let aux=&data["auxiliary"];
        let table=&aux["table_structure"];
        ensure!(table["architecture"]=="SLANet_plus","Declare the actual SLANet_plus table model");
        for (name,row) in [("table_model",&table["model"]),("table_dictionary",&table["dictionary"])]{
            let b=resource(&root,row,name)?;
            ensure!(p.resources.values().all(|v|v["path"]!=b["path"]),"Table model assets must be distinct");
            p.resources.insert(name.into(),b);
        }
        if cell_mode!="off"{
            let key=format!("table_cells_{cell_mode}");
            let b=resource(&root,&aux[&key],"RT-DETR-L cell detector")?;
            ensure!(p.resources.values().all(|v|v["path"]!=b["path"]),"Cell model assets must be distinct");
            p.resources.insert("table_cell_detector".into(),b);
        }
        // A caller-selected table crop is not a page. Do not auto-rotate it.
        p.resources.remove("orientation");p.policy["table_cell_mode"]=json!(cell_mode);
        p.verify()?;Ok(p)
    }
    fn verify(&self)->Result<()>{
        for b in std::iter::once(&self.manifest).chain(self.resources.values()){
            let p=path(&b["path"])?;check_write::no_link(&p)?;
            ensure!(binding(&p)?==*b,"OCR_RESOURCE_CHANGED: {}",p.display());
        }Ok(())
    }
    fn get(&self,name:&str)->Result<PathBuf>{
        path(&self.resources.get(name).with_context(||format!("Missing OCR resource {name}"))?["path"])
    }
    fn utf8(&self,name:&str)->Result<&str>{
        str_nonempty(&self.resources.get(name).context("Missing model")?["path"],name)
    }
    pub fn report(&self)->Value{
        json!({"manifest":self.manifest,"resources":self.resources,"hardware":self.policy,
            "backend":"ppocr-rs","backend_revision":OCR_REV,"qualified":false})
    }
}
pub fn inspect_package(manifest_path:&Path,expected_sha:&str,requested:&str,layout:&str)->Result<Value>{
    Ok(Package::load(manifest_path,expected_sha,requested,layout)?.report())
}
static RUNTIME:Mutex<Option<Value>>=Mutex::new(None);
fn init_cpu(p:&Package)->Result<()>{
    let mut runtime=RUNTIME.lock().map_err(|_|anyhow!("OCR runtime initialisation lock poisoned"))?;
    if let Some(b)=runtime.as_ref(){
        ensure!(b==&p.resources["runtime"],"Cannot switch ONNX Runtime within a process");
        return Ok(());
    }
    let dll=p.get("runtime")?;
    let builder=ort::init_from(&dll).context("OCR_RUNTIME_LOAD_FAILED (install the verified CPU runtime)")?;
    ensure!(builder.with_execution_providers([ort::ep::CPU::default().with_arena_allocator(false).build()])
        .with_telemetry(false).commit(),"ONNX Runtime already configured outside the CPU-first policy");
    *runtime=Some(p.resources["runtime"].clone());Ok(())
}

pub struct Engine{
    package:Package,
    ocr:OcrLite,
    layout:Option<LayoutAnalyzer>,
    orientation:Option<DocOrientationClassifier>,
}
fn guarded<T>(f:impl FnOnce()->Result<T>)->Result<T>{
    catch_unwind(AssertUnwindSafe(f)).map_err(|_|anyhow!("OCR_ENGINE_PANIC: upstream inference did not complete"))?
}
impl Engine{
    pub fn open(package:Package)->Result<Self>{
        guarded(||{
            ensure!(package.policy["memory_budget_available"]==true,"Insufficient available memory for OCR; free memory and retry");
            package.verify()?;
            init_cpu(&package)?;
            let threads=package.policy["threads"].as_u64().unwrap() as usize;
            let mut ocr=OcrLite::new();
            ocr.init_models_no_angle(package.utf8("detector")?,package.utf8("recognizer")?,package.utf8("dictionary")?,threads)
                .context("PP-OCRv6 model initialisation failed")?;
            let layout=if package.resources.contains_key("layout"){
                let session=ort::session::Session::builder().map_err(ppocr_rs::OcrError::from)?
                    .with_intra_threads(threads).map_err(ppocr_rs::OcrError::from)?
                    .with_inter_threads(1).map_err(ppocr_rs::OcrError::from)?
                    .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3).map_err(ppocr_rs::OcrError::from)?
                    .commit_from_file(package.get("layout")?).map_err(ppocr_rs::OcrError::from)?;
                Some(LayoutAnalyzer::from_session(session))
            }else{None};
            let orientation=if package.resources.contains_key("orientation"){
                Some(DocOrientationClassifier::from_path(package.get("orientation")?)?)
            }else{None};
            package.verify()?;
            Ok(Self{package,ocr,layout,orientation})
        })
    }
    pub fn recognize_png(&mut self,png:&Path,expected_sha:Option<&str>)->Result<Value>{
        guarded(||self.recognize(png,expected_sha))
    }
    fn recognize(&mut self,png:&Path,expected_sha:Option<&str>)->Result<Value>{
        self.package.verify()?;
        ensure!(available().1>=512*1024*1024,"Insufficient free memory for another OCR page");
        let (mut img,source)=read_rgb(png,expected_sha)?;
        let (w,h)=img.dimensions();
        let (angle,orientation_score)=if let Some(classifier)=&mut self.orientation{
            let (o,score)=classifier.classify(&img)?;
            let n=o.degrees();
            img=match n{0=>img,90=>image::imageops::rotate270(&img),180=>image::imageops::rotate180(&img),270=>image::imageops::rotate90(&img),_=>bail!("Unexpected orientation class")};
            (n,finite_score(score))
        }else{(0,None)};
        let options=OcrOptions{return_word_box:true,use_doc_orientation:false,..Default::default()};
        let side=self.package.policy["max_detection_side"].as_u64().unwrap() as u32;
        let mut regions=Vec::new();
        let mut blocks=Vec::new();
        if let Some(layout)=&mut self.layout{
            let result=self.ocr.detect_with_layout(&img,layout,10,side,0.6,0.3,1.6,false,false,options)?;
            ensure!(result.page_angle==0,"Unexpected second page rotation");
            for (i,r) in result.layout_boxes.iter().enumerate(){
                let right=r.x.checked_add(r.w).context("Layout width overflow")?;
                let bottom=r.y.checked_add(r.h).context("Layout height overflow")?;
                let quad=[[r.x,r.y],[right,r.y],[right,bottom],[r.x,bottom]];
                regions.push(json!({"index":i,"class":format!("{:?}",r.semantic_class()),
                    "score":finite_score(r.score),"reading_order":r.reading_order,
                    "upright_pixel_bbox":[r.x,r.y,right,bottom],
                    "original_pixel_quad":map_quad(&quad,angle,w,h,true)}));
            }
            for (i,r) in result.blocks.iter().enumerate(){
                let mut block=block_record(&r.block,i,angle,w,h);
                block["layout_index"]=json!(r.layout_index);
                block["layout_distance"]=json!(if r.distance.is_finite(){Some(r.distance)}else{None});
                blocks.push(block);
            }
        }else{
            let result=self.ocr.detect_with_options(&img,10,side,0.6,0.3,1.6,false,false,options)?;
            ensure!(result.page_angle==0,"Unexpected second page rotation");
            blocks.extend(result.text_blocks.iter().enumerate().map(|(i,b)|block_record(b,i,angle,w,h)));
        }
        let review:Vec<_>=blocks.iter().filter(|b|b["location_valid"]!=true||b["text_score"].as_f64().is_none_or(|s|s<0.90)
            ||b["box_score"].as_f64().is_none_or(|s|s<0.60)).map(|b|b["index"].clone()).collect();
        let text=blocks.iter().filter_map(|b|b["text"].as_str()).collect::<Vec<_>>().join("\n");
        self.package.verify()?;
        ensure!(binding(png)?==source,"OCR source image changed during inference");
        Ok(json!({"schema_version":1,"source_image":source,"source_image_size":[w,h],
            "upright_image_size":[img.width(),img.height()],"observed_page_orientation_degrees":angle,
            "orientation_model_used":self.orientation.is_some(),"orientation_score":orientation_score,
            "orientation_coordinate_contract":"Original pixel-center points for OCR words/lines; pixel-edge points for layout regions",
            "model_package":self.package.report(),"layout_regions":regions,"blocks":blocks,"plain_text":text,
            "review_suggestion_indices":review,"status":if blocks.is_empty(){"no_text_detected_review_required"}else{"ocr_candidate"},
            "source_fidelity":"not_assessed","acceptance_ready":false,"evidence_boundary":BOUNDARY}))
    }
}
fn read_rgb(png:&Path,expected_sha:Option<&str>)->Result<(image::RgbImage,Value)>{
        let (raw,source)=read_small(png,MAX_PNG_BYTES)?;
        if let Some(expected)=expected_sha{ensure!(source["sha256"]==sha(&json!(expected))?,"OCR_IMAGE_HASH_MISMATCH");}
        let (w,h)=image::ImageReader::with_format(Cursor::new(&raw),image::ImageFormat::Png).into_dimensions()?;
        ensure!(w>0&&h>0&&u64::from(w)*u64::from(h)<=MAX_PIXELS,"OCR image exceeds safe pixel budget; choose a smaller region");
        let rgba=image::load_from_memory_with_format(&raw,image::ImageFormat::Png)?.into_rgba8();
        drop(raw);
        // Explicitly composite transparency over white, rather than silently
        // ignoring alpha or allowing hidden pixels into an OCR hypothesis.
        let rgb:Vec<u8>=rgba.as_raw().chunks_exact(4).flat_map(|p|{
            let a=u32::from(p[3]);[0usize,1,2].map(|c|((u32::from(p[c])*a+255*(255-a)+127)/255) as u8)
        }).collect();
        drop(rgba);

    let image=image::RgbImage::from_raw(w,h,rgb).context("Invalid RGB buffer")?;Ok((image,source))
}
fn finite_score(s:f32)->Option<f32>{s.is_finite().then_some(s)}

// The upstream orientation label is the observed clockwise rotation;
// correction uses its inverse (90 -> rotate270). Undo that correction.
// OCR points use pixel centers; layout rectangles use half-open edges.
fn unrotate(p:[u32;2],angle:u32,w:u32,h:u32,edges:bool)->Option<[f64;2]>{
    let (rw,rh)=if matches!(angle,90|270){(h,w)}else{(w,h)};
    let delta=if edges{0}else{1};
    let maxx=rw.checked_sub(delta)?;let maxy=rh.checked_sub(delta)?;
    if p[0]>maxx||p[1]>maxy{return None;}
    let (x,y)=(f64::from(p[0]),f64::from(p[1]));
    let (wx,hy)=(f64::from(w.checked_sub(delta)?),f64::from(h.checked_sub(delta)?));
    match angle{0=>Some([x,y]),90=>Some([wx-y,x]),180=>Some([wx-x,hy-y]),270=>Some([y,hy-x]),_=>None}
}
fn map_quad(q:&[[u32;2]],angle:u32,w:u32,h:u32,edges:bool)->Option<Vec<[f64;2]>>{
    if q.len()!=4{return None;}
    q.iter().map(|p|unrotate(*p,angle,w,h,edges)).collect()
}
fn point_list(points:&[Point])->Vec<[u32;2]>{points.iter().map(|p|[p.x,p.y]).collect()}
fn block_record(b:&TextBlock,i:usize,angle:u32,w:u32,h:u32)->Value{
    let q=point_list(&b.box_points);let mapped=map_quad(&q,angle,w,h,false);
    let words:Vec<_>=b.words.iter().map(|word|{
        let points=point_list(&word.box_points);let original=map_quad(&points,angle,w,h,false);
        json!({"text":word.text,"score":finite_score(word.score),"upright_pixel_quad":points,
            "original_pixel_quad":original,"location_valid":original.is_some()})
    }).collect();
    let valid=mapped.is_some()&&words.iter().all(|w|w["location_valid"]==true);
    json!({"index":i,"text":b.text,"text_score":finite_score(b.text_score),"box_score":finite_score(b.box_score),
        "upright_pixel_quad":q,"original_pixel_quad":mapped,"words":words,"location_valid":valid})
}
pub fn recognize_png(png:&Path,package:&Path,package_sha:&str,profile:&str,layout:&str,expected_sha:Option<&str>)->Result<Value>{
    let package=Package::load(package,package_sha,profile,layout)?;
    Engine::open(package)?.recognize_png(png,expected_sha)
}

/// Run the actual optional table model and independently preserve the ordinary
/// OCR readings plus any RT-DETR-L cell detections. Never trust the structure
/// box count as proof of cell identity (upstream can omit invalid boxes).
pub fn recognize_table_png(png:&Path,manifest:&Path,hash:&str,profile:&str,cell_mode:&str,expected:Option<&str>)->Result<Value>{
    guarded(||{
        let package=Package::for_table(manifest,hash,profile,cell_mode)?;
        let mut engine=Engine::open(package)?;
        let (img,source)=read_rgb(png,expected)?;
        let (w,h)=img.dimensions();
        let side=engine.package.policy["max_detection_side"].as_u64().unwrap() as u32;
        let options=OcrOptions{return_word_box:true,use_doc_orientation:false,..Default::default()};
        let result=engine.ocr.detect_with_options(&img,10,side,0.6,0.3,1.6,false,false,options)?;
        ensure!(result.page_angle==0,"Unexpected table crop rotation");
        let blocks:Vec<_>=result.text_blocks.iter().enumerate().map(|(i,b)|block_record(b,i,0,w,h)).collect();
        let model=engine.package.get("table_model")?;
        let dict=engine.package.get("table_dictionary")?;
        let mut recognizer=TableStructureRecognizer::from_path_with_dict(&model,Some(&dict))?.with_input_size(488);
        let structure=recognizer.recognize(&img)?;
        drop(recognizer);
        let parsed=match crate::ocr_table::parse_structure(&structure.html_tokens){
            Ok(v)=>v,Err(e)=>json!({"status":"structure_unresolved","reason":format!("{e:#}"),"acceptance_ready":false})
        };
        let raw_boxes:Vec<_>=structure.cell_boxes.iter().enumerate().map(|(i,b)|{
            let bbox=[f64::from(b.x1),f64::from(b.y1),f64::from(b.x2),f64::from(b.y2)];
            let quad=crate::ocr_table::cell_quad(bbox,w,h);
            json!({"index":i,"raw_bbox":bbox,"original_pixel_quad":quad,"location_valid":!quad.is_null()})
        }).collect();
        let mut cells=Vec::new();
        if cell_mode!="off"{
            ensure!(available().1>=1024*1024*1024,"Insufficient available memory for cell detection");
            let threads=engine.package.policy["threads"].as_u64().unwrap() as usize;
            let session=ort::session::Session::builder().map_err(ppocr_rs::OcrError::from)?
                .with_intra_threads(threads).map_err(ppocr_rs::OcrError::from)?
                .with_inter_threads(1).map_err(ppocr_rs::OcrError::from)?
                .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3).map_err(ppocr_rs::OcrError::from)?
                .commit_from_file(engine.package.get("table_cell_detector")?).map_err(ppocr_rs::OcrError::from)?;
            let mut detector=CellDetector::from_session(session);
            let detected=detector.detect(&img)?;
            ensure!(detected.len()<=4096,"Cell detection budget exceeded");
            for (i,b) in detected.iter().enumerate(){
                let bbox=[f64::from(b.left),f64::from(b.top),f64::from(b.right),f64::from(b.bottom)];
                let quad=crate::ocr_table::cell_quad(bbox,w,h);
                cells.push(json!({"index":i,"raw_bbox":bbox,"original_pixel_quad":quad,
                    "score":finite_score(b.score),"location_valid":!quad.is_null()}));
            }
        }
        let association=crate::ocr_table::associate_readings(&cells,&blocks)?;
        engine.package.verify()?;
        ensure!(binding(png)?==source,"Table crop changed during inference");
        Ok(json!({"schema_version":1,"source_image":source,"source_image_size":[w,h],"model_package":engine.package.report(),
            "layout_regions":[],"blocks":blocks,"raw_structure_tokens":structure.html_tokens,
            "structure_score":finite_score(structure.score),"structure_candidate":parsed,
            "structure_model_boxes":raw_boxes,"detected_cells":cells,"cell_mode":cell_mode,
            "box_identity_verified":false,"cell_association":association,"status":"table_candidate_review_required",
            "source_fidelity":"not_assessed","acceptance_ready":false,"evidence_boundary":crate::ocr_table::BOUNDARY}))
    })
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn conservative_cpu_policy(){
        assert_eq!(plan(4*GIB,2*GIB,64,"auto",None,"auto",true).unwrap()["profile"],"tiny");
        let p=plan(8*GIB,4*GIB,12,"auto",None,"auto",true).unwrap();
        assert_eq!(p["profile"],"small");assert_eq!(p["threads"],4);assert_eq!(p["gpu_required"],false);assert_eq!(p["layout_enabled"],true);
        assert_eq!(plan(32*GIB,12*GIB,32,"auto",None,"off",false).unwrap()["profile"],"medium");
        assert!(plan(4*GIB,2*GIB,8,"medium",None,"off",false).is_err());
        assert!(plan(8*GIB,2*GIB,8,"auto",None,"on",true).is_err());
        let m=Map::from_iter([("tiny".into(),json!({}))]);
        assert_eq!(plan(32*GIB,12*GIB,32,"auto",Some(&m),"off",false).unwrap()["profile"],"tiny");
    }
    #[test]
    fn rotation_inverse_keeps_source_coordinates(){
        assert_eq!(unrotate([2,3],90,10,6,false),Some([6.,2.]));
        assert_eq!(unrotate([2,3],270,10,6,false),Some([3.,3.]));
        assert_eq!(unrotate([0,10],90,10,6,true),Some([0.,0.]));
        assert!(unrotate([6,2],90,10,6,false).is_none());
        assert_eq!(unrotate([2,3],180,10,6,false),Some([7.,2.]));
    }
}
