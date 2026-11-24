//! Explicit GPU provider attempts with crash/timeout-safe CPU restart.
//! Each worker is the same native z2o executable, never Python. Both
//! attempts use exactly the same selected recognition model resources.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{fs,io::Read,path::Path,process::{Command,Stdio},thread,time::{Duration,Instant}};
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
