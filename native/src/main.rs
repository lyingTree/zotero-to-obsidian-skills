use anyhow::{anyhow,ensure,Result};
use serde_json::json;
use std::{env,fs,io::{self,Write},path::PathBuf,process::ExitCode};
use z2o_native::{describe,invoke,parse_json,ENGINE_VERSION};

fn run()->Result<serde_json::Value>{
    let mut args=env::args_os().skip(1);
    let cmd=args.next().ok_or_else(||anyhow!("use describe, invoke <request.json>, or --version"))?;
    match cmd.to_str(){
        Some("describe")=>{ensure!(args.next().is_none(),"unexpected argument");Ok(describe())}
        Some("--version")=>{ensure!(args.next().is_none(),"unexpected argument");Ok(json!({"engine":"rust-native","version":ENGINE_VERSION}))}
        Some("invoke")=>{
            let p=PathBuf::from(args.next().ok_or_else(||anyhow!("request file required"))?);
            ensure!(args.next().is_none(),"unexpected argument");
            invoke(parse_json(&fs::read_to_string(p)?)?)
        }
        _=>Err(anyhow!("unsupported command; no Python fallback"))
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
