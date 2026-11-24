//! Build and install a hash-bound offline native candidate bundle.
//! No Python, Git, compiler, subprocess or network fetch during installation.
use anyhow::{anyhow,bail,ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,BTreeSet},fs,io::{self,Write},path::{Path,PathBuf}};
use crate::{check_ledger,check_write,files,path,path_text,ENGINE_VERSION};

pub const SKILLS:&[&str]=&[
    "literature-zotero2obsidian","literature-inspect-zotero","literature-fulltext-extract",
    "literature-translate-zh","literature-write2obsidian","literature-filename",
    "literature-note-properties","literature-header-format","literature-body-format",
    "literature-internal-links","literature-library-links","literature-supplement-materials",
];
const MAX_FILES:usize=50_000;
const MAX_TOTAL:u64=2*1024*1024*1024;
const BOUNDARY:&str="Offline native migration candidate. The twelve local skills and native executable are installed. Legacy Python source stays in the development repository and existing installation backups, not in the candidate skill payload; z2o never executes it. External integrations, OCR resource distribution, licenses and formal release qualification remain separate.";
const GUARD:&str="\n\n> **Rust 原生迁移候选 / Native runtime only**\n>\n> 此安装包不交付旧 Python 源码、字节码、Python 扩展模块或缓存；文档中保留的旧脚本调用与示例仅为 1.1.0 开发参考，禁止将其作为任务执行路径。已迁移的机械操作使用主技能目录 literature-zotero2obsidian/runtime/z2o（Windows: z2o.exe）的 describe / invoke JSON 协议；参考 literature-zotero2obsidian/references/rust-native.md。尚不可用的操作和外部集成应明确阻断，绝不自动安装或运行 Python、调用旧脚本或虚构通过状态。当前不是正式 2.0.0 发布包。\n\n";
type FileMap=BTreeMap<String,Value>;
fn exe_name()->&'static str{if cfg!(windows){"z2o.exe"}else{"z2o"}}
fn exe_relative()->String{format!("literature-zotero2obsidian/runtime/{}",exe_name())}
fn linked(m:&fs::Metadata)->bool{
    if m.file_type().is_symlink(){return true;}
    #[cfg(windows)]{use std::os::windows::fs::MetadataExt;if m.file_attributes()&0x400!=0{return true;}}
    false
}
fn dir(p:&Path,create:bool)->Result<PathBuf>{
    match fs::symlink_metadata(p){
        Ok(m)=>ensure!(m.is_dir()&&!linked(&m),"Refusing linked/non-directory path: {}",p.display()),
        Err(e) if e.kind()==io::ErrorKind::NotFound&&create=>fs::create_dir_all(p)?,
        Err(e)=>return Err(e.into()),
    }
    let m=fs::symlink_metadata(p)?;
    ensure!(m.is_dir()&&!linked(&m),"Refusing unsafe directory: {}",p.display());
    files::canonical(p)
}
fn segment(s:&str)->Result<()>{
    ensure!(!s.is_empty()&&!matches!(s,"."|"..")&&!s.ends_with(['.',' '])&&
        !s.chars().any(|c|c.is_control()||"\\/:<>\"|?*".contains(c)),"Unsafe portable package path: {s}");
    let stem=s.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
    ensure!(!["CON","PRN","AUX","NUL","CONIN$","CONOUT$"].contains(&stem.as_str())&&
        !stem.strip_prefix("COM").or_else(||stem.strip_prefix("LPT")).is_some_and(|n|n.chars().count()==1&&"123456789¹²³".contains(n)),
        "Reserved Windows package filename");
    Ok(())
}
fn relative(raw:&str)->Result<PathBuf>{
    let mut p=PathBuf::new();for s in raw.split('/'){segment(s)?;p.push(s);}Ok(p)
}
fn posix(p:&Path)->Result<String>{
    let mut parts=vec![];
    for c in p.components(){
        let std::path::Component::Normal(s)=c else{bail!("Nonportable package path");};
        let s=s.to_str().context("Non-Unicode package path")?;segment(s)?;parts.push(s);
    }
    ensure!(!parts.is_empty(),"Empty relative path");Ok(parts.join("/"))
}
fn portable_key(s:&str)->String{
    use unicode_casefold::UnicodeCaseFold;
    use unicode_normalization::UnicodeNormalization;
    s.nfc().collect::<String>().case_fold().collect::<String>().nfc().collect()
}
fn child(root:&Path,raw:&str)->Result<PathBuf>{
    let rel=relative(raw)?;let mut p=root.to_owned();let count=rel.components().count();
    for(i,s)in rel.components().enumerate(){
        p.push(s);let m=fs::symlink_metadata(&p)?;
        ensure!(!linked(&m)&&(if i+1==count{m.is_file()}else{m.is_dir()}),"Unsafe package resource: {}",p.display());
    }
    let p=files::canonical(&p)?;ensure!(p.starts_with(root),"Package resource escaped root");Ok(p)
}
fn exec_bit(m:&fs::Metadata)->bool{
    #[cfg(unix)]{use std::os::unix::fs::PermissionsExt;return m.permissions().mode()&0o111!=0;}
    #[cfg(not(unix))]{let _=m;false}
}
fn record(p:&Path)->Result<Value>{
    check_write::no_link(p)?;let v=files::snapshot(p)?;
    Ok(json!({"sha256":v["sha256"],"size":v["size"],"executable":exec_bit(&fs::metadata(p)?)}))
}
// Keep the reference implementation in the source repository, never in a
// native candidate payload. Match portable paths case-insensitively.
fn legacy_python_path(raw:&str)->bool{
    raw.split('/').any(|part|{
        let part=part.to_ascii_lowercase();
        part=="__pycache__" || part.rsplit_once('.').is_some_and(|(_,ext)|
            matches!(ext,"py"|"pyw"|"pyc"|"pyo"|"pyz"|"pyd"|"pyi"))
    })
}
fn scan(root:&Path,skip_cache:bool)->Result<FileMap>{
    let root=dir(root,false)?;let mut result=BTreeMap::new();let mut aliases=BTreeSet::new();
    let mut stack=vec![root.clone()];let mut total=0u64;
    while let Some(p)=stack.pop(){
        for e in fs::read_dir(&p)?{
            let e=e?;let p=e.path();let m=fs::symlink_metadata(&p)?;
            ensure!(!linked(&m)&&(m.is_file()||m.is_dir()),"Refusing linked or special file: {}",p.display());
            if skip_cache&&(e.file_name()=="__pycache__"||m.is_file()&&p.extension().and_then(|s|s.to_str()).is_some_and(|s|matches!(s,"pyc"|"pyo"))){continue;}
            let name=posix(p.strip_prefix(&root)?)?;
            ensure!(aliases.insert(portable_key(&name)),"Ambiguous portable path: {name}");
            ensure!(aliases.len()<=MAX_FILES,"Package path count exceeds budget");
            if m.is_dir(){stack.push(p);continue;}
            total=total.checked_add(m.len()).context("Package size overflow")?;
            ensure!(total<=MAX_TOTAL,"Package exceeds byte budget");result.insert(name,record(&p)?);
        }
    }Ok(result)
}
fn read_small(p:&Path,limit:u64)->Result<Vec<u8>>{
    check_write::no_link(p)?;ensure!(fs::metadata(p)?.len()<=limit,"Input exceeds size budget");
    let raw=fs::read(p)?;ensure!(raw.len() as u64<=limit,"Input exceeds size budget");Ok(raw)
}
fn skill_header(root:&Path,name:&str)->Result<(String,usize)>{
    let p=root.join("SKILL.md");let text=String::from_utf8(read_small(&p,4*1024*1024)?)?;
    let first=text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut offset=text.len()-first.len();let mut lines=first.split_inclusive('\n');
    let head=lines.next().context("Skill frontmatter required")?;
    ensure!(head.trim()=="---","Skill frontmatter required");offset+=head.len();
    let mut header=String::new();let mut closed=false;
    for line in lines{offset+=line.len();if line.trim()=="---"{closed=true;break;}header.push_str(line);}
    ensure!(closed,"Unclosed skill frontmatter");
    let meta=crate::yaml_nodes::load(&header).map_err(|e|anyhow!("Invalid skill frontmatter: {}",e.message))?.value;
    ensure!(meta["name"]==name,"Unexpected skill name in {}",p.display());Ok((text,offset))
}
fn copy_file(source:&Path,dest:&Path,expected:&Value)->Result<()>{
    check_write::no_link(source)?;fs::create_dir_all(dest.parent().context("Destination parent")?)?;
    let mut input=fs::File::open(source)?;
    let mut output=fs::OpenOptions::new().write(true).create_new(true).open(dest)?;
    let n=io::copy(&mut input,&mut output)?;output.sync_all()?;drop(output);
    fs::set_permissions(dest,fs::metadata(source)?.permissions())?;
    ensure!(expected["size"].as_u64()==Some(n)&&record(dest)?==*expected&&record(source)?==*expected,"Package file changed during copy");Ok(())
}
fn copy_map(src:&Path,dst:&Path,map:&FileMap)->Result<()>{
    for(name,r)in map{let p=child(src,name)?;copy_file(&p,&dst.join(relative(name)?),r)?;}Ok(())
}
fn json_new(p:&Path,v:&Value)->Result<Value>{
    ensure!(!p.exists()&&!p.is_symlink(),"Keep prior package receipt: {}",p.display());
    let mut bytes=serde_json::to_vec_pretty(v)?;bytes.push(b'\n');
    let mut f=tempfile::NamedTempFile::new_in(p.parent().context("Receipt parent")?)?;
    f.write_all(&bytes)?;f.as_file().sync_all()?;
    f.persist_noclobber(p).map_err(|e|anyhow!("Cannot publish receipt: {}",e.error))?;
    let binding=check_ledger::bind_file(p)?;
    ensure!(binding["sha256"]==files::digest(&bytes),"Receipt changed");Ok(binding)
}
fn valid_hash(s:&str)->bool{s.len()==64&&s.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))}
pub fn package(source:&Path,destination:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let source=dir(source,false)?;
    ensure!(!destination.exists()&&!destination.is_symlink(),"Use a new bundle destination");
    let parent=dir(destination.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new(".")),true)?;
    let destination=parent.join(destination.file_name().context("Bundle destination name")?);
    ensure!(SKILLS.iter().all(|name|!destination.starts_with(source.join(name))),"Bundle cannot be inside an input skill");
    let _lock=check_write::lock_target(&destination)?;
    let stage=tempfile::Builder::new().prefix(".z2o-package-").tempdir_in(&parent)?;
    let payload=stage.path().join("skills");fs::create_dir(&payload)?;
    for(i,name)in SKILLS.iter().enumerate(){
        let root=dir(&source.join(name),false)?;skill_header(&root,name)?;
        let m=scan(&root,true)?;
        ensure!(m.keys().all(|p|!p.starts_with("runtime/")),"Package input must be a repository skill, not an installed runtime");
        let native_files:FileMap=m.iter().filter(|(p,_)|!legacy_python_path(p))
            .map(|(p,r)|(p.clone(),r.clone())).collect();
        let dest=payload.join(name);fs::create_dir(&dest)?;copy_map(&root,&dest,&native_files)?;
        let (mut header,at)=skill_header(&dest,name)?;
        header.insert_str(at,GUARD);fs::write(dest.join("SKILL.md"),header.as_bytes())?;
        ensure!(scan(&root,true)?==m,"Skill changed during packaging");
        progress(i+1,SKILLS.len(),"stage native skill bundle");
    }
    let exe=files::canonical(&std::env::current_exe()?)?;
    let target=payload.join(exe_relative());let executable=record(&exe)?;
    ensure!(executable["size"].as_u64().is_some_and(|n|n>0),"Missing actual executable");
    copy_file(&exe,&target,&executable)?;
    #[cfg(unix)]ensure!(executable["executable"]==true,"Packaged binary is not executable");
    let payload_files=scan(&payload,false)?;
    ensure!(payload_files.keys().all(|p|!legacy_python_path(p)),"Legacy Python file in native payload");
    let manifest=json!({"schema_version":1,"bundle_kind":"native_migration_candidate","qualified":false,
        "engine_version":ENGINE_VERSION,"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "skills":SKILLS,"runtime":exe_relative(),"files":payload_files,"system_python_required":false,"legacy_python_files_included":false,"boundary":BOUNDARY});
    let m=json_new(&stage.path().join("package.json"),&manifest)?;
    ensure!(!destination.exists()&&!destination.is_symlink(),"Bundle destination appeared during preparation");
    fs::rename(stage.path(),&destination).context("Cannot publish bundle directory")?;
    let p=destination.join("package.json");
    let checked=load_bundle(&p,m["sha256"].as_str().unwrap())?;
    ensure!(checked.files==payload_files,"Published bundle differs");
    Ok(json!({"package_manifest":checked.binding,"package_root":path_text(&destination)?,"file_count":payload_files.len(),
        "skill_count":SKILLS.len(),"qualified":false,"boundary":BOUNDARY}))
}
struct Bundle{binding:Value,payload:PathBuf,files:FileMap}
fn load_manifest(manifest:&Path,expected:&str)->Result<(Value,FileMap)>{
    ensure!(valid_hash(expected),"Provide the independently trusted manifest SHA-256");
    let raw=read_small(manifest,32*1024*1024)?;
    ensure!(files::digest(&raw)==expected,"NATIVE_BUNDLE_MANIFEST_HASH_MISMATCH");
    let b=check_ledger::bind_file(manifest)?;ensure!(b["sha256"]==expected,"Bundle manifest changed");
    let v=crate::parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(v["schema_version"].as_u64()==Some(1)&&v["bundle_kind"]=="native_migration_candidate"&&v["qualified"]==false,
        "Expected explicit native candidate bundle");
    ensure!(v["platform"]==std::env::consts::OS&&v["architecture"]==std::env::consts::ARCH&&v["engine_version"]==ENGINE_VERSION,
        "Bundle architecture, platform or engine version mismatch");
    ensure!(v["skills"]==json!(SKILLS)&&v["runtime"]==exe_relative()&&v["system_python_required"]==false&&v["legacy_python_files_included"]==false,"Bundle skill scope or runtime differs");
    let table=v["files"].as_object().filter(|m|!m.is_empty()&&m.len()<=MAX_FILES).context("Bundle file list required")?;
    let mut total=0u64;let mut aliases=BTreeSet::new();let mut expected_files=BTreeMap::new();
    for(name,r)in table{
        relative(name)?;let skill=name.split('/').next().unwrap();
        ensure!(SKILLS.contains(&skill)&&name.contains('/'),"Unexpected bundle file");
        ensure!(!legacy_python_path(name),"NATIVE_BUNDLE_LEGACY_PYTHON_FILE: {name}");
        ensure!(aliases.insert(portable_key(name)),"Ambiguous bundle filename");
        ensure!(r.as_object().is_some_and(|m|m.len()==3)&&r["sha256"].as_str().is_some_and(valid_hash)&&r["executable"].is_boolean(),"Invalid resource binding");
        total=total.checked_add(r["size"].as_u64().context("Bundle file size")?).context("Bundle size overflow")?;
        ensure!(total<=MAX_TOTAL,"Bundle exceeds byte budget");expected_files.insert(name.clone(),r.clone());
    }
    for name in SKILLS{ensure!(expected_files.contains_key(&format!("{name}/SKILL.md")),"Missing skill manifest");}
    let runtime=expected_files.get(&exe_relative()).context("Missing bundled runtime")?;
    ensure!(runtime["sha256"]==files::snapshot(&std::env::current_exe()?)?["sha256"],"Run the exact executable bound by this package");
    #[cfg(unix)]ensure!(runtime["executable"]==true,"Bundle runtime executable bit missing");
    Ok((b,expected_files))
}
fn load_bundle(manifest:&Path,expected:&str)->Result<Bundle>{
    let (b,expected_files)=load_manifest(manifest,expected)?;
    let root=path(&b["path"])?;
    let payload=dir(&root.parent().context("Bundle directory")?.join("skills"),false)?;
    for name in SKILLS{skill_header(&payload.join(name),name)?;}
    ensure!(scan(&payload,false)?==expected_files,"NATIVE_BUNDLE_FILE_MISMATCH: missing, added or changed asset");
    ensure!(check_ledger::bind_file(manifest)?==b,"Bundle manifest changed during verification");
    Ok(Bundle{binding:b,payload,files:expected_files})
}
pub fn inspect_bundle(manifest:&Path,expected:&str)->Result<Value>{
    let b=load_bundle(manifest,expected)?;
    Ok(json!({"package_manifest":b.binding,"file_count":b.files.len(),"skill_count":SKILLS.len(),"qualified":false,"boundary":BOUNDARY}))
}
fn skill_files(files:&FileMap,name:&str)->FileMap{
    let prefix=format!("{name}/");files.iter().filter_map(|(p,r)|p.strip_prefix(&prefix).map(|x|(x.to_owned(),r.clone()))).collect()
}
fn snapshot(p:&Path,name:&str)->Result<Option<FileMap>>{
    match fs::symlink_metadata(p){
        Ok(_)=>{let root=dir(p,false)?;skill_header(&root,name)?;Ok(Some(scan(&root,false)?))},
        Err(e) if e.kind()==io::ErrorKind::NotFound=>Ok(None),
        Err(e)=>Err(e.into()),
    }
}
/// Compare the installed candidate against its independently trusted manifest.
/// This does not require unpacked bundle contents and never repairs the installation.
pub fn verify_install(manifest:&Path,expected:&str,skills_root:&Path)->Result<Value>{
    let (binding,installed)=load_manifest(manifest,expected)?;
    let root=dir(skills_root,false)?;
    let runtime=child(&root,&exe_relative())?;
    ensure!(runtime==files::canonical(&std::env::current_exe()?)?,
        "NATIVE_INSTALLED_RUNTIME_PATH_MISMATCH: run the executable inside this skills root");
    for name in SKILLS{
        ensure!(snapshot(&root.join(name),name)?==Some(skill_files(&installed,name)),
            "NATIVE_INSTALLED_FILE_MISMATCH: missing, added or changed skill file in {name}");
    }
    ensure!(check_ledger::bind_file(manifest)?==binding,"Bundle manifest changed during verification");
    Ok(json!({"schema_version":1,"status":"installed_candidate_files_verified","skills_root":path_text(&root)?,
        "package_manifest":binding,"runtime_path":path_text(&runtime)?,
        "runtime_sha256":installed[&exe_relative()]["sha256"],"skill_count":SKILLS.len(),
        "file_count":installed.len(),"system_python_required":false,"qualified":false,
        "boundary":"Read-only comparison of the installed candidate files against this manifest. Other skills are outside scope. This does not qualify platform installs, external integrations, licenses, OCR resources, or the full literature workflow."}))
}
pub fn install(manifest:&Path,expected:&str,skills_root:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let b=load_bundle(manifest,expected)?;
    let root=dir(skills_root,true)?;
    ensure!(!root.starts_with(&b.payload)&&!b.payload.starts_with(&root),"Install destination overlaps source bundle");
    let _lock=check_write::lock_target(&root.join("native-install"))?;
    let mut before=BTreeMap::new();let mut changed=vec![];let mut skipped=vec![];
    for &name in SKILLS{
        let old=snapshot(&root.join(name),name)?;
        if old.as_ref()==Some(&skill_files(&b.files,name)){skipped.push(name);}else{changed.push(name);}
        before.insert(name,old);
    }
    if changed.is_empty(){return Ok(json!({"installed":[],"skipped":skipped,"backup_root":null,
        "package_manifest":b.binding,"system_python_required":false,"qualified":false,"boundary":BOUNDARY}));}
    let stage=tempfile::Builder::new().prefix(".z2o-native-stage-").tempdir_in(&root)?;
    let staged=stage.path().join("skills");fs::create_dir(&staged)?;
    for &name in &changed{
        let dst=staged.join(name);fs::create_dir(&dst)?;
        let input=dir(&b.payload.join(name),false)?;
        let expected_files=skill_files(&b.files,name);copy_map(&input,&dst,&expected_files)?;
        ensure!(scan(&dst,false)?==expected_files,"Staged skill changed");
    }
    let now=load_bundle(manifest,expected)?;
    ensure!(now.binding==b.binding&&now.files==b.files,"Bundle changed during installation");
    for &name in SKILLS{ensure!(snapshot(&root.join(name),name)?==before[name],"Installed skill changed during preparation: {name}");}
    let backups=dir(&root.join(".z2o-backups"),true)?;
    let recovery=tempfile::Builder::new().prefix("native-").tempdir_in(&backups)?.keep();
    let previous=recovery.join("previous");let rejected=recovery.join("failed-new");
    fs::create_dir(&previous)?;fs::create_dir(&rejected)?;
    json_new(&recovery.join("plan.json"),&json!({"schema_version":1,"status":"prepared","package_manifest":b.binding,
        "root":path_text(&root)?,"changed":changed,"before":before,
        "after":changed.iter().map(|name|(*name,skill_files(&b.files,name))).collect::<BTreeMap<_,_>>(),
        "staging_directory":path_text(stage.path())?,"boundary":"Inspect actual target/preimage hashes before recovering an interrupted installation. Never overwrite locally modified files."}))?;
    let mut actions=Vec::<(&str,bool,bool)>::new();
    let result=(||->Result<()>{
        for(i,&name)in changed.iter().enumerate(){
            let target=root.join(name);ensure!(snapshot(&target,name)?==before[name],"Target changed: {name}");
            actions.push((name,false,false));
            if before[name].is_some(){fs::rename(&target,previous.join(name))?;actions.last_mut().unwrap().1=true;}
            ensure!(!target.exists()&&!target.is_symlink(),"Unexpected installation target");
            fs::rename(staged.join(name),&target)?;actions.last_mut().unwrap().2=true;
            ensure!(snapshot(&target,name)?==Some(skill_files(&b.files,name)),"Published skill differs");
            progress(i+1,changed.len(),"install native skill");
        }
        for &name in SKILLS{ensure!(snapshot(&root.join(name),name)?==Some(skill_files(&b.files,name)),"Final install verification differs: {name}");}
        Ok(())
    })();
    if let Err(e)=result{
        let mut rollback=vec![];
        for (name,old,new) in actions.into_iter().rev(){
            let target=root.join(name);
            let r=(||->Result<()>{
                if new{
                    ensure!(snapshot(&target,name)?==Some(skill_files(&b.files,name)),"Installed files changed; preserve them");
                    fs::rename(&target,rejected.join(name))?;
                }
                if old{ensure!(!target.exists()&&!target.is_symlink(),"Target appeared; preserve existing");fs::rename(previous.join(name),&target)?;}
                Ok(())
            })();
            if let Err(e)=r{rollback.push(format!("{name}: {e:#}"));}
        }
        let _=json_new(&recovery.join("failed.json"),&json!({"error":format!("{e:#}"),"rollback_errors":rollback}));
        bail!("Native installation failed: {e:#}; recovery evidence: {}; rollback errors: {rollback:?}",recovery.display());
    }
    let receipt=json!({"schema_version":1,"status":"completed_candidate_install","installed":changed,"skipped":skipped,
        "backup_root":path_text(&recovery)?,"package_manifest":b.binding,"system_python_required":false,"qualified":false,
        "external_integrations":"not_installed_or_qualified","boundary":BOUNDARY});
    json_new(&recovery.join("completed.json"),&receipt)?;Ok(receipt)
}
