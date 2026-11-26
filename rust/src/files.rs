use crate::{decision, parse_json, path, path_text};
use anyhow::{anyhow, ensure, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs::{self,File}, io::Read, path::{Path,PathBuf}};

pub fn digest(data:&[u8])->String { format!("{:x}",Sha256::digest(data)) }
pub fn canonical(p:&Path)->Result<PathBuf> {
    dunce::canonicalize(p).with_context(||format!("resolve {}",p.display()))
}
pub fn snapshot(p:&Path)->Result<Value> {snapshot_impl(p,None)}
/// Hash large inference weights without retaining verification reads in the
/// Darwin filesystem cache. The digest and file-stability checks are identical.
pub(crate) fn snapshot_model_weight(p:&Path,max:u64)->Result<Value> {snapshot_impl(p,Some(max))}

#[cfg(target_os="macos")]
fn hint_uncached(file:&File){
    use std::os::fd::AsRawFd;
    // SAFETY: the File owns a live descriptor throughout this call. Darwin
    // F_NOCACHE takes an integer, with 1 disabling data caching. Unsupported
    // filesystems can ignore the hint; the full SHA still runs.
    let _=unsafe {libc::fcntl(file.as_raw_fd(),libc::F_NOCACHE,1 as libc::c_int)};
}

fn snapshot_impl(p:&Path,model_limit:Option<u64>)->Result<Value> {
    let p=canonical(p)?;
    ensure!(fs::metadata(&p)?.is_file(),"not a regular file");
    let mut file=File::open(&p)?;
    let before=file.metadata()?;
    ensure!(before.is_file(),"not a regular file");
    if let Some(limit)=model_limit{
        ensure!(before.len()<=limit,"Model asset exceeds size budget");
        #[cfg(target_os="macos")]
        hint_uncached(&file);
    }
    let cap=before.len().clamp(4096,1024*1024) as usize;
    let mut buffer=vec![0u8;cap];
    let mut hasher=Sha256::new();
    let mut size=0u64;
    loop {
        let n=file.read(&mut buffer)?;
        if n==0 {break;}
        size+=n as u64;
        ensure!(model_limit.is_none_or(|limit|size<=limit),"Model asset exceeds size budget");
        hasher.update(&buffer[..n]);
    }
    let after=file.metadata()?;
    ensure!(size==before.len() && size==after.len() && before.modified().ok()==after.modified().ok(),
        "file changed while reading");
    Ok(json!({"path":path_text(&p)?,"sha256":format!("{:x}",hasher.finalize()),"size":size}))
}
pub fn read_bound(p:&Path)->Result<(Vec<u8>,Value)> {
    let p=canonical(p)?;
    ensure!(fs::metadata(&p)?.is_file(),"not a regular file");
    let data=fs::read(&p)?;
    let binding=json!({"path":path_text(&p)?,"sha256":digest(&data)});
    Ok((data,binding))
}
pub fn verify_binding(record:&Value,actual:&Path,base:&Path)->Result<Value> {
    let current=snapshot(actual)?;
    let Some(raw)=record.get("path").and_then(Value::as_str).filter(|v|!v.is_empty()) else {
        return Ok(json!({"matches":false,"reason":"missing recorded file path","actual":current}));
    };
    let mut bound=PathBuf::from(raw);
    if !bound.is_absolute(){bound=base.join(bound);}
    let same_path=canonical(&bound).map(|p|path_text(&p).ok()==current["path"].as_str().map(str::to_owned)).unwrap_or(false);
    let same_hash=record.get("sha256")==current.get("sha256");
    Ok(json!({"matches":same_path&&same_hash,"same_path":same_path,"same_hash":same_hash,"actual":current}))
}

fn local_path(root:&Path,raw:&str)->Result<PathBuf>{
    ensure!(!raw.is_empty()&&!raw.contains(['\\','\0']),"invalid vault-relative POSIX path");
    ensure!(!raw.split('/').any(|part|part.is_empty()||part=="."||part==".."||part.contains(':')),
        "absolute, traversing or noncanonical relative path");
    let mut target=root.to_path_buf();
    for part in raw.split('/'){
        if cfg!(windows){
            ensure!(!part.ends_with(['.',' '])&&!part.contains(['<','>','"','|','?','*']),"ambiguous Windows filename segment");
            let stem=part.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
            let reserved=["CON","PRN","AUX","NUL","CONIN$","CONOUT$"].contains(&stem.as_str()) ||
                stem.strip_prefix("COM").or_else(||stem.strip_prefix("LPT")).is_some_and(|n|n.chars().count()==1&&"123456789¹²³".contains(n));
            ensure!(!reserved,"Windows device name is not a vault resource");
        }
        target.push(part);
    }
    let target=canonical(&target)?;
    ensure!(target.starts_with(root),"target escapes the resource root");
    Ok(target)
}
pub fn inspect_relative_files(root:&Path,values:&Value)->Result<Value>{
    let root=canonical(root)?;
    ensure!(root.is_dir(),"resource root must be a directory");
    let values=values.as_array().ok_or_else(||anyhow!("provide explicit local relative paths"))?;
    let mut result=Vec::with_capacity(values.len());
    let mut cache:HashMap<PathBuf,Value>=HashMap::new();
    for (i, value) in values.iter().enumerate(){
        let row=(||->Result<Value>{
            let raw=value.as_str().ok_or_else(||anyhow!("invalid vault-relative POSIX path"))?;
            let target=local_path(&root,raw)?;
            let entry=if let Some(entry)=cache.get(&target){entry.clone()}else{
                let entry=snapshot(&target)?; cache.insert(target,entry.clone()); entry
            };
            let mut row=entry.as_object().unwrap().clone();
            row.insert("relative_path".into(),json!(raw));
            row.insert("status".into(),json!(if entry["size"]==json!(0){"empty"}else{"readable"}));
            Ok(Value::Object(row))
        })().unwrap_or_else(|e|json!({"relative_path":value,"status":"blocked","reason":e.to_string()}));
        result.push(row);
        if (i+1)%128==0 || i+1==values.len(){eprintln!("[{}/{}] relative_files",i+1,values.len());}
    }
    Ok(json!(result))
}
pub fn audit_translation_binding(record:&Path,actual:&Path,kind:&str)->Result<Value>{
    let (id,keys):(&str,&[&str])=match kind{
        "glossary"=>("TRN-A17",&["glossary"]),
        "candidate"=>("TRN-A25",&["bindings","candidate"]),
        _=>anyhow::bail!("kind must be glossary or candidate")
    };
    let (raw,record_binding)=read_bound(record)?;
    let record_path=path(&record_binding["path"])?;
    let checked=(||->Result<Value>{
        let doc=parse_json(std::str::from_utf8(&raw)?)?;
        let mut field=&doc;
        for key in keys {field=field.get(*key).ok_or_else(||anyhow!("missing record field: {key}"))?;}
        verify_binding(field,actual,record_path.parent().context("record parent")?)
    })();
    let actual=snapshot(actual)?;
    let (value,reason)=match checked {
        Ok(v) if v["actual"]==actual=>(v["matches"].as_bool(),"recorded path and SHA-256 compared to independently chosen actual file".to_owned()),
        Ok(_)=>(None,"actual file changed during comparison".to_owned()),
        Err(e)=>(None,e.to_string()),
    };
    Ok(json!({"stage":"translation","inputs":[record_binding,{"path":actual["path"],"sha256":actual["sha256"]}],
        "checks":{(id):decision(id,value,&reason)},"source_fidelity":"not_assessed"}))
}
pub fn prepend_env_path(var:&str,dir:&Path)->std::ffi::OsString{
    let mut paths=vec![dir.to_path_buf()];
    if let Some(existing)=std::env::var_os(var).filter(|v|!v.is_empty()){
        paths.extend(std::env::split_paths(&existing).filter(|p|!p.as_os_str().is_empty()));
    }
    std::env::join_paths(paths).unwrap_or_else(|_|dir.as_os_str().to_os_string())
}
pub(crate) fn is_name_surrogate_reparse_tag(tag:u32)->bool{
    (tag&0x2000_0000)!=0||matches!(tag,0xA000_0003|0xA000_000C)
}
pub fn is_linked(path:&Path,meta:&fs::Metadata)->bool{
    if meta.file_type().is_symlink(){return true;}
    #[cfg(windows)]{
        use std::os::windows::{ffi::OsStrExt,fs::MetadataExt};
        if meta.file_attributes()&0x400==0{return false;}
        #[repr(C)]
        struct FileTime{low:u32,high:u32}
        #[repr(C)]
        #[allow(dead_code)]
        struct Win32FindDataW{
            attrs:u32,created:FileTime,accessed:FileTime,written:FileTime,
            size_high:u32,size_low:u32,reserved0:u32,reserved1:u32,
            file_name:[u16;260],alt_name:[u16;14],
        }
        #[link(name="kernel32")]
        unsafe extern "system"{
            fn FindFirstFileW(file_name:*const u16,data:*mut Win32FindDataW)->*mut core::ffi::c_void;
            fn FindClose(handle:*mut core::ffi::c_void)->i32;
        }
        const INVALID_HANDLE:*mut core::ffi::c_void=(-1isize) as *mut core::ffi::c_void;
        let mut wide:Vec<u16>=path.as_os_str().encode_wide().collect();
        while wide.len()>1&&matches!(wide.last().copied(),Some(0x2F|0x5C))&&!(wide.len()==3&&wide[1]==0x3A){wide.pop();}
        wide.push(0);
        let mut data=std::mem::MaybeUninit::<Win32FindDataW>::zeroed();
        let handle=unsafe{FindFirstFileW(wide.as_ptr(),data.as_mut_ptr())};
        if handle.is_null()||handle==INVALID_HANDLE{return true;}
        let data=unsafe{let _=FindClose(handle);data.assume_init()};
        is_name_surrogate_reparse_tag(data.reserved0)
    }
    #[cfg(not(windows))]{let _=path;false}
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn prepend_env_path_keeps_target_dir_first_and_preserves_existing_entries(){
        let dir=Path::new(if cfg!(windows){r"C:\z2o\bin"}else{"/opt/z2o/bin"});
        let missing=prepend_env_path("Z2O_MISSING_TEST_ENV_PATH",dir);
        assert_eq!(std::env::split_paths(&missing).collect::<Vec<_>>(),vec![dir.to_path_buf()]);
        let path=prepend_env_path("PATH",dir);
        let entries:Vec<PathBuf>=std::env::split_paths(&path).collect();
        assert_eq!(entries.first().map(PathBuf::as_path),Some(dir));
        if let Some(existing)=std::env::var_os("PATH").filter(|v|!v.is_empty()){
            let expected:usize=std::env::split_paths(&existing).filter(|p|!p.as_os_str().is_empty()).count();
            assert_eq!(entries.len(),expected+1);
        }
    }
    #[test]
    fn surrogate_reparse_tag_classification_allows_cloud_placeholders_and_blocks_links(){
        assert!(is_name_surrogate_reparse_tag(0xA000_0003));
        assert!(is_name_surrogate_reparse_tag(0xA000_000C));
        assert!(is_name_surrogate_reparse_tag(0x2000_0001));
        assert!(!is_name_surrogate_reparse_tag(0x9000_001A));
        assert!(!is_name_surrogate_reparse_tag(0x9000_F01A));
        assert!(!is_name_surrogate_reparse_tag(0x8000_0017));
        assert!(!is_name_surrogate_reparse_tag(0));
    }
}
