//! Explicit GPU provider attempts with crash/timeout-safe CPU restart.
//! Each worker is the same native z2o executable, never legacy. Both
//! attempts use exactly the same selected recognition model resources.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{fs,io::Read,path::{Path,PathBuf},process::{Command,Stdio},thread,time::{Duration,Instant}};
use crate::{check_ledger,files,ocr_backend,path,parse_json};
const CPU:&str="CPUExecutionProvider";
fn gpu_provider()->&'static str{
    if cfg!(target_os="windows"){"DmlExecutionProvider"}else if cfg!(target_os="macos"){"CoreMLExecutionProvider"}else{"CUDAExecutionProvider"}
}
fn text<'a>(v:&'a Value,label:&str)->Result<&'a str>{v.as_str().filter(|s|!s.is_empty()).with_context(||format!("{label} required"))}
pub fn worker(v:&Value)->Result<Value>{
    ensure!(v["schema_version"].as_u64()==Some(1),"Invalid OCR worker schema");
    let png=path(&v["image"])?;let manifest=path(&v["manifest"])?;
    let provider=text(&v["provider"],"provider")?;
    let package=ocr_backend::Package::load_for(&manifest,text(&v["manifest_sha256"],"manifest digest")?,text(&v["profile"],"profile")?,text(&v["layout"],"layout")?,provider)?;
    let mut result=ocr_backend::Engine::open(package)?.recognize_png(&png,Some(text(&v["image_sha256"],"image digest")?))?;
    result["execution_provider_requested"]=json!(provider);
    result["provider_registration_policy"]=json!("error_on_failure");
    result["gpu_node_placement_verified"]=json!(false);
    Ok(result)
}
fn run_worker(v:&Value,timeout:Duration)->Result<Value>{
    let temp=tempfile::tempdir()?;
    let request=temp.path().join("request.json");fs::write(&request,serde_json::to_vec(v)?)?;
    let stdout=temp.path().join("stdout.json");let stderr=temp.path().join("stderr.txt");
    let exe=std::env::current_exe()?;
    let mut cmd=Command::new(exe);
    cmd.arg("ocr-provider-worker").arg(&request).stdin(Stdio::null())
        .stdout(Stdio::from(fs::File::create(&stdout)?)).stderr(Stdio::from(fs::File::create(&stderr)?));
    let manifest=path(&v["manifest"])?;
    // ORT's provider shared libraries should live with the independently
    // bound runtime. Keep installed system driver discovery; no shell.
    let runtime=path(&v["runtime_path"])?;let dir=runtime.parent().context("Runtime directory")?;
    cmd.current_dir(dir);
    #[cfg(target_os="linux")]{cmd.env("LD_LIBRARY_PATH",dir);cmd.env_remove("LD_PRELOAD");}
    #[cfg(target_os="macos")]{cmd.env("DYLD_LIBRARY_PATH",dir);cmd.env_remove("DYLD_INSERT_LIBRARIES");}
    #[cfg(windows)]{cmd.env("PATH",dir);}
    if v["provider"]==CPU{cmd.env("CUDA_VISIBLE_DEVICES","").env("HIP_VISIBLE_DEVICES","");}
    let mut child=cmd.spawn()?;
    let begin=Instant::now();
    let status=loop{
        match child.try_wait(){Ok(Some(s))=>break s,Ok(None)=>{},Err(e)=>{let _=child.kill();let _=child.wait();return Err(e.into());}}
        if begin.elapsed()>=timeout||fs::metadata(&stdout).map_or(true,|m|m.len()>64*1024*1024)||fs::metadata(&stderr).map_or(true,|m|m.len()>16*1024*1024){
            let _=child.kill();let _=child.wait();anyhow::bail!("OCR worker deadline/output budget exceeded");
        }
        thread::sleep(Duration::from_millis(100));
    };
    if !status.success(){
        let mut reason=Vec::new();fs::File::open(&stderr)?.take(4096).read_to_end(&mut reason)?;
        anyhow::bail!("OCR worker exited {status}: {}",String::from_utf8_lossy(&reason));
    }
    ensure!(fs::metadata(&stdout)?.len()<=64*1024*1024,"OCR worker output exceeds budget");
    let raw=fs::read(&stdout)?;let result=parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(result["source_image"]["sha256"]==v["image_sha256"]&&result["model_package"]["manifest"]["sha256"]==v["manifest_sha256"]&&
        result["model_package"]["hardware"]["execution_provider"]==v["provider"]&&result["acceptance_ready"]==false,
        "OCR worker output binding differs");
    ensure!(check_ledger::bind_file(&manifest)?["sha256"]==v["manifest_sha256"],"Manifest changed during worker");Ok(result)
}
fn models(report:&Value)->Value{
    let mut result=serde_json::Map::new();
    for k in ["detector","recognizer","dictionary","orientation","layout"]{
        if let Some(v)=report["resources"].get(k){result.insert(k.into(),v["sha256"].clone());}
    }
    Value::Object(result)
}
pub struct Options<'a>{
    pub image:&'a Path,pub image_sha:&'a str,pub cpu:&'a Path,pub cpu_sha:&'a str,
    pub gpu:Option<(&'a Path,&'a str)>,pub profile:&'a str,pub layout:&'a str,pub acceleration:&'a str,pub timeout:u64,
}
pub fn recognize(opt:Options<'_>)->Result<Value>{
    ensure!(matches!(opt.acceleration,"auto"|"cpu"|"gpu_required")&&(5..=1800).contains(&opt.timeout),"Invalid accelerator policy or deadline");
    let img=check_ledger::bind_file(opt.image)?;
    ensure!(img["sha256"]==opt.image_sha,"Source image changed");
    // Resolve a single model profile once. GPU failure is not permission to
    // change the model, dictionary, image, or layout configuration.
    let cpu=ocr_backend::Package::load(opt.cpu,opt.cpu_sha,opt.profile,opt.layout)?.report();
    let profile=text(&cpu["hardware"]["profile"],"resolved profile")?;
    let layout=if cpu["hardware"]["layout_enabled"]==true{"on"}else{"off"};
    let mut attempts=vec![];
    let mut selected=None;
    if opt.acceleration!="cpu"{
        let outcome=(||->Result<Value>{
            let (p,h)=opt.gpu.context("No explicitly verified accelerator package supplied")?;
            let provider=gpu_provider();
            let report=ocr_backend::Package::load_for(p,h,profile,layout,provider)?.report();
            ensure!(models(&report)==models(&cpu),"GPU package would change recognition models");
            let req=json!({"schema_version":1,"image":img["path"],"image_sha256":opt.image_sha,"manifest":report["manifest"]["path"],
                "manifest_sha256":h,"profile":profile,"layout":layout,"provider":provider,"runtime_path":report["resources"]["runtime"]["path"]});
            run_worker(&req,Duration::from_secs(opt.timeout))
        })();
        match outcome{Ok(r)=>{attempts.push(json!({"provider":gpu_provider(),"status":"completed","node_placement_verified":false}));selected=Some(r);},
            Err(e)=>attempts.push(json!({"provider":gpu_provider(),"status":"failed_or_unavailable","reason":format!("{e:#}")}))}
    }
    if selected.is_none()&&opt.acceleration!="gpu_required"{
        let req=json!({"schema_version":1,"image":img["path"],"image_sha256":opt.image_sha,"manifest":cpu["manifest"]["path"],
            "manifest_sha256":opt.cpu_sha,"profile":profile,"layout":layout,"provider":CPU,"runtime_path":cpu["resources"]["runtime"]["path"]});
        match run_worker(&req,Duration::from_secs(opt.timeout)){
            Ok(r)=>{attempts.push(json!({"provider":CPU,"status":"completed"}));selected=Some(r);}
            Err(e)=>attempts.push(json!({"provider":CPU,"status":"failed","reason":format!("{e:#}")})),
        }
    }
    ensure!(check_ledger::bind_file(opt.image)?==img,"Image changed during acceleration attempts");
    if let Some(mut r)=selected{
        r["acceleration_attempts"]=json!(attempts);
        r["cpu_fallback_used"]=json!(opt.acceleration=="auto"&&r["model_package"]["hardware"]["execution_provider"]==CPU);
        Ok(r)
    }else{Ok(json!({"schema_version":1,"source_image":img,"status":"ocr_acceleration_blocked","acceleration_attempts":attempts,
        "acceptance_ready":false,"source_fidelity":"not_assessed"}))}
}


#[derive(Clone)]
pub struct Settings{pub mode:String,pub gpu:Option<(PathBuf,String)>,pub timeout:u64}
impl Settings{
    pub fn from_value(v:&Value)->Result<Option<Self>>{
        if v.is_null(){return Ok(None);}
        let m=v.as_object().context("Accelerator config must be an object")?;
        ensure!(m.keys().all(|k|matches!(k.as_str(),"acceleration"|"gpu_package_manifest"|"expected_gpu_package_sha256"|"timeout_seconds")),"Unknown accelerator config field");
        let mode=v.get("acceleration").map(|v|text(v,"acceleration")).transpose()?.unwrap_or("auto").to_owned();
        let timeout=v.get("timeout_seconds").map(|v|v.as_u64().context("timeout must be integer")).transpose()?.unwrap_or(180);
        ensure!(matches!(mode.as_str(),"auto"|"cpu"|"gpu_required")&&(5..=1800).contains(&timeout),"Invalid accelerator configuration");
        let gpu=match(v.get("gpu_package_manifest").filter(|v|!v.is_null()),v.get("expected_gpu_package_sha256").filter(|v|!v.is_null())){
            (None,None)=>None,
            (Some(p),Some(h))=>{
                let p=path(p)?;crate::check_write::no_link(&p)?;let p=files::canonical(&p)?;
                let h=text(h,"GPU package SHA-256")?.to_owned();
                ensure!(check_ledger::bind_file(&p)?["sha256"]==h,"GPU manifest hash differs");
                Some((p,h))
            },
            _=>anyhow::bail!("GPU manifest and hash must be supplied together")
        };
        ensure!(mode!="gpu_required"||gpu.is_some(),"gpu_required needs a verified GPU package");
        Ok(Some(Self{mode,gpu,timeout}))
    }
    pub fn binding(&self)->Result<Value>{
        let gpu=if let Some((p,h))=&self.gpu{
            let b=check_ledger::bind_file(p)?;ensure!(b["sha256"]==*h,"GPU manifest changed");b
        }else{Value::Null};
        Ok(json!({"acceleration":self.mode,"gpu_manifest":gpu,"timeout_seconds":self.timeout}))
    }
}
// Reuse one resolved model profile throughout the job; after a real failure
// do not repeatedly initialize a known-broken accelerator on every page.
#[derive(Default)]
pub struct JobState{resolved:Option<(String,String)>,gpu_failure:Option<Value>}
impl JobState{
    pub fn run(&mut self,png:&Path,image_sha:&str,cpu:&Path,cpu_sha:&str,profile:&str,layout:&str,cfg:&Settings)->Result<Value>{
        cfg.binding()?;
        if self.resolved.is_none(){
            let report=ocr_backend::Package::load(cpu,cpu_sha,profile,layout)?.report();
            self.resolved=Some((text(&report["hardware"]["profile"],"resolved profile")?.to_owned(),
                if report["hardware"]["layout_enabled"]==true{"on".into()}else{"off".into()}));
        }
        let (profile,layout)=self.resolved.as_ref().unwrap();
        let disabled=self.gpu_failure.is_some()&&cfg.mode=="auto";
        let mode=if disabled{"cpu"}else{cfg.mode.as_str()};
        let mut r=recognize(Options{image:png,image_sha,cpu,cpu_sha,gpu:cfg.gpu.as_ref().map(|(p,h)|(p.as_path(),h.as_str())),
            profile,layout,acceleration:mode,timeout:cfg.timeout})?;
        if disabled{
            r["cpu_fallback_used"]=json!(true);r["prior_gpu_failure"]=self.gpu_failure.clone().unwrap();
        }else if cfg.mode=="auto"{
            if let Some(a)=r["acceleration_attempts"].as_array().and_then(|rows|rows.iter().find(|row|row["status"]=="failed_or_unavailable")){
                self.gpu_failure=Some(a.clone());
            }
        }
        if r["status"]=="ocr_acceleration_blocked"{anyhow::bail!("All OCR attempts blocked: {}",r["acceleration_attempts"]);}
        cfg.binding()?;Ok(r)
    }
}
