//! Direct native client for the official MiniCPM-OCR and Jev GGUF models.
//! An independently verified llama-server process per attempt isolates GPU
//! failures and CPU fallback. No Python SDK, browser renderer or remote API.
use anyhow::{ensure,Context,Result,bail};
use base64::{engine::general_purpose::STANDARD,Engine as _};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,BTreeSet},fs,io::{BufReader,Read,Seek,SeekFrom,Write},net::{Ipv4Addr,SocketAddr,TcpListener,TcpStream},
    path::{Path,PathBuf},process::{Child,Command,ExitStatus,Stdio},time::{Duration,Instant},thread};
use crate::{check_ledger,check_write,files,path,path_text,parse_json};
pub const UPSTREAM:&str="c21a3ca524263ef8166cb6d350864b10c3056b61";
pub const LLAMA_RELEASE:&str="b11901";
pub const JEV_LLAMA_RELEASE:&str="b11928";
fn release_for(kind:&str)->Result<&'static str>{
    match kind{
        "minicpm-ocr"=>Ok(LLAMA_RELEASE),
        "jev" if matches!((std::env::consts::OS,std::env::consts::ARCH),
            ("linux"|"windows","x86_64"|"aarch64")|("macos","aarch64"))=>Ok(JEV_LLAMA_RELEASE),
        "jev"=>bail!("JEV_RUNTIME_PLATFORM_UNSUPPORTED"),
        _=>bail!("Unknown model kind"),
    }
}
const BOUNDARY:&str="Actual local MiniCPM inference candidate, not verified transcription. Jev class-1 probability is a model score, not a correctness decision. Candidate render association, source content, formulas and tables require independent review. GPU offload evidence is runtime-reported, not a benchmark.";
const GIB:u64=1024*1024*1024;
const MAX_IMAGE:u64=32*1024*1024;
fn text<'a>(v:&'a Value,label:&str)->Result<&'a str>{v.as_str().filter(|s|!s.is_empty()).with_context(||format!("{label} required"))}
fn hash(v:&Value)->Result<&str>{let s=text(v,"SHA-256")?;ensure!(s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)),"Actual lowercase SHA-256 required");Ok(s)}
fn relative(s:&str)->Result<PathBuf>{
    ensure!(!s.is_empty()&&!s.contains(['\\',':','\0'])&&!s.split('/').any(|x|x.is_empty()||matches!(x,"."|"..")||x.ends_with(['.',' '])||x.chars().any(|c|c.is_control()||"<>\"|?*".contains(c))),"Unsafe relative package path");
    let p=PathBuf::from(s);ensure!(p.is_relative(),"Absolute package path forbidden");Ok(p)
}
fn no_link(p:&Path)->Result<fs::Metadata>{
    let m=fs::symlink_metadata(p)?;ensure!(!m.file_type().is_symlink(),"Linked model/engine file prohibited");
    #[cfg(windows)]{use std::os::windows::fs::MetadataExt;ensure!(m.file_attributes()&0x400==0,"Reparse-point model/engine file prohibited");}
    Ok(m)
}
fn local(root:&Path,r:&str)->Result<PathBuf>{
    let rel=relative(r)?;let mut p=root.to_owned();
    for c in rel.components(){p.push(c);no_link(&p)?;}
    let p=files::canonical(&p)?;ensure!(p.starts_with(root),"Package path escaped root");Ok(p)
}
fn bind(root:&Path,row:&Value,max:u64)->Result<Value>{
    let p=local(root,text(&row["path"],"asset path")?)?;let meta=no_link(&p)?;
    ensure!(meta.is_file()&&meta.len()<=max,"Model/engine asset exceeds size budget or is not a file");
    let b=check_ledger::bind_file(&p)?;ensure!(b["sha256"]==hash(&row["sha256"])?,"Model/engine resource changed: {}",p.display());Ok(b)
}
fn verify(b:&Value)->Result<()>{let p=path(&b["path"])?;check_write::no_link(&p)?;ensure!(check_ledger::bind_file(&p)?==*b,"Bound input changed");Ok(())}
fn gguf(root:&Path,row:&Value,allowed:&[&str])->Result<Value>{
    let b=bind(root,row,8*GIB)?;
    let p=path(&b["path"])?;
    ensure!(p.file_name().and_then(|s|s.to_str()).is_some_and(|s|allowed.contains(&s)),"Expected actual official MiniCPM GGUF filename");
    let mut magic=[0;4];fs::File::open(p)?.read_exact(&mut magic)?;
    ensure!(&magic==b"GGUF","Asset is not a GGUF model");Ok(b)
}
struct EngineFiles{root:PathBuf,exe:PathBuf,files:Vec<Value>,variant:String}
impl EngineFiles{
    fn load(root:&Path,v:&Value)->Result<Self>{
        let dir=local(root,text(&v["directory"],"Engine directory")?)?;ensure!(no_link(&dir)?.is_dir(),"Expected engine directory");
        let variant=text(&v["variant"],"engine variant")?.to_owned();
        let manifest=v["files"].as_array().filter(|a|!a.is_empty()&&a.len()<=512).context("Declare all engine distribution files")?;
        let mut expected=BTreeMap::new();let mut aliases=BTreeSet::new();
        for row in manifest{
            let r=text(&row["path"],"Engine relative file")?;
            use unicode_normalization::UnicodeNormalization;use unicode_casefold::UnicodeCaseFold;
            let k:String=r.nfc().collect::<String>().case_fold().collect();ensure!(aliases.insert(k),"Ambiguous engine file path");
            let b=bind(&dir,row,2*GIB)?;
            ensure!(expected.insert(path(&b["path"])?,b).is_none(),"Duplicate engine file");
        }
        let mut todo=vec![dir.clone()];let mut found=BTreeSet::new();let mut n=0;
        while let Some(p)=todo.pop(){
            for e in fs::read_dir(p)?{
                let e=e?;let p=e.path();n+=1;ensure!(n<=2048,"Engine directory exceeds budget");
                let m=no_link(&p)?;
                if m.is_dir(){todo.push(p);continue;}
                ensure!(m.is_file(),"Unexpected engine directory entry");let p=files::canonical(&p)?;
                ensure!(expected.contains_key(&p)&&found.insert(p),"Unbound or aliased engine file");
            }
        }
        ensure!(found.len()==expected.len(),"Engine distribution incomplete");
        let exe=local(&dir,text(&v["executable"],"llama-server executable")?)?;
        ensure!(fs::metadata(&exe)?.len()>0&&expected.contains_key(&exe)&&exe.file_name().and_then(|s|s.to_str())==Some(if cfg!(windows){"llama-server.exe"}else{"llama-server"}),"Expected bound native llama-server executable");
        Ok(Self{root:dir,exe,files:expected.into_values().collect(),variant})
    }
    fn verify(&self)->Result<()>{for b in &self.files{verify(b)?;}Ok(())}
    fn report(&self,release:&str)->Value{json!({"variant":self.variant,"executable":self.exe,"files":self.files,"release":release})}
}
struct Package{binding:Value,kind:String,release:&'static str,cpu:EngineFiles,gpu:Option<EngineFiles>,gpu_error:Option<String>,model:Value,projector:Value,draft:Option<Value>}
impl Package{
    fn load(p:&Path,h:&str,kind:&str)->Result<Self>{
        let release=release_for(kind)?;
        check_write::no_link(p)?;ensure!(fs::metadata(p)?.len()<=2*1024*1024,"Package manifest too large");
        let(raw,b)=files::read_bound(p)?;
        ensure!(raw.len()<=2*1024*1024&&b["sha256"]==h,"MINICPM_PACKAGE_HASH_MISMATCH");
        let v=parse_json(std::str::from_utf8(&raw)?)?;
        ensure!(v["schema_version"].as_u64()==Some(1)&&v["engine"]=="llama.cpp"&&v["model_kind"]==kind,"Unexpected MiniCPM package identity");
        ensure!(v["release"]==release,"MINICPM_RUNTIME_RELEASE_MISMATCH: {kind} requires {release}");
        ensure!(v["platform"]==std::env::consts::OS&&v["architecture"]==std::env::consts::ARCH,"Package platform mismatch");
        let root=path(&b["path"])?.parent().context("Package root")?.to_owned();
        let cpu=EngineFiles::load(&root,&v["cpu"])?;
        ensure!(cpu.variant=="cpu","CPU fallback package must declare CPU");
        let (gpu,gpu_error)=if v["gpu"].is_null(){(None,Some("No verified GPU distribution supplied".to_owned()))}else{
            let result=(||->Result<EngineFiles>{
                let e=EngineFiles::load(&root,&v["gpu"])?;
                ensure!(matches!(e.variant.as_str(),"cuda"|"vulkan"|"metal")&&(!cfg!(target_os="macos")||e.variant=="metal"),"Unsupported GPU variant");
                Ok(e)
            })();
            match result{Ok(e)=>(Some(e),None),Err(e)=>(None,Some(format!("{e:#}")))}
        };
        let (model,projector,draft)=match kind{
            "minicpm-ocr"=>(
                gguf(&root,&v["model"],&["MiniCPM-OCR-1_0-Q8_0.gguf","MiniCPM-OCR-1_0-Q4_K_M.gguf"])?,
                gguf(&root,&v["projector"],&["mmproj-model-f16-4x.gguf"])?,
                if v["draft"].is_null(){None}else{
                    ensure!(v["model"]["path"].as_str().is_some_and(|p|p.ends_with("MiniCPM-OCR-1_0-Q8_0.gguf")),"DSpark draft must match Q8 main model");
                    Some(gguf(&root,&v["draft"],&["dspark-MiniCPM-OCR-1_0-Q8_0.gguf"])?)
                }),
            "jev"=>(
                gguf(&root,&v["model"],&["MiniCPM-OCR-Jev-Q4_K_M.gguf","MiniCPM-OCR-Jev-Q8_0.gguf","MiniCPM-OCR-Jev-F16.gguf"])?,
                gguf(&root,&v["projector"],&["mmproj-model-f16.gguf"])?,None),
            _=>bail!("Unknown model kind"),
        };
        ensure!(model["path"]!=projector["path"]&&draft.as_ref().is_none_or(|b|b["path"]!=model["path"]&&b["path"]!=projector["path"]),"Model resources must differ");
        let p=Self{binding:b,kind:kind.into(),release,cpu,gpu,gpu_error,model,projector,draft};p.verify_models()?;Ok(p)
    }
    fn verify_models(&self)->Result<()>{verify(&self.binding)?;verify(&self.model)?;verify(&self.projector)?;if let Some(b)=&self.draft{verify(b)?;}Ok(())}
    fn report(&self,e:&EngineFiles)->Value{json!({"manifest":self.binding,"kind":self.kind,"model":self.model,"projector":self.projector,
        "draft":self.draft,"runtime":e.report(self.release),"upstream_revision":UPSTREAM,"qualified":false})}
}
fn request(addr:SocketAddr,token:&str,route:&str,body:Option<&Value>,timeout:Duration)->Result<Value>{
    ensure!(addr.ip().is_loopback()&&matches!(route,"/health"|"/v1/models"|"/v1/chat/completions"|"/v1/systemone"),"Only local native model endpoints");
    let mut stream=TcpStream::connect_timeout(&addr,Duration::from_secs(2))?;
    stream.set_read_timeout(Some(timeout))?;stream.set_write_timeout(Some(timeout.min(Duration::from_secs(30))))?;
    let data=if let Some(b)=body{serde_json::to_vec(b)?}else{vec![]};
    ensure!(data.len()<=100*1024*1024,"Inference request exceeds budget");
    write!(stream,"{} {route} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept-Encoding: identity\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",if body.is_some(){"POST"}else{"GET"},addr.port(),data.len())?;
    stream.write_all(&data)?;
    let r=crate::zotero_local::decode_http(BufReader::new(stream))?;
    parse_json(std::str::from_utf8(&r.body)?)
}
struct Server{child:Child,temp:tempfile::TempDir,addr:SocketAddr,token:String,offload:Option<(u64,u64)>}
impl Drop for Server{fn drop(&mut self){let _=self.child.kill();let _=self.child.wait();}}
fn command(e:&EngineFiles)->Command{
    let mut cmd=Command::new(&e.exe);
    let dir=e.exe.parent().unwrap_or(&e.root);
    cmd.current_dir(dir).stdin(Stdio::null());
    // Ignore inherited model/server overrides and proxies. GPU drivers may
    // still come from the platform's installed driver/runtime environment.
    for(k,_)in std::env::vars_os(){let s=k.to_string_lossy().to_ascii_uppercase();if s.starts_with("LLAMA_")||s.ends_with("_PROXY"){cmd.env_remove(k);}}
    cmd.env("PATH",dir);
    #[cfg(target_os="linux")]{cmd.env("LD_LIBRARY_PATH",dir);cmd.env_remove("LD_PRELOAD");}
    #[cfg(target_os="macos")]{cmd.env("DYLD_LIBRARY_PATH",dir);cmd.env_remove("DYLD_INSERT_LIBRARIES");}
    cmd
}
fn log_offload(p:&Path)->Result<Option<(u64,u64)>>{
    ensure!(fs::metadata(p)?.len()<=8*1024*1024,"Runtime startup log exceeds budget");
    let s=fs::read_to_string(p)?;
    let re=regex::Regex::new(r"offloaded ([0-9]+)/([0-9]+) layers to GPU")?;
    let r=re.captures_iter(&s).last().and_then(|c|Some((c[1].parse().ok()?,c[2].parse().ok()?)));
    Ok(r)
}
// Only closed-vocabulary classifications leave the isolated temp directory.
// Never return the actual bytes, an arbitrary log line, request/response data,
// environment values, model paths or the child command line.
const STARTUP_LOG_SAMPLE:u64=32*1024;
#[derive(Debug)]
struct StartupDiagnostic{
    phase:&'static str,exit:Option<ExitStatus>,signatures:Vec<&'static str>,sampled_bytes:usize,truncated:bool,log_readable:bool,
}
impl StartupDiagnostic{
    fn collect(file:&mut fs::File,phase:&'static str,exit:Option<ExitStatus>)->Self{
        let mut diag=Self{phase,exit,signatures:vec![],sampled_bytes:0,truncated:false,log_readable:false};
        let read=(||->std::io::Result<Vec<u8>>{
            // Hold the original file handle opened before starting the child.
            // It cannot be replaced with a new path or a blocking named pipe.
            let meta=file.metadata()?;
            if !meta.is_file(){return Err(std::io::ErrorKind::InvalidData.into());}
            let size=meta.len();diag.truncated=size>STARTUP_LOG_SAMPLE;
            file.seek(SeekFrom::Start(0))?;
            let mut sample=Vec::with_capacity(size.min(STARTUP_LOG_SAMPLE) as usize);
            if diag.truncated{
                let half=STARTUP_LOG_SAMPLE/2;
                (&mut *file).take(half).read_to_end(&mut sample)?;
                let boundary=sample.len();
                file.seek(SeekFrom::Start(size-half))?;
                file.take(half).read_to_end(&mut sample)?;
                // A pattern must not be created by joining separated bytes.
                sample.insert(boundary,b'\n');
            }else{file.take(size).read_to_end(&mut sample)?;}
            Ok(sample)
        })();
        if let Ok(sample)=read{
            diag.log_readable=true;
            diag.sampled_bytes=sample.len()-usize::from(diag.truncated);
            diag.signatures=startup_signatures(&String::from_utf8_lossy(&sample).to_ascii_lowercase());
        }
        diag
    }
    fn json(&self)->Value{
        json!({"phase":self.phase,"exit_code":self.exit.and_then(|v|v.code()),
            "signatures":self.signatures,"log_bytes_sampled":self.sampled_bytes,
            "log_sample_truncated":self.truncated,"log_readable":self.log_readable})
    }
}
impl std::fmt::Display for StartupDiagnostic{
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{
        if let Some(status)=self.exit{write!(f,"llama-server exited during startup: {status}")?;}
        else{f.write_str("llama-server startup deadline exceeded")?;}
        if !self.signatures.is_empty(){write!(f," ({})",self.signatures.join(", "))?;}
        Ok(())
    }
}
impl std::error::Error for StartupDiagnostic{}
fn startup_signatures(log:&str)->Vec<&'static str>{
    let patterns:&[(&str,&[&str])]=&[
        ("shared_library_load_failed",&["error while loading shared libraries","library not loaded:","cannot open shared object file"]),
        ("unsupported_server_option",&["unknown argument","unrecognized argument","unrecognized option","invalid argument: --"]),
        ("unsupported_decision_model",&["unsupported decision model type"]),
        ("unsupported_model_architecture",&["unknown model architecture","unsupported model architecture","unsupported architecture"]),
        ("invalid_model_metadata",&["missing tensor","tensor not found","unknown tensor type","unknown pre-tokenizer","unknown pretokenizer","failed to load model vocabulary","invalid gguf","gguf_init_from_file: failed"]),
        ("projector_load_failed",&["failed to load mmproj","failed to load the mmproj","failed to load projector","failed to load clip","failed to load multimodal","clip_model_load: error","mtmd_init_from_file: error"]),
        ("memory_allocation_failed",&["std::bad_alloc","cannot allocate memory","out of memory","failed to allocate","unable to allocate"]),
        ("endpoint_bind_failed",&["address already in use","failed to bind","couldn't bind","could not bind","failed to listen"]),
        ("model_load_failed",&["failed to load model","error loading model","failed loading model"]),
        ("runtime_assertion_failed",&["ggml_assert","assertion failed","illegal instruction"]),
    ];
    let abi_version_missing=log.lines().any(|line|line.contains("version")&&line.contains("not found")&&
        ["glibc_","glibcxx_","cxxabi_"].iter().any(|marker|line.contains(marker)));
    abi_version_missing.then_some("loader_abi_version_missing").into_iter()
        .chain(patterns.iter().filter(|(_,markers)|markers.iter().any(|marker|log.contains(marker))).map(|(code,_)|*code))
        .collect()
}
// Keep the same conservative reservation. Report only byte counts when the
// host cannot satisfy it; a startup log does not exist in that case.
const MODEL_MEMORY_RESERVE:u64=2*GIB;
#[derive(Debug)]
struct MemoryBudgetError{available_bytes:u64,required_bytes:u64,host_weight_bytes:u64}
impl MemoryBudgetError{
    fn json(&self)->Value{
        json!({"available_bytes":self.available_bytes,"required_bytes":self.required_bytes,
            "host_weight_bytes":self.host_weight_bytes,"reserve_bytes":MODEL_MEMORY_RESERVE})
    }
}
impl std::fmt::Display for MemoryBudgetError{
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{
        f.write_str("Insufficient conservative memory budget for this MiniCPM attempt")
    }
}
impl std::error::Error for MemoryBudgetError{}
fn require_model_memory(available_bytes:u64,weight_bytes:u64,gpu:bool)->Result<()>{
    let host_weight_bytes=if gpu{0}else{weight_bytes};
    let required_bytes=host_weight_bytes.saturating_add(MODEL_MEMORY_RESERVE);
    if available_bytes<required_bytes{
        return Err(MemoryBudgetError{available_bytes,required_bytes,host_weight_bytes}.into());
    }
    Ok(())
}
impl Server{
    fn start(p:&Package,e:&EngineFiles,gpu:bool,timeout:Duration)->Result<Self>{
        p.verify_models()?;e.verify()?;
        let weight_bytes=fs::metadata(path(&p.model["path"])?)?.len()+fs::metadata(path(&p.projector["path"])?)?.len()+
            p.draft.as_ref().map(|b|fs::metadata(path(&b["path"])?).map(|m|m.len()).map_err(anyhow::Error::from)).transpose()?.unwrap_or(0);
        let mut sys=sysinfo::System::new();sys.refresh_memory();
        require_model_memory(sys.available_memory(),weight_bytes,gpu)?;
        let temp=tempfile::Builder::new().prefix("z2o-llama-").tempdir()?;
        let a=tempfile::NamedTempFile::new_in(temp.path())?;
        let b=tempfile::NamedTempFile::new_in(temp.path())?;
        let token=files::digest(format!("{}{}",a.path().display(),b.path().display()).as_bytes());
        let auth=temp.path().join("key.txt");fs::write(&auth,format!("{token}\n"))?;
        let listener=TcpListener::bind((Ipv4Addr::LOCALHOST,0))?;let addr=listener.local_addr()?;drop(listener);
        let log=temp.path().join("server.log");let output=fs::File::create(&log)?;
        let mut diagnostic_input=fs::File::open(&log)?;
        let mut cmd=command(e);
        cmd.arg("--model").arg(path(&p.model["path"])?).arg("--mmproj").arg(path(&p.projector["path"])?)
            .args(["--host","127.0.0.1","--port",&addr.port().to_string(),"--parallel","1","--no-webui","--api-key-file"]).arg(&auth)
            .args(["--alias",if p.kind=="jev"{"z2o-jev"}else{"z2o-minicpm"},"--ctx-size",if p.kind=="jev"{"16384"}else{"8192"}])
            .args(["--threads",&std::thread::available_parallelism().map(|n|n.get().min(8)).unwrap_or(1).to_string()]);
        if p.kind=="minicpm-ocr"{cmd.args(["--jinja","--reasoning","off"]);}
        if gpu{cmd.args(["--n-gpu-layers","99"]);}
        else{cmd.args(["--device","none","--n-gpu-layers","0","--no-mmproj-offload"]).env("CUDA_VISIBLE_DEVICES","").env("HIP_VISIBLE_DEVICES","");}
        if let Some(d)=&p.draft{
            cmd.arg("--model-draft").arg(path(&d["path"])?).args(["--spec-type","draft-dspark","--spec-draft-n-max","15","--gpu-layers-draft",if gpu{"99"}else{"0"}]);
        }
        let child=cmd.stdout(Stdio::from(output.try_clone()?)).stderr(Stdio::from(output)).spawn().context("Cannot launch bound native llama-server")?;
        let mut s=Self{child,temp,addr,token,offload:None};
        let start=Instant::now();
        while start.elapsed()<timeout{
            if let Some(status)=s.child.try_wait()?{
                return Err(StartupDiagnostic::collect(&mut diagnostic_input,"early_exit",Some(status)).into());
            }
            if let Ok(v)=request(addr,&s.token,"/health",None,Duration::from_secs(2)){
                if v["status"]=="ok"{
                    let v=request(addr,&s.token,"/v1/models",None,Duration::from_secs(5))?;
                    let alias=if p.kind=="jev"{"z2o-jev"}else{"z2o-minicpm"};
                    ensure!(v["data"].as_array().is_some_and(|a|a.iter().any(|r|r["id"]==alias)),"Native server model alias not verified");
                    s.offload=log_offload(&s.temp.path().join("server.log"))?;
                    if gpu{ensure!(s.offload.is_some_and(|(n,total)|n>0&&n<=total),"No actual GPU layer offload reported; retry CPU");}
                    else{ensure!(s.offload.is_none_or(|(n,_)|n==0),"CPU attempt unexpectedly reports GPU offload");}
                    return Ok(s);
                }
            }
            thread::sleep(Duration::from_millis(500));
        }
        let exit=s.child.try_wait()?;
        Err(StartupDiagnostic::collect(&mut diagnostic_input,if exit.is_some(){"early_exit"}else{"startup_timeout"},exit).into())
    }
}
fn image(p:&Path,h:&str)->Result<(Value,String)>{
    check_write::no_link(p)?;ensure!(fs::metadata(p)?.len()<=MAX_IMAGE,"Image exceeds size budget");
    let(raw,b)=files::read_bound(p)?;
    ensure!(!raw.is_empty()&&raw.len() as u64<=MAX_IMAGE&&b["sha256"]==h,"Image hash differs");
    let format=image::guess_format(&raw)?;
    ensure!(matches!(format,image::ImageFormat::Png|image::ImageFormat::Jpeg),"Use actual PNG/JPEG image");
    let (w,h)=image::ImageReader::with_format(std::io::Cursor::new(&raw),format).into_dimensions()?;
    ensure!(w>0&&h>0&&u64::from(w)*u64::from(h)<=20_000_000,"Image dimensions exceed budget");
    let mime=if format==image::ImageFormat::Png{"image/png"}else{"image/jpeg"};
    Ok((b,format!("data:{mime};base64,{}",STANDARD.encode(raw))))
}
fn prompt(category:&str)->Result<&'static str>{
    Ok(match category{"text"=>"Text Recognition:","table"=>"Table Recognition:","formula"=>"Formula Recognition:",_=>bail!("Explicit text/table/formula region required")})
}
fn recognition(v:&Value)->Result<Value>{
    let choices=v["choices"].as_array().filter(|a|a.len()==1).context("Expected one actual OCR response")?;
    let row=&choices[0];let reading=text(&row["message"]["content"],"Actual OCR text")?;
    ensure!(reading.len()<=2*1024*1024,"Recognition output exceeds budget");
    let finish=text(&row["finish_reason"],"Finish reason")?;
    ensure!(matches!(finish,"stop"|"length"),"Unknown OCR completion state");
    Ok(json!({"candidate_text":reading,"finish_reason":finish,"truncated":finish=="length","usage":v["usage"]}))
}
fn probabilities(v:&Value)->Result<Value>{
    let p=&v["answers"]["consistency"]["probabilities"];
    let zero=p["0"].as_f64().context("Missing Jev class 0")?;
    let one=p["1"].as_f64().context("Missing Jev class 1")?;
    ensure!(zero.is_finite()&&one.is_finite()&&(0. ..=1.).contains(&zero)&&(0. ..=1.).contains(&one)&&(zero+one-1.).abs()<=0.005,"Invalid actual Jev distribution");
    Ok(json!({"prob_0":zero,"prob_1":one,"usage":v["usage"],"softmax_applied_by_backend":true}))
}
fn attempts(p:&Package,acceleration:&str,timeout:Duration,route:&str,body:&Value,parse:fn(&Value)->Result<Value>)->Result<Value>{
    ensure!(matches!(acceleration,"auto"|"cpu"|"gpu_required"),"acceleration must be auto/cpu/gpu_required");
    let mut log=vec![];
    let mut modes=vec![];
    if acceleration!="cpu"{
        if let Some(gpu)=&p.gpu{modes.push((gpu,true));}
        else{
            log.push(json!({"mode":"gpu","status":"unavailable","reason":p.gpu_error}));
            if acceleration=="gpu_required"{return Ok(json!({"status":"blocked","attempts":log,"result":null,"execution_mode":null}));}
        }
    }
    if acceleration!="gpu_required"{modes.push((&p.cpu,false));}
    for(e,gpu)in modes{
        let started=Instant::now();
        let outcome=(||->Result<Value>{
            let server=Server::start(p,e,gpu,timeout)?;
            let response=request(server.addr,&server.token,route,Some(body),timeout)?;
            let result=parse(&response)?;
            let offload=server.offload;
            drop(server);p.verify_models()?;e.verify()?;
            Ok(json!({"result":result,"runtime":p.report(e),"gpu_offload_reported_layers":offload}))
        })();
        let mode=if gpu{"gpu"}else{"cpu"};
        match outcome{
            Ok(mut r)=>{
                log.push(json!({"mode":mode,"variant":e.variant,"status":"completed","elapsed_seconds":started.elapsed().as_secs_f64()}));
                r["attempts"]=json!(log);r["execution_mode"]=json!(mode);
                r["fallback_used"]=json!(!gpu&&acceleration=="auto");return Ok(r)
            }
            Err(e)=>{
                let mut attempt=json!({"mode":mode,"status":"failed","reason":format!("{e:#}"),"elapsed_seconds":started.elapsed().as_secs_f64()});
                if let Some(d)=e.downcast_ref::<StartupDiagnostic>(){attempt["startup_diagnostic"]=d.json();}
                if let Some(d)=e.downcast_ref::<MemoryBudgetError>(){attempt["memory_budget"]=d.json();}
                log.push(attempt);
            },
        }
    }
    Ok(json!({"status":"blocked","attempts":log,"result":null,"execution_mode":null}))
}
pub fn recognize(png:&Path,sha:&str,manifest:&Path,msha:&str,category:&str,acceleration:&str,timeout:u64)->Result<Value>{
    ensure!((5..=1800).contains(&timeout),"Invalid inference timeout");
    let p=Package::load(manifest,msha,"minicpm-ocr")?;
    let (img,url)=image(png,sha)?;
    let body=json!({"model":"z2o-minicpm","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":url}},{"type":"text","text":prompt(category)?}]}],
        "temperature":0,"max_tokens":4096,"stream":false});
    let mut r=attempts(&p,acceleration,Duration::from_secs(timeout),"/v1/chat/completions",&body,recognition)?;
    verify(&img)?;
    r["schema_version"]=json!(1);r["source_image"]=img;r["category"]=json!(category);
    if r["status"]!="blocked"{r["status"]=json!(if r["result"]["truncated"]==true{"truncated_review_required"}else{"minicpm_candidate_review_required"});}
    r["source_fidelity"]=json!("not_assessed");r["acceptance_ready"]=json!(false);r["boundary"]=json!(BOUNDARY);Ok(r)
}
pub fn score(original:&Path,original_sha:&str,render:&Path,render_sha:&str,candidate:&Path,candidate_sha:&str,
    manifest:&Path,msha:&str,category:&str,acceleration:&str,timeout:u64)->Result<Value>{
    prompt(category)?;ensure!((5..=1800).contains(&timeout),"Invalid inference timeout");
    let p=Package::load(manifest,msha,"jev")?;
    let (source_image,url)=image(original,original_sha)?;
    let (rendered,render_url)=self::image(render,render_sha)?;
    check_write::no_link(candidate)?;ensure!(fs::metadata(candidate)?.len()<=1024*1024,"Candidate text exceeds budget");
    let(raw,b)=files::read_bound(candidate)?;ensure!(raw.len()<=1024*1024&&b["sha256"]==candidate_sha,"Candidate text changed");
    let reading=std::str::from_utf8(&raw)?;ensure!(!reading.trim().is_empty(),"Empty candidate text");
    // Exact official llama.cpp Jev request: original first, render second;
    // no extra instructions or second softmax.
    let body=json!({"state":reading,"files":[url,render_url],"questions":{"consistency":{"type":"choice","criteria":{"0":null,"1":null}}}});
    let mut r=attempts(&p,acceleration,Duration::from_secs(timeout),"/v1/systemone",&body,probabilities)?;
    verify(&source_image)?;verify(&rendered)?;verify(&b)?;
    r["schema_version"]=json!(1);r["source_image"]=source_image;r["candidate_render"]=rendered;r["candidate_text_file"]=b;
    r["category"]=json!(category);r["render_semantics_verified"]=json!(false);
    if r["status"]!="blocked"{r["status"]=json!("jev_score_review_required");}
    r["source_fidelity"]=json!("not_assessed");r["acceptance_ready"]=json!(false);r["boundary"]=json!(BOUNDARY);Ok(r)
}
#[cfg(test)]
mod tests{
    #[test]
    fn model_memory_diagnostics_preserve_cpu_and_gpu_reservation(){
        let weights=1_430_124_352;
        let required=3_577_608_000;
        assert!(require_model_memory(required,weights,false).is_ok());
        let err=require_model_memory(required-1,weights,false).unwrap_err();
        let detail=err.downcast_ref::<MemoryBudgetError>().unwrap();
        assert_eq!(detail.json(),json!({"available_bytes":required-1,"required_bytes":required,
            "host_weight_bytes":weights,"reserve_bytes":MODEL_MEMORY_RESERVE}));
        assert!(require_model_memory(MODEL_MEMORY_RESERVE,weights,true).is_ok());
        let err=require_model_memory(MODEL_MEMORY_RESERVE-1,weights,true).unwrap_err();
        assert_eq!(err.downcast_ref::<MemoryBudgetError>().unwrap().json()["host_weight_bytes"],0);
        assert!(require_model_memory(0,weights,false).is_err());
        assert!(require_model_memory(u64::MAX-1,u64::MAX,false).is_err());
    }

    use super::*;
    #[test]
    fn jev_uses_actual_class_one_and_never_softmaxes_twice(){
        let r=json!({"answers":{"consistency":{"probabilities":{"1":0.91,"0":0.09}}}});
        assert_eq!(probabilities(&r).unwrap()["prob_1"],0.91);
        assert!(probabilities(&json!({"answers":{"consistency":{"probabilities":{"0":0.8,"1":0.8}}}})).is_err());
        assert!(probabilities(&json!({"answers":{"consistency":{"probabilities":{"1":1.0}}}})).is_err());
    }
    #[test]
    fn preserve_truncated_ocr_without_approval(){
        let r=json!({"choices":[{"finish_reason":"length","message":{"content":"actual candidate"}}]});
        assert_eq!(recognition(&r).unwrap()["truncated"],true);
    }
    #[test]
    fn bounded_diagnostics_preserve_fixed_signatures_not_log_text(){
        let dir=tempfile::tempdir().unwrap();let p=dir.path().join("server.log");
        let mut bytes=b"/private/path/model: GLIBC_2.38 version not found (--api-key DO_NOT_RETURN)\n".to_vec();
        bytes.extend_from_slice(&[b'x';40*1024]);
        bytes.extend_from_slice(b"\xff\nmtmd_init_from_file: error; failed to load projector\nBearer NEVER_RETURN\n");
        fs::write(&p,&bytes).unwrap();
        let diag=StartupDiagnostic::collect(&mut fs::File::open(&p).unwrap(),"startup_timeout",None);
        let out=diag.json();
        assert_eq!(out["log_bytes_sampled"],32*1024);
        assert_eq!(out["log_sample_truncated"],true);
        assert_eq!(out["signatures"],json!(["loader_abi_version_missing","projector_load_failed"]));
        let evidence=format!("{diag} {out}");
        for secret in ["/private/path","DO_NOT_RETURN","NEVER_RETURN","--api-key","GLIBC_2.38"]{
            assert!(!evidence.contains(secret));
        }
        let mut write_only=fs::File::create(dir.path().join("unreadable")).unwrap();
        write_only.write_all(b"data").unwrap();
        let unreadable=StartupDiagnostic::collect(&mut write_only,"startup_timeout",None);
        assert_eq!(unreadable.json()["log_readable"],false);
        assert!(unreadable.signatures.is_empty());
        assert_eq!(startup_signatures("failed to init decision model: unsupported decision model type: private-label"),
            ["unsupported_decision_model"]);
    }
    #[cfg(any(target_os="linux",all(target_os="macos",target_arch="aarch64")))]
    #[test]
    fn public_jev_startup_failure_reports_closed_codes_without_child_args(){
        use std::os::unix::fs::PermissionsExt;
        let dir=tempfile::tempdir().unwrap();let root=dir.path();
        let engine=root.join("engine");fs::create_dir(&engine).unwrap();
        let exe=engine.join("llama-server");
        let program=r#"#!/bin/sh
printf '%s\n' "$*" >&2
after_key_flag=""
for arg in "$@"; do
    if [ "$after_key_flag" = "yes" ]; then
        IFS= read -r actual_key < "$arg"
        printf 'Actual key: %s\n' "$actual_key" >&2
        break
    fi
    if [ "$arg" = "--api-key-file" ]; then after_key_flag=yes; fi
done
printf '%s\n' 'Authorization: Bearer LEAK_MARKER' 'error loading model: unknown model architecture PRIVATE_ARCH' >&2
exit 19
"#;
        fs::write(&exe,program).unwrap();fs::set_permissions(&exe,fs::Permissions::from_mode(0o700)).unwrap();
        let model="MiniCPM-OCR-Jev-Q4_K_M.gguf";let projector="mmproj-model-f16.gguf";
        fs::write(root.join(model),b"GGUFunit-test-model").unwrap();
        fs::write(root.join(projector),b"GGUFunit-test-projector").unwrap();
        let sum=|p:&Path|files::digest(&fs::read(p).unwrap());
        let manifest=root.join("package.json");
        fs::write(&manifest,serde_json::to_vec(&json!({"schema_version":1,"engine":"llama.cpp","release":JEV_LLAMA_RELEASE,
            "model_kind":"jev","platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,
            "cpu":{"directory":"engine","variant":"cpu","executable":"llama-server","files":[{"path":"llama-server","sha256":sum(&exe)}]},
            "model":{"path":model,"sha256":sum(&root.join(model))},
            "projector":{"path":projector,"sha256":sum(&root.join(projector))}})).unwrap()).unwrap();
        let source=root.join("source.png");let render=root.join("render.png");let text=root.join("candidate.txt");
        image::GrayImage::from_raw(1,1,vec![150]).unwrap().save(&source).unwrap();
        image::GrayImage::from_raw(1,1,vec![80]).unwrap().save(&render).unwrap();
        fs::write(&text,b"Candidate content not a startup diagnostic").unwrap();
        let report=crate::invoke(json!({"schema_version":1,"operation":"prepare_source_inventory.score_jev_region",
            "args":[source],"kwargs":{"expected_image_sha256":sum(&source),
                "render_image_path":render,"expected_render_sha256":sum(&render),
                "candidate_text_path":text,"expected_candidate_sha256":sum(&text),
                "package_manifest":manifest,"expected_package_sha256":sum(&manifest),
                "category":"text","acceleration":"auto","timeout_seconds":5}})).unwrap();
        let report=&report["result"];
        assert_eq!(report["status"],"blocked");
        assert_eq!(report["acceptance_ready"],false);
        assert_eq!(report["source_fidelity"],"not_assessed");
        assert!(report["result"].is_null());
        assert_eq!(report["attempts"][0]["status"],"unavailable");
        let attempt=&report["attempts"][1];
        assert_eq!(attempt["status"],"failed");
        assert_eq!(attempt["startup_diagnostic"]["phase"],"early_exit","{attempt}");
        assert_eq!(attempt["startup_diagnostic"]["exit_code"],19);
        assert_eq!(attempt["startup_diagnostic"]["signatures"],json!(["unsupported_model_architecture","model_load_failed"]));
        let diagnostic=attempt.to_string();
        for secret in ["LEAK_MARKER","PRIVATE_ARCH","--api-key-file",model,projector,"key.txt","Candidate content","Actual key:"]{
            assert!(!diagnostic.contains(secret));
        }
        assert!(!regex::Regex::new("[0-9a-f]{64}").unwrap().is_match(&diagnostic));
    }
    #[test]
    fn known_runtime_release_is_bound_to_model_kind_before_launch(){
        assert_eq!(release_for("minicpm-ocr").unwrap(),"b11901");
        let jev=release_for("jev");
        if jev.is_err(){assert!(format!("{jev:?}").contains("JEV_RUNTIME_PLATFORM_UNSUPPORTED"));return;}
        assert_eq!(jev.unwrap(),"b11928");
        let dir=tempfile::tempdir().unwrap();
        let manifest=dir.path().join("package.json");
        for (kind,release) in [("jev",LLAMA_RELEASE),("minicpm-ocr",JEV_LLAMA_RELEASE)]{
            let payload=serde_json::to_vec(&json!({"schema_version":1,"engine":"llama.cpp","release":release,
                "model_kind":kind,"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH})).unwrap();
            fs::write(&manifest,&payload).unwrap();
            assert!(format!("{:#}",inspect(&manifest,&files::digest(&payload),kind).unwrap_err())
                .contains("MINICPM_RUNTIME_RELEASE_MISMATCH"));
        }
    }
    #[test]
    fn gpu_offload_requires_positive_actual_layer_count(){
        let dir=tempfile::tempdir().unwrap();let p=dir.path().join("server.log");
        fs::write(&p,"load_tensors: offloaded 0/25 layers to GPU\n").unwrap();
        assert_eq!(log_offload(&p).unwrap(),Some((0,25)));
        fs::write(&p,"load_tensors: offloaded 25/25 layers to GPU\n").unwrap();
        assert_eq!(log_offload(&p).unwrap(),Some((25,25)));
    }
}


pub fn inspect(manifest:&Path,sha:&str,kind:&str)->Result<Value>{
    let p=Package::load(manifest,sha,kind)?;
    Ok(json!({"manifest":p.binding,"cpu":p.report(&p.cpu),"gpu":p.gpu.as_ref().map(|e|p.report(e)),
        "gpu_error":p.gpu_error,"runtime_loaded":false,"qualified":false,"boundary":BOUNDARY}))
}
pub struct RegionOptions<'a>{
    pub page:u64,pub bbox:&'a Value,pub coordinates:&'a Value,pub dpi:u64,pub source_sha:&'a str,
    pub manifest:&'a Path,pub manifest_sha:&'a str,pub category:&'a str,pub acceleration:&'a str,pub timeout:u64,
}
pub fn recognize_pdf_region(source:&Path,destination:&Path,opt:RegionOptions<'_>)->Result<Value>{
    ensure!(*opt.coordinates==json!({"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"}),"Explicit unrotated PDF-point coordinates required");
    ensure!((72..=600).contains(&opt.dpi),"Invalid region dpi");
    let b=opt.bbox.as_array().filter(|a|a.len()==4).context("Four finite bbox points required")?;
    let mut bbox=[0.;4];for(i,n)in b.iter().enumerate(){bbox[i]=n.as_f64().filter(|n|n.is_finite()).context("Invalid bbox")?;}
    ensure!(bbox[0]<bbox[2]&&bbox[1]<bbox[3],"Empty bbox");
    let mut pdf=crate::pdfcraft_backend::Backend::open(source,Some(opt.source_sha))?;
    fs::create_dir_all(destination)?;
    let root=files::canonical(destination)?;
    ensure!(no_link(&root)?.is_dir(),"Actual evidence directory required");
    let attempt=tempfile::Builder::new().prefix("minicpm-region-").tempdir_in(&root)?.keep();
    let png=attempt.join("region.png");let render=pdf.render(opt.page,Some(bbox),opt.dpi,&png)?;
    let r=recognize(&png,hash(&render["asset"]["sha256"])?,opt.manifest,opt.manifest_sha,opt.category,opt.acceleration,opt.timeout);
    pdf.verify()?;
    let mut r=r.unwrap_or_else(|e|json!({"schema_version":1,"status":"minicpm_blocked","reason":format!("{e:#}"),"source_fidelity":"not_assessed","acceptance_ready":false}));
    r["source_pdf"]=pdf.source_binding().clone();r["page"]=json!(opt.page);r["bbox"]=opt.bbox.clone();
    r["coordinate_system"]=opt.coordinates.clone();r["render"]=render;
    let b=crate::runtime_reports::save_new_report(&attempt.join("recognition.json"),&r)?;
    r["record"]=b;Ok(r)
}
