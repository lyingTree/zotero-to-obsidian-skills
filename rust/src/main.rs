use anyhow::{anyhow,ensure,Result};
use serde_json::json;
use std::{env,fs,io::{self,Write},path::PathBuf,process::ExitCode};
use z2o_native::{describe,invoke,parse_json,ENGINE_VERSION,native_install,runtime_reports};

fn run()->Result<serde_json::Value>{
    let mut args=env::args_os().skip(1);
    let cmd=args.next().ok_or_else(||anyhow!("use describe, invoke <request.json>, package <source_root> <destination>, install <package.json> <trusted_sha256> <skills_root>, verify-install <package.json> <trusted_sha256> <skills_root>, or --version"))?;
    match cmd.to_str(){
        Some("describe")=>{ensure!(args.next().is_none(),"unexpected argument");Ok(describe())}
        Some("--version")=>{ensure!(args.next().is_none(),"unexpected argument");Ok(json!({"engine":"rust","version":format!("v{ENGINE_VERSION}"),"engine_version":ENGINE_VERSION}))}
        Some("package")=>{
            let source=PathBuf::from(args.next().ok_or_else(||anyhow!("source root required"))?);
            let dest=PathBuf::from(args.next().ok_or_else(||anyhow!("new package destination required"))?);
            ensure!(args.next().is_none(),"unexpected argument");
            native_install::package(&source,&dest,runtime_reports::progress_line)
        }
        Some("install"|"verify-install")=>{
            let manifest=PathBuf::from(args.next().ok_or_else(||anyhow!("package manifest required"))?);
            let sha=args.next().ok_or_else(||anyhow!("independently trusted manifest SHA-256 required"))?;
            let sha=sha.to_str().ok_or_else(||anyhow!("manifest hash must be ASCII"))?;
            let root=PathBuf::from(args.next().ok_or_else(||anyhow!("skills root required"))?);
            ensure!(args.next().is_none(),"unexpected argument");
            if cmd=="verify-install" {native_install::verify_install(&manifest,sha,&root)}
            else {native_install::install(&manifest,sha,&root,runtime_reports::progress_line)}
        }
        Some("ocr-provider-worker")=>{
            let p=PathBuf::from(args.next().ok_or_else(||anyhow!("worker request file required"))?);
            ensure!(args.next().is_none()&&fs::metadata(&p)?.len()<=2*1024*1024,"Invalid worker request");
            z2o_native::ocr_acceleration::worker(&parse_json(&fs::read_to_string(p)?)?)
        }
        Some("invoke")=>{
            let p=PathBuf::from(args.next().ok_or_else(||anyhow!("request file required"))?);
            ensure!(args.next().is_none(),"unexpected argument");
            invoke(parse_json(&fs::read_to_string(p)?)?)
        }
        _=>Err(anyhow!("unsupported command"))
    }
}
fn main()->ExitCode{
    let result=run();
    match result{
        Ok(v)=>{
            let stdout=io::stdout();
            let mut out=stdout.lock();
            if serde_json::to_writer(&mut out,&v).is_err() || writeln!(out).is_err(){return ExitCode::FAILURE;}
            ExitCode::SUCCESS
        }
        Err(e)=>{
            let stderr=io::stderr();let mut out=stderr.lock();
            let _=serde_json::to_writer(&mut out,&json!({"schema_version":1,"error":"native_operation_failed","message":format!("{e:#}")}));
            let _=writeln!(out);
            ExitCode::FAILURE
        }
    }
}
