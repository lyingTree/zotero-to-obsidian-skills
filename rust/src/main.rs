use anyhow::{anyhow,ensure,Result};
use serde_json::json;
use std::{env,fs,io::{self,Read,Write},path::PathBuf,process::ExitCode};
use z2o_native::{describe,describe_operation,files,invoke,parse_json,path_text,pipeline,schema,ENGINE_VERSION,native_install,runtime_reports};

fn read_stdin(max_bytes:usize)->Result<String>{
    let mut buf=Vec::new();
    io::stdin().take((max_bytes as u64).saturating_add(1)).read_to_end(&mut buf)?;
    ensure!(buf.len()<=max_bytes,"stdin request exceeds {} byte budget",max_bytes);
    Ok(String::from_utf8(buf)?)
}
fn usage_json()->serde_json::Value{
    json!({
        "schema_version":1,
        "engine":"rust",
        "version":format!("v{ENGINE_VERSION}"),
        "commands":{
            "describe":"z2o describe                  List runtime identity and all registered operation names",
            "describe_op":"z2o describe <operation>      Show positional args, kwargs, and request_template for one operation",
            "describe_all":"z2o describe --all            Include schemas for every registered operation",
            "invoke_file":"z2o invoke <request.json> [--output <response.json>]     Execute an operation from a UTF-8 JSON file",
            "invoke_stdin":"z2o invoke - [--output <response.json>]                  Execute an operation reading UTF-8 JSON from stdin",
            "package":"z2o package <source_root> <destination> [--ocr-bundle <ocr_dir>]",
            "install":"z2o install <package.json> <trusted_sha256> <skills_root>",
            "verify_install":"z2o verify-install <package.json> <trusted_sha256> <skills_root>",
            "version":"z2o --version",
            "help":"z2o --help"
        },
        "example_request":{"schema_version":1,"operation":"zotero_local.search_items","args":["user",0,"topology"],"kwargs":{"limit":5}}
    })
}
fn invoke_help_json()->serde_json::Value{
    json!({
        "schema_version":1,
        "command":"z2o invoke <request.json | -> [--output <response.json>]",
        "stdin_supported":true,
        "output_flag":"--output <file> (or -o <file>) writes full pretty JSON to <file> and prints only a compact receipt to stdout.",
        "request_format":{"schema_version":1,"operation":"<registered operation name>","args":["<positional args...>"],"kwargs":{"<keyword arg>":"<value>"}},
        "discover_operations":"Run `z2o describe` for all operation names, or `z2o describe <operation>` for exact parameters."
    })
}
fn describe_help_json()->serde_json::Value{
    json!({
        "schema_version":1,
        "command":"z2o describe [operation | --all]",
        "modes":{
            "no_args":"Return runtime version, capabilities, and list of all registered operation names.",
            "<operation>":"Return summary, args, kwargs, and request_template for one operation.",
            "--all":"Return standard describe payload plus operation_schemas map for every operation."
        }
    })
}
fn run()->Result<serde_json::Value>{
    let mut args=env::args_os().skip(1);
    let cmd=args.next().ok_or_else(||anyhow!("use describe [operation|--all], invoke <request.json|->, package <source_root> <destination> [--ocr-bundle <ocr_dir>], install <package.json> <trusted_sha256> <skills_root>, verify-install <package.json> <trusted_sha256> <skills_root>, --version, or --help"))?;
    match cmd.to_str(){
        Some("--help"|"-h"|"help")=>{
            ensure!(args.next().is_none(),"unexpected argument; run `z2o --help` for usage");
            Ok(usage_json())
        }
        Some("describe")=>{
            let Some(arg)=args.next() else {return Ok(describe());};
            ensure!(args.next().is_none(),"unexpected argument; usage: z2o describe [operation | --all]");
            match arg.to_str(){
                Some("--help"|"-h")=>Ok(describe_help_json()),
                Some("--all")=>{
                    let mut out=describe();
                    if let Some(m)=out.as_object_mut(){m.insert("operation_schemas".into(),schema::all_schemas());}
                    Ok(out)
                }
                Some(op)=>describe_operation(op),
                None=>Err(anyhow!("operation name must be valid UTF-8")),
            }
        }
        Some("--version")=>{ensure!(args.next().is_none(),"unexpected argument");Ok(json!({"engine":"rust","version":format!("v{ENGINE_VERSION}"),"engine_version":ENGINE_VERSION}))}
        Some("package")=>{
            let rest:Vec<_>=args.collect();
            let mut pos:Vec<PathBuf>=Vec::new();
            let mut ocr_bundle:Option<PathBuf>=None;
            let mut i=0usize;
            while i<rest.len(){
                let cur=&rest[i];
                if cur.to_str()==Some("--ocr-bundle"){
                    ensure!(ocr_bundle.is_none(),"duplicate --ocr-bundle flag");
                    let val=rest.get(i+1).ok_or_else(||anyhow!("--ocr-bundle requires an OCR package directory"))?;
                    ocr_bundle=Some(PathBuf::from(val));
                    i+=2;
                    continue;
                }
                if let Some(val)=cur.to_str().and_then(|s|s.strip_prefix("--ocr-bundle=")){
                    ensure!(ocr_bundle.is_none(),"duplicate --ocr-bundle flag");
                    ensure!(!val.is_empty(),"--ocr-bundle requires a nonempty directory path");
                    ocr_bundle=Some(PathBuf::from(val));
                    i+=1;
                    continue;
                }
                ensure!(cur.to_str().is_none_or(|s|!s.starts_with('-')),"unexpected option in package; usage: z2o package <source_root> <destination> [--ocr-bundle <ocr_dir>]");
                pos.push(PathBuf::from(cur));
                i+=1;
            }
            ensure!(pos.len()==2,"usage: z2o package <source_root> <destination> [--ocr-bundle <ocr_dir>]");
            native_install::package_with_ocr(&pos[0],&pos[1],ocr_bundle.as_deref(),runtime_reports::progress_line)
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
            let rest:Vec<_>=args.collect();
            if rest.is_empty(){
                return Err(anyhow!("request file or '-' (stdin) required; run `z2o invoke --help`"));
            }
            if rest.len()==1 && matches!(rest[0].to_str(),Some("--help"|"-h")){
                return Ok(invoke_help_json());
            }
            let mut target:Option<std::ffi::OsString>=None;
            let mut output_path:Option<PathBuf>=None;
            let mut i=0usize;
            while i<rest.len(){
                let cur=&rest[i];
                if matches!(cur.to_str(),Some("--output"|"-o")){
                    ensure!(output_path.is_none(),"duplicate --output flag");
                    let val=rest.get(i+1).ok_or_else(||anyhow!("--output requires a destination file path"))?;
                    output_path=Some(PathBuf::from(val));
                    i+=2;
                    continue;
                }
                if let Some(val)=cur.to_str().and_then(|s|s.strip_prefix("--output=")){
                    ensure!(output_path.is_none(),"duplicate --output flag");
                    ensure!(!val.is_empty(),"--output requires a nonempty destination file path");
                    output_path=Some(PathBuf::from(val));
                    i+=1;
                    continue;
                }
                ensure!(cur.to_str().is_none_or(|s|s=="-"||!s.starts_with('-')),"unexpected option in invoke; usage: z2o invoke <request.json | -> [--output <response.json>]");
                ensure!(target.is_none(),"unexpected positional argument; usage: z2o invoke <request.json | -> [--output <response.json>]");
                target=Some(cur.clone());
                i+=1;
            }
            let target=target.ok_or_else(||anyhow!("request file or '-' (stdin) required; run `z2o invoke --help`"))?;
            let text=if target.to_str()==Some("-"){
                read_stdin(64*1024*1024)?
            }else{
                fs::read_to_string(PathBuf::from(&target))?
            };
            let result=invoke(parse_json(&text)?)?;
            if let Some(out_p)=output_path{
                let name=out_p.file_name().ok_or_else(||anyhow!("--output requires a file name"))?;
                let parent=out_p.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or_else(||std::path::Path::new("."));
                fs::create_dir_all(parent)?;
                let dest=files::canonical(parent)?.join(name);
                let mut bytes=serde_json::to_vec_pretty(&result)?;
                bytes.push(b'\n');
                pipeline::publish_replace(&dest,&bytes)?;
                let snap=files::snapshot(&dest)?;
                Ok(json!({"schema_version":1,"status":"saved_output","output":{"path":path_text(&dest)?,"sha256":snap["sha256"],"bytes":snap["size"]}}))
            }else{
                Ok(result)
            }
        }
        _=>Err(anyhow!("unsupported command; run `z2o --help` for usage"))
    }
}
fn main()->ExitCode{
    struct RuntimeTimer(std::time::Instant);
    impl Drop for RuntimeTimer {
        fn drop(&mut self) {
            let elapsed_ms = self.0.elapsed().as_millis();
            let _ = writeln!(io::stderr().lock(), "{{\"event\":\"z2o_elapsed\",\"elapsed_ms\":{elapsed_ms}}}");
        }
    }
    let _timer = RuntimeTimer(std::time::Instant::now());
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
