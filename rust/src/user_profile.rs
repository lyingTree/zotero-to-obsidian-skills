//! Skill-local persistent configuration. User state lives beside the skill,
//! never in a generic home-directory config location.
use anyhow::{anyhow,ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::{fs,path::{Path,PathBuf}};
use crate::{acceptance,check_write,files,path,path_text,pipeline,zotero_local};

pub const SKILL:&str="literature-zotero2obsidian";
pub const RELATIVE:&str="user/profile.json";
const MAX_BYTES:u64=256*1024;
const PATH_FIELDS:&[&str]=&[
    "zotero_local_api","zotero_data_dir","zotero_storage_dir","obsidian_vault_root",
    "literature_root","template_root","glossary_path","assets_resource_dir","work_root","obsidian_cli",
];

fn skill_root(explicit:Option<&Path>)->Result<PathBuf>{
    if let Some(p)=explicit{
        let p=files::canonical(p)?;
        ensure!(p.join("SKILL.md").is_file()&&p.file_name().and_then(|s|s.to_str())==Some(SKILL),
            "skill_root must name the {SKILL} skill directory");
        return Ok(p);
    }
    let exe=files::canonical(&std::env::current_exe()?)?;
    let runtime=exe.parent().context("runtime directory")?;
    let root=runtime.parent().context("skill directory")?;
    ensure!(root.file_name().and_then(|s|s.to_str())==Some(SKILL),
        "Cannot locate {SKILL} from the running executable; pass skill_root explicitly");
    files::canonical(root)
}
pub fn defaults()->Value{
    json!({
        "schema_version":1,
        "skill":SKILL,
        "paths":{
            "zotero_local_api":"http://127.0.0.1:23119/api",
            "zotero_data_dir":null,
            "zotero_storage_dir":null,
            "obsidian_vault_root":null,
            "literature_root":null,
            "template_root":null,
            "glossary_path":null,
            "assets_resource_dir":null,
            "work_root":null,
            "obsidian_cli":null
        },
        "zotero_library":null,
        "templates":{},
        "preferences":{
            "formula_table_enhancement":"unasked",
            "arxiv_tex_auto":true,
            "transparent_figure":"preserve_original"
        },
        "provenance":{"recorded_by":"first_run_discovery","notes":[]},
        "updated_at":null
    })
}
fn merge(base:&mut Value,patch:&Value){
    if let (Some(a),Some(b))=(base.as_object_mut(),patch.as_object()){
        for (k,v) in b{
            match (a.get_mut(k),v){
                (Some(Value::Object(old)),Value::Object(new))=>merge_value(old,new),
                _=>{a.insert(k.clone(),v.clone());}
            }
        }
    }
}
fn merge_value(base:&mut Map<String,Value>,patch:&Map<String,Value>){
    for (k,v) in patch{
        match (base.get_mut(k),v){
            (Some(Value::Object(old)),Value::Object(new))=>merge_value(old,new),
            _=>{base.insert(k.clone(),v.clone());}
        }
    }
}
fn looks_like_item_work_dir(p:&Path)->bool{
    let Some(name)=p.file_name().and_then(|s|s.to_str()) else{return false;};
    if matches!(name,"work"|"output"){return true;}
    if p.join("work").join("run.json").exists() || p.join("output").join("candidate.md").exists(){return true;}
    if let Some((_,tail))=name.rsplit_once('-'){
        if tail.len()==8 && tail.bytes().all(|b|b.is_ascii_uppercase()||b.is_ascii_digit()){
            return true;
        }
    }
    false
}
fn work_inside_vault(work:&Path,vault:&Path)->bool{
    let w=acceptance::resolve(work).unwrap_or_else(|_|work.to_path_buf());
    let v=acceptance::resolve(vault).unwrap_or_else(|_|vault.to_path_buf());
    w==v || w.starts_with(&v)
}
fn validate(profile:&Value)->Result<()>{
    let m=profile.as_object().context("profile must be a JSON object")?;
    ensure!(m.get("schema_version").and_then(Value::as_u64)==Some(1),"profile schema_version must be 1");
    ensure!(m.get("skill").and_then(Value::as_str).unwrap_or(SKILL)==SKILL,"profile skill identity differs");
    let paths=m.get("paths").and_then(Value::as_object).context("profile paths must be an object")?;
    for (key,value) in paths{
        ensure!(PATH_FIELDS.contains(&key.as_str()),"unknown profile path field: {key}");
        ensure!(value.is_null()||value.as_str().is_some_and(|s|!s.trim().is_empty()&&s.trim()==s),
            "profile path {key} must be null or nonempty trimmed text");
    }
    if let Some(wr)=paths.get("work_root").and_then(Value::as_str){
        let p=Path::new(wr);
        ensure!(p.is_absolute(),"paths.work_root must be an absolute path");
        ensure!(!looks_like_item_work_dir(p),
            "paths.work_root must be the shared base work directory (e.g. .../literature-pipeline-work/<Vault>), not a per-item subdirectory");
        if let Some(vr)=paths.get("obsidian_vault_root").and_then(Value::as_str){
            let vp=Path::new(vr);
            if vp.is_absolute(){
                ensure!(!work_inside_vault(p,vp),"paths.work_root must reside strictly outside obsidian_vault_root");
            }
        }
    }
    if let Some(lib)=m.get("zotero_library").filter(|v|!v.is_null()){
        let lm=lib.as_object().context("zotero_library must be an object or null")?;
        ensure!(lm.get("type").and_then(Value::as_str).is_some_and(|s|matches!(s,"user"|"group")),"zotero_library.type must be user or group");
        ensure!(lm.get("id").and_then(Value::as_u64).is_some(),"zotero_library.id must be an integer");
        ensure!(lm.get("name").is_none_or(|v|v.is_null()||v.as_str().is_some()),"zotero_library.name must be string or null");
    }
    if let Some(tpl)=m.get("templates").filter(|v|!v.is_null()){
        let tm=tpl.as_object().context("templates must be an object or null")?;
        for (k,v) in tm{
            ensure!(v.is_null()||v.as_str().is_some_and(|s|!s.trim().is_empty()&&s.trim()==s),
                "templates.{k} must be null or nonempty trimmed text");
        }
    }
    let preferences=m.get("preferences").and_then(Value::as_object).context("profile preferences must be an object")?;
    ensure!(preferences.get("formula_table_enhancement").and_then(Value::as_str)
        .is_some_and(|s|matches!(s,"unasked"|"native"|"ocr")),"formula_table_enhancement must be unasked, native or ocr");
    ensure!(preferences.get("arxiv_tex_auto").is_none_or(Value::is_boolean),"arxiv_tex_auto must be boolean");
    ensure!(preferences.get("transparent_figure").and_then(Value::as_str)
        .is_none_or(|s|matches!(s,"preserve_original"|"white")),"transparent_figure must be preserve_original or white");
    Ok(())
}
fn resolve_command_on_path(cmd:&str)->Option<PathBuf>{
    let path_var=std::env::var_os("PATH")?;
    let has_ext=Path::new(cmd).extension().is_some();
    let exts:Vec<String>=if cfg!(windows)&&!has_ext{
        std::env::var("PATHEXT").unwrap_or_else(|_|".COM;.EXE;.BAT;.CMD".to_owned())
            .split(';').filter(|s|!s.trim().is_empty()).map(|s|s.trim().to_owned()).collect()
    }else{vec![String::new()]};
    for dir in std::env::split_paths(&path_var){
        if dir.as_os_str().is_empty(){continue;}
        for ext in &exts{
            let candidate=if ext.is_empty(){dir.join(cmd)}else{dir.join(format!("{cmd}{ext}"))};
            if fs::symlink_metadata(&candidate).is_ok_and(|m|m.is_file()){
                return Some(files::canonical(&candidate).unwrap_or(candidate));
            }
        }
    }
    None
}
fn cli_state(value:&Value)->Value{
    let Some(raw)=value.as_str() else{return json!({"status":"not_configured"});};
    if raw.contains('/')||raw.contains('\\')||raw.contains(':'){
        return path_state(value);
    }
    if let Some(resolved)=resolve_command_on_path(raw){
        json!({"status":"ok","value":raw,"resolved":path_text(&resolved).unwrap_or_else(|_|raw.to_owned()),"kind":"command"})
    }else{
        json!({"status":"command_not_found","value":raw,"kind":"command"})
    }
}
fn path_state(value:&Value)->Value{
    let Some(raw)=value.as_str()else{return json!({"status":"not_configured"});};
    let p=Path::new(raw);
    if !p.is_absolute(){return json!({"status":"relative_path_rejected","value":raw});}
    match fs::symlink_metadata(p){
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>json!({"status":"missing","value":raw}),
        Err(e)=>json!({"status":"unreadable","value":raw,"reason":e.to_string()}),
        Ok(m)=>{
            let kind=if m.is_file(){"file"}else if m.is_dir(){"directory"}else{"other"};
            match files::canonical(p){
                Ok(actual)=>json!({"status":"ok","value":raw,"canonical":path_text(&actual).unwrap_or_else(|_|raw.to_owned()),"kind":kind}),
                Err(e)=>json!({"status":"unreadable","value":raw,"reason":format!("{e:#}")}),
            }
        }
    }
}
fn work_root_state(value:&Value,vault_root:Option<&str>)->Value{
    let Some(raw)=value.as_str() else{return json!({"status":"not_configured"});};
    let p=Path::new(raw);
    if !p.is_absolute(){return json!({"status":"relative_path_rejected","value":raw});}
    if looks_like_item_work_dir(p){return json!({"status":"item_subdir_rejected","value":raw});}
    if let Some(vr)=vault_root{
        let vp=Path::new(vr);
        if vp.is_absolute() && work_inside_vault(p,vp){
            return json!({"status":"inside_vault_rejected","value":raw});
        }
    }
    match fs::symlink_metadata(p){
        Ok(m) if !m.is_dir()=>json!({"status":"not_directory","value":raw}),
        Ok(_)=>path_state(value),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{
            let mut ancestor=p.parent();
            while let Some(a)=ancestor{
                match fs::symlink_metadata(a){
                    Ok(m) if m.is_dir()&&!files::is_linked(a,&m)=>return json!({"status":"creatable","value":raw,"existing_ancestor":path_text(a).ok()}),
                    Ok(_)=>return json!({"status":"unwritable_ancestor","value":raw}),
                    Err(e2) if e2.kind()==std::io::ErrorKind::NotFound=>{ancestor=a.parent();}
                    Err(e2)=>return json!({"status":"unreadable","value":raw,"reason":e2.to_string()}),
                }
            }
            json!({"status":"missing","value":raw})
        }
        Err(e)=>json!({"status":"unreadable","value":raw,"reason":e.to_string()}),
    }
}
fn validate_paths(profile:&Value)->Result<Value>{
    let paths=profile["paths"].as_object().context("profile paths")?;
    let vault_root=paths.get("obsidian_vault_root").and_then(Value::as_str);
    let mut rows=Map::new();let mut valid=true;
    for (key,value) in paths{
        let row=if key=="zotero_local_api"{
            let ok=value.as_str().is_some_and(|s|s.starts_with("http://127.0.0.1:")||s.starts_with("http://localhost:"));
            if !ok{valid=false;}
            json!({"status":if ok{"configured"}else{"invalid"},"value":value})
        }else if key=="obsidian_cli"{
            let row=cli_state(value);
            if !matches!(row["status"].as_str(),Some("ok"|"not_configured"|"command_not_found")){valid=false;}
            row
        }else if key=="work_root"{
            let row=work_root_state(value,vault_root);
            if !matches!(row["status"].as_str(),Some("ok"|"creatable"|"not_configured")){valid=false;}
            row
        }else{
            let row=path_state(value);
            if !row["status"].is_null()&&!matches!(row["status"].as_str(),Some("ok"|"not_configured")){valid=false;}
            row
        };
        rows.insert(key.clone(),row);
    }
    Ok(json!({"valid":valid,"paths":rows}))
}
pub fn sha1_hex(data:&[u8])->String{
    let mut msg=data.to_vec();
    let bit_len=(data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len()%64!=56{msg.push(0);}
    msg.extend_from_slice(&bit_len.to_be_bytes());
    let mut h=[0x67452301u32,0xEFCDAB89,0x98BADCFE,0x10325476,0xC3D2E1F0];
    for chunk in msg.chunks_exact(64){
        let mut w=[0u32;80];
        for i in 0..16{
            w[i]=u32::from_be_bytes([chunk[i*4],chunk[i*4+1],chunk[i*4+2],chunk[i*4+3]]);
        }
        for i in 16..80{
            w[i]=(w[i-3]^w[i-8]^w[i-14]^w[i-16]).rotate_left(1);
        }
        let [mut a,mut b,mut c,mut d,mut e]=h;
        for (i,&wt) in w.iter().enumerate(){
            let (f,k)=match i{
                0..=19=>((b&c)|((!b)&d),0x5A827999u32),
                20..=39=>(b^c^d,0x6ED9EBA1u32),
                40..=59=>((b&c)|(b&d)|(c&d),0x8F1BBCDCu32),
                _ =>(b^c^d,0xCA62C1D6u32),
            };
            let temp=a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wt);
            e=d;d=c;c=b.rotate_left(30);b=a;a=temp;
        }
        h[0]=h[0].wrapping_add(a);h[1]=h[1].wrapping_add(b);h[2]=h[2].wrapping_add(c);
        h[3]=h[3].wrapping_add(d);h[4]=h[4].wrapping_add(e);
    }
    h.iter().map(|v|format!("{v:08x}")).collect()
}
pub(crate) fn title_bucket(title:&str)->String{
    let normalized:String=title.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let digest=sha1_hex(normalized.as_bytes());
    digest[..16].to_owned()
}
fn clean_component(s:&str)->String{
    s.chars().map(|ch|if "<>:\"/\\|?*".contains(ch)||ch.is_control(){'_'}else{ch}).collect()
}
fn has_cjk(s:&str)->bool{
    s.chars().any(|c|('\u{4E00}'..='\u{9FFF}').contains(&c)||('\u{3400}'..='\u{4DBF}').contains(&c))
}
fn extract_year(raw:&str)->Option<String>{
    let b=raw.as_bytes();
    for i in 0..b.len().saturating_sub(3){
        let ok_prev=i==0||!b[i-1].is_ascii_digit();
        let ok_next=i+4==b.len()||!b[i+4].is_ascii_digit();
        if ok_prev&&ok_next&&b[i..i+4].iter().all(u8::is_ascii_digit){
            let s=&raw[i..i+4];
            if matches!(s.as_bytes()[0],b'1'|b'2'){return Some(s.to_owned());}
        }
    }
    None
}
fn creator_marker(c:&Value)->Option<String>{
    if let Some(name)=c["name"].as_str().map(str::trim).filter(|s|!s.is_empty()){
        return Some(name.to_owned());
    }
    let last=c["lastName"].as_str().map(str::trim).unwrap_or("");
    let first=c["firstName"].as_str().map(str::trim).unwrap_or("");
    if !last.is_empty(){
        if has_cjk(last)&&!first.is_empty()&&has_cjk(first){
            return Some(format!("{last}{first}"));
        }
        return Some(last.to_owned());
    }
    if !first.is_empty(){return Some(first.to_owned());}
    None
}
fn author_markers(creators:&Value)->Vec<String>{
    let Some(list)=creators.as_array() else{return vec![];};
    let authors:Vec<String>=list.iter()
        .filter(|c|c["creatorType"]=="author")
        .filter_map(creator_marker)
        .collect();
    if !authors.is_empty(){return authors;}
    list.iter().filter_map(creator_marker).collect()
}
fn first_author_token(creators:&Value)->String{
    author_markers(creators).into_iter().next().unwrap_or_else(||"未知作者".into())
}
fn filename_author_token(creators:&Value)->String{
    let markers=author_markers(creators);
    match markers.as_slice(){
        []=>"未知作者".into(),
        [one]=>one.clone(),
        [one,two]=>format!("{one} and {two}"),
        [first,..]=>format!("{first} et al"),
    }
}
fn truncate_filename(prefix:&str,title:&str,suffix:&str,max_units:usize)->String{
    let fixed_units=prefix.encode_utf16().count()+suffix.encode_utf16().count();
    let budget=max_units.saturating_sub(fixed_units);
    let mut consumed=0usize;
    let mut cut=0usize;
    for (idx,ch) in title.char_indices(){
        let u=ch.len_utf16();
        if consumed+u>budget{break;}
        consumed+=u;
        cut=idx+ch.len_utf8();
    }
    let trimmed=title[..cut].trim_end();
    format!("{prefix}{trimmed}{suffix}")
}
fn relative_under_vault(vault_root:Option<&str>,target:Option<&str>,fallback:&str)->String{
    let (Some(v),Some(t))=(vault_root,target) else{return fallback.into();};
    let vp=files::canonical(Path::new(v)).unwrap_or_else(|_|PathBuf::from(v));
    let tp=files::canonical(Path::new(t)).unwrap_or_else(|_|PathBuf::from(t));
    if let Ok(rel)=tp.strip_prefix(&vp){
        let parts:Vec<&str>=rel.components().filter_map(|c|match c{
            std::path::Component::Normal(s)=>s.to_str(),
            _=>None,
        }).collect();
        if !parts.is_empty(){return parts.join("/");}
    }
    fallback.into()
}
fn extract_template_skeleton(template_root:Option<&str>,template_name:Option<&str>,item_type:&str)->Value{
    let (Some(dir),Some(name))=(template_root,template_name) else{return Value::Null;};
    let p=Path::new(dir).join(name);
    if !p.is_file(){return Value::Null;}
    let Ok(raw)=fs::read_to_string(&p) else{return Value::Null;};
    let trimmed_raw=raw.trim_start_matches('\u{feff}').trim_start();
    if trimmed_raw.starts_with("<%*"){
        let type_label=match item_type{
            "journalArticle"=>"期刊论文",
            "thesis"=>"学位论文",
            "conferencePaper"=>"会议论文",
            "preprint"=>"预印本",
            _=>"",
        };
        let mut fm_keys=Vec::new();
        for key in ["类型","年份","关键词"]{
            if raw.contains(&format!("'{key}: "))||raw.contains(&format!("\"{key}: "))||raw.contains(key){
                fm_keys.push(key.to_owned());
            }
        }
        if item_type=="journalArticle"&&(raw.contains("'期刊名: ")||raw.contains("\"期刊名: ")||raw.contains("期刊名")){
            fm_keys.push("期刊名".to_owned());
        }
        return json!({
            "template_file":name,
            "template_kind":"templater_js",
            "type_label":type_label,
            "frontmatter_keys":fm_keys,
            "headings":[
                "## 摘要",
                "## 1 <首节>",
                "## N 参考文献",
                "## [Zotero原条目](zotero://select/library/items/<KEY>)"
            ],
            "format_cues":[
                "yaml: 仅保留 frontmatter_keys（无作者/标题/DOI/链接/库内参考文献）",
                "header: # 原题\\n中文译名\\n\\n[Name](mailto:addr)${}^{1,\\ast}$; Name2${}^{1,2}$\\n\\n${}^{1}$ 单位1  \\n${}^{2}$ 单位2",
                "citations: <sup>\\[[1](<#^ref-1>)-[5](<#^ref-5>)\\]</sup> 或 [Author (Year)](<#^ref-1>)",
                "figures: ![[<asset_bucket>/figN.png]]\\n**图N：** 题注\\n^fig-N（块内无空行）",
                "tables: **表N：** 题注 + 标准 GFM 表格 + ^tbl-N（convert_markdown_tables 自动转 Rich Table v2）",
                "equations: $$\\n... \\tag{n}\\n$$\\n^eq-n",
                "footer: ---\\n\\n## [Zotero原条目](zotero://select/library/items/<KEY>)"
            ]
        });
    }
    let lines:Vec<&str>=raw.lines().collect();
    let mut fm_keys=Vec::new();
    let mut headings=Vec::new();
    let mut markers=Vec::new();
    let mut i=0usize;
    let mut in_templater=false;
    if lines.first().is_some_and(|l|l.trim()=="---"){
        i=1;
        while i<lines.len(){
            let t=lines[i].trim();
            if t=="---"{i+=1;break;}
            if t.starts_with("<%*"){in_templater=true;}
            if !in_templater&&!t.is_empty()&&!t.starts_with('#')&&!t.starts_with('-'){
                if let Some((k,_))=t.split_once(':'){
                    let k=k.trim();
                    if !k.is_empty()&&!k.contains('<'){fm_keys.push(k.to_owned());}
                }
            }
            if t.ends_with("%>"){in_templater=false;}
            i+=1;
        }
    }
    in_templater=false;
    for l in &lines[i..]{
        let t=l.trim();
        if t.starts_with("<%*"){in_templater=true;}
        if in_templater{
            if t.ends_with("%>"){in_templater=false;}
            continue;
        }
        if t.starts_with("## ")||t.starts_with("### "){
            if headings.len()<24{headings.push(t.to_owned());}
        }else if (t.starts_with("**")||t.starts_with('^'))&&markers.len()<16&&!t.contains("<%"){
            markers.push(t.chars().take(60).collect::<String>());
        }
    }
    json!({"template_file":name,"template_kind":"markdown","frontmatter_keys":fm_keys,"headings":headings,"format_cues":markers})
}
pub fn locate(skill_root:Option<&Path>)->Result<Value>{
    let root=match skill_root{
        Some(p)=>{
            let c=files::canonical(p)?;
            let nested=c.join(SKILL);
            if !c.join("SKILL.md").is_file()&&nested.join("SKILL.md").is_file(){
                files::canonical(&nested)?
            }else{
                c
            }
        }
        None=>self::skill_root(None)?,
    };
    let p=root.join(RELATIVE);
    let exists=p.is_file();
    let writable=fs::symlink_metadata(&root).is_ok_and(|m|m.is_dir())&&!root.join(RELATIVE).is_symlink();
    Ok(json!({"schema_version":1,"status":if exists{"configured"}else{"uninitialized"},
        "skill_root":path_text(&root)?,"profile_path":path_text(&p)?,"exists":exists,"writable":writable,
        "scope":"skill_local","generic_user_directory_used":false}))
}
pub fn load(skill_root:Option<&Path>,verify_paths:bool)->Result<Value>{
    let loc=locate(skill_root)?;
    let p=path(&loc["profile_path"])?;
    if !p.is_file(){
        let mut profile=defaults();
        profile["onboarding"]=json!({"formula_table_enhancement_question":"pending"});
        return Ok(json!({"schema_version":1,"status":"uninitialized","profile":profile,
            "profile_path":loc["profile_path"],"onboarding_pending":true,"generic_user_directory_used":false}));
    }
    check_write::no_link(&p)?;
    ensure!(fs::metadata(&p)?.len()<=MAX_BYTES,"profile exceeds size budget");
    let(raw,b)=files::read_bound(&p)?;
    ensure!(raw.len() as u64<=MAX_BYTES,"profile exceeds size budget");
    let mut profile=crate::parse_json(std::str::from_utf8(&raw)?.trim_start_matches('\u{feff}'))?;
    validate(&profile)?;
    let defaults=defaults();
    for (k,v) in defaults.as_object().unwrap(){
        if k!="updated_at"&&profile.get(k).is_none(){profile[k]=v.clone();}
    }
    let validation=if verify_paths{validate_paths(&profile)?}else{json!({"valid":null,"paths":{}})};
    let pending=profile["preferences"]["formula_table_enhancement"]=="unasked";
    Ok(json!({"schema_version":1,"status":if validation["valid"]==false{"stale_paths"}else{"configured"},
        "profile":profile,"profile_binding":b,"profile_path":loc["profile_path"],
        "validation":validation,"onboarding_pending":pending,
        "next_step":if pending{"Ask once whether to install optional formula/table OCR enhancement; otherwise use the built-in PDF path."}else{"Reuse the recorded paths; do not search again unless a recorded path is missing or the user changes it."},
        "generic_user_directory_used":false}))
}
pub fn save(profile:Value,skill_root:Option<&Path>)->Result<Value>{
    validate(&profile)?;
    let loc=locate(skill_root)?;
    let p=path(&loc["profile_path"])?;
    let parent=p.parent().context("profile parent")?;
    match fs::symlink_metadata(parent){
        Ok(m)=>ensure!(m.is_dir()&&!files::is_linked(parent,&m),"Refusing a linked profile directory"),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>fs::create_dir_all(parent)?,
        Err(e)=>return Err(e.into()),
    }
    let _lock=check_write::lock_target(&p)?;
    let mut profile=profile;
    profile["updated_at"]=json!(chrono::Utc::now().to_rfc3339());
    let mut bytes=serde_json::to_vec_pretty(&profile)?;bytes.push(b'\n');
    ensure!(bytes.len() as u64<=MAX_BYTES,"profile exceeds size budget");
    pipeline::publish_replace(&p,&bytes)?;
    let(raw,b)=files::read_bound(&p)?;
    ensure!(raw==bytes&&b["sha256"]==files::digest(&bytes),"profile readback mismatch");
    let saved=crate::parse_json(std::str::from_utf8(&raw)?)?;
    Ok(json!({"schema_version":1,"status":"saved","profile":saved,"profile_binding":b,
        "profile_path":loc["profile_path"],"scope":"skill_local","generic_user_directory_used":false}))
}
pub fn patch(patch:Value,skill_root:Option<&Path>)->Result<Value>{
    ensure!(patch.is_object(),"profile patch must be an object");
    let current=if locate(skill_root)?.get("exists").and_then(Value::as_bool)==Some(true){
        load(skill_root,false)?["profile"].clone()
    }else{defaults()};
    let mut merged=current;
    merge(&mut merged,&patch);
    save(merged,skill_root)
}
pub fn arxiv_tex_enabled(profile:Option<&Value>)->bool{
    profile.and_then(|p|p["preferences"]["arxiv_tex_auto"].as_bool()).unwrap_or(true)
}
pub fn formula_table_mode(profile:Option<&Value>)->&str{
    profile.and_then(|p|p["preferences"]["formula_table_enhancement"].as_str()).unwrap_or("unasked")
}
pub fn ensure_profile_binding(profile:&Value)->Result<()>{
    validate(profile).map_err(|e|anyhow!("invalid persisted profile: {e:#}"))
}
fn slugify_item_dir(title:&str,key:&str)->String{
    let mut words=Vec::new();
    let mut cur=String::new();
    for ch in title.chars(){
        if ch.is_ascii_alphanumeric(){
            cur.push(ch.to_ascii_lowercase());
        }else if !cur.is_empty(){
            words.push(std::mem::take(&mut cur));
            if words.len()>=6{break;}
        }
    }
    if !cur.is_empty()&&words.len()<6{words.push(cur);}
    let mut slug=words.join("-");
    if slug.len()>48{slug.truncate(48);slug=slug.trim_end_matches('-').to_owned();}
    if slug.is_empty(){format!("item-{key}")}else{format!("{slug}-{key}")}
}
fn list_templates(template_root:Option<&str>)->Vec<String>{
    let Some(dir)=template_root.map(Path::new).filter(|p|p.is_dir()) else{return vec![];};
    let mut out=Vec::new();
    if let Ok(rd)=fs::read_dir(dir){
        for e in rd.flatten(){
            let p=e.path();
            if p.is_file()&&p.extension().and_then(|s|s.to_str()).is_some_and(|ext|ext.eq_ignore_ascii_case("md")){
                if let Some(name)=p.file_name().and_then(|s|s.to_str()){out.push(name.to_owned());}
            }
        }
    }
    out.sort();
    out
}
pub fn preflight(skill_root:Option<&Path>,query:Option<&str>,item_key:Option<&str>,storage_override:Option<&Path>,create_work_dirs:bool)->Result<Value>{
    let loaded=load(skill_root,true)?;
    let profile=&loaded["profile"];
    let api_url=profile["paths"]["zotero_local_api"].as_str().unwrap_or("http://127.0.0.1:23119/api");
    let zotero_probe=zotero_local::probe_api(api_url);
    let available_templates=list_templates(profile["paths"]["template_root"].as_str());
    let lib_obj=profile.get("zotero_library").filter(|v|v.is_object())
        .or_else(||zotero_probe.get("library").filter(|v|v.is_object()));
    let lib_type=lib_obj.and_then(|v|v["type"].as_str()).unwrap_or("user");
    let lib_id=lib_obj.and_then(|v|v["id"].as_u64()).unwrap_or(0);
    let storage_owned=profile["paths"]["zotero_storage_dir"].as_str().map(PathBuf::from);
    let storage=storage_override.or(storage_owned.as_deref()).filter(|p|p.is_dir());
    let work_root=profile["paths"]["work_root"].as_str();
    let vault_root=profile["paths"]["obsidian_vault_root"].as_str();
    let lit_rel=relative_under_vault(vault_root,profile["paths"]["literature_root"].as_str(),"文献");
    let assets_rel=relative_under_vault(vault_root,profile["paths"]["assets_resource_dir"].as_str(),&format!("{lit_rel}/attachments"));
    let mut selected_key=item_key.map(|s|s.trim().to_owned()).filter(|s|!s.is_empty());
    let mut search_summary=Value::Null;
    if zotero_probe["reachable"]==true{
        if let Ok(addr)=zotero_local::parse_loopback_api_addr(api_url){
            if selected_key.is_none(){
                if let Some(q)=query.map(str::trim).filter(|s|!s.is_empty()){
                    match zotero_local::search_items_at(addr,lib_type,lib_id,zotero_local::SearchOptions{
                        query:q,qmode:"titleCreatorYear",item_type:None,top_only:true,limit:5,start:0,
                    }){
                        Ok(res)=>{
                            let hits:Vec<Value>=res["items"].as_array().into_iter().flatten().map(|it|json!({
                                "key":it["key"],"itemType":it["data"]["itemType"],"title":it["data"]["title"],
                                "date":it["data"]["date"],"collections":it["data"]["collections"],
                            })).collect();
                            if hits.len()==1{selected_key=hits[0]["key"].as_str().map(str::to_owned);}
                            search_summary=json!({"returned_count":res["returned_count"],"total_results":res["total_results"],"hits":hits});
                        }
                        Err(e)=>{search_summary=json!({"error":format!("{e:#}")});}
                    }
                }
            }
            if let Some(key)=selected_key.as_deref(){
                let inspected=zotero_local::inspect_item_at(addr,lib_type,lib_id,key,storage,&json!({}),|_,_,_|{});
                match inspected{
                    Ok(rep)=>{
                        let data=&rep["raw"]["data"];
                        let title=data["title"].as_str().unwrap_or("");
                        let item_type=data["itemType"].as_str().unwrap_or("");
                        let suggested_dir=work_root.map(|wr|path_text(&Path::new(wr).join(slugify_item_dir(title,key))).unwrap_or_default());
                        let arxiv_id=crate::tex_source::detect_arxiv_id(&rep["raw"],None);
                        let matched_tpl=profile["templates"].as_object().and_then(|m|m.get(item_type).and_then(Value::as_str).map(str::to_owned));
                        let year=data["date"].as_str().and_then(extract_year);
                        let year_token=year.clone().unwrap_or_else(||"无年份".into());
                        let first_author=first_author_token(&data["creators"]);
                        let clean_first_author=clean_component(&first_author);
                        let filename_author=filename_author_token(&data["creators"]);
                        let clean_filename_author=clean_component(&filename_author);
                        let clean_title=clean_component(title.trim());
                        let suggested_filename=if clean_title.is_empty(){None}else{
                            Some(truncate_filename(&format!("{year_token}-{clean_filename_author}-"),&clean_title,".md",250))
                        };
                        let bucket=if title.trim().is_empty(){None}else{Some(title_bucket(title))};
                        let mut col_paths:Vec<String>=rep["collection_ancestry"]["collections"].as_array().into_iter().flatten().filter_map(|c|{
                            if c["status"]!="traced"{return None;}
                            let segs:Vec<String>=c["ancestry"].as_array().into_iter().flatten()
                                .filter_map(|node|node["name"].as_str().map(|s|clean_component(s.trim())))
                                .filter(|s|!s.is_empty()).collect();
                            if segs.is_empty(){None}else{Some(segs.join("/"))}
                        }).collect();
                        col_paths.sort();
                        let col_rel=if let Some(first)=col_paths.first(){
                            format!("{lit_rel}/{first}")
                        }else if rep["collection_ancestry"]["status"]=="complete"&&rep["collection_ancestry"]["collections"].as_array().is_some_and(Vec::is_empty){
                            format!("{lit_rel}/未分类")
                        }else{
                            lit_rel.clone()
                        };
                        let suggested_note_relative=suggested_filename.as_ref().map(|f|format!("{col_rel}/{f}"));
                        let suggested_asset_bucket_relative=bucket.as_ref().map(|b|format!("{assets_rel}/{year_token}/{clean_first_author}/{b}"));
                        let tpl_skeleton=extract_template_skeleton(profile["paths"]["template_root"].as_str(),matched_tpl.as_deref(),item_type);
                        let atts:Vec<Value>=rep["attachments"].as_array().into_iter().flatten().map(|a|json!({
                            "key":a["key"],"link_mode":a["link_mode"],"declared_mime":a["declared_mime"],
                            "filename":a["raw"]["data"]["filename"],"local":a["local"],
                        })).collect();
                        let pdf_count=atts.iter().filter(|a|a["declared_mime"].as_str().is_some_and(|m|m.contains("pdf"))||a["filename"].as_str().is_some_and(|f|f.to_ascii_lowercase().ends_with(".pdf"))).count();
                        let has_supplement=pdf_count>1||atts.iter().any(|a|a["filename"].as_str().is_some_and(|f|{
                            let low=f.to_ascii_lowercase();
                            low.contains("supp")||low.contains("appendix")||low.contains("-si")||low.contains("_si")
                        }));
                        let mut work_dirs=Value::Null;
                        if create_work_dirs{
                            if let Some(ref wd)=suggested_dir{
                                let wp=Path::new(wd);
                                if pipeline::ensure_outside_vault(&wp.join("probe")).is_ok(){
                                    let input=wp.join("input");
                                    let evidence=wp.join("evidence");
                                    let out_assets=wp.join("output").join("assets");
                                    let backups=wp.join("work").join("backups");
                                    for d in [&input,&evidence,&out_assets,&backups]{let _=fs::create_dir_all(d);}
                                    work_dirs=json!({
                                        "root":wd,
                                        "input":path_text(&input)?,
                                        "evidence":path_text(&evidence)?,
                                        "output_assets":path_text(&out_assets)?,
                                        "backups":path_text(&backups)?,
                                    });
                                }
                            }
                        }
                        return Ok(json!({
                            "schema_version":1,"status":loaded["status"],"onboarding_pending":loaded["onboarding_pending"],
                            "profile_path":loaded["profile_path"],"profile":profile,"validation":loaded["validation"],
                            "zotero_probe":zotero_probe,"available_templates":available_templates,"search":search_summary,
                            "item_inspection":{
                                "item_key":key,"itemType":item_type,"title":title,"date":data["date"],
                                "DOI":data["DOI"],"url":data["url"],"declared_language":rep["declared_language"],
                                "collection_ancestry":rep["collection_ancestry"],"arxiv_id":arxiv_id,
                                "selected_template":matched_tpl,"template_skeleton":tpl_skeleton,
                                "attachments":atts,"has_supplement_attachments":has_supplement,
                                "local_version_consistent":rep["local_version_consistent"],"raw":rep["raw"],
                            },
                            "naming_plan":{
                                "year":year,"year_token":year_token,"author_token":clean_filename_author,"first_author":first_author,
                                "filename":suggested_filename,"title_bucket":bucket,
                                "literature_relative":lit_rel,"assets_relative":assets_rel,
                                "collection_relative":col_rel,"suggested_note_relative":suggested_note_relative,
                                "suggested_asset_bucket_relative":suggested_asset_bucket_relative,
                            },
                            "suggested_item_work_dir":suggested_dir,
                            "work_dirs":work_dirs,
                            "generic_user_directory_used":false,
                        }));
                    }
                    Err(e)=>{
                        return Ok(json!({
                            "schema_version":1,"status":loaded["status"],"onboarding_pending":loaded["onboarding_pending"],
                            "profile_path":loaded["profile_path"],"profile":profile,"validation":loaded["validation"],
                            "zotero_probe":zotero_probe,"available_templates":available_templates,"search":search_summary,
                            "item_inspection_error":format!("{e:#}"),
                            "generic_user_directory_used":false,
                        }));
                    }
                }
            }
        }
    }
    Ok(json!({
        "schema_version":1,"status":loaded["status"],"onboarding_pending":loaded["onboarding_pending"],
        "profile_path":loaded["profile_path"],"profile":profile,"validation":loaded["validation"],
        "zotero_probe":zotero_probe,"available_templates":available_templates,"search":search_summary,
        "generic_user_directory_used":false,
    }))
}

#[cfg(test)]
mod tests{
    use super::*;
    fn root()->tempfile::TempDir{
        let d=tempfile::tempdir().unwrap();
        fs::write(d.path().join("SKILL.md"),"---\nname: literature-zotero2obsidian\nversion: v0\n---\n").unwrap();
        d
    }
    #[test]
    fn profile_is_skill_local_atomic_and_round_trips(){
        let d=root();let skill=d.path();
        let canonical_skill=files::canonical(skill).unwrap();
        let first=load(Some(skill),true).unwrap();
        assert_eq!(first["status"],"uninitialized");assert_eq!(first["onboarding_pending"],true);
        assert!(first["profile_path"].as_str().unwrap().starts_with(&path_text(&canonical_skill).unwrap()));
        let mut profile=first["profile"].clone();
        profile["paths"]["obsidian_vault_root"]=json!(path_text(&canonical_skill).unwrap());
        profile["preferences"]["formula_table_enhancement"]=json!("native");
        let saved=save(profile,Some(skill)).unwrap();
        assert_eq!(saved["status"],"saved");
        let again=load(Some(skill),true).unwrap();
        assert_eq!(again["status"],"configured");
        assert_eq!(again["onboarding_pending"],false);
        assert_eq!(again["validation"]["paths"]["obsidian_vault_root"]["status"],"ok");
        assert_eq!(again["profile"]["preferences"]["arxiv_tex_auto"],true);
    }
    #[test]
    fn patch_updates_only_given_preferences_and_rejects_bad_values(){
        let d=root();let skill=d.path();
        patch(json!({"preferences":{"formula_table_enhancement":"ocr"}}),Some(skill)).unwrap();
        let row=load(Some(skill),false).unwrap();
        assert_eq!(row["profile"]["preferences"]["formula_table_enhancement"],"ocr");
        assert_eq!(row["profile"]["preferences"]["transparent_figure"],"preserve_original");
        assert!(patch(json!({"preferences":{"formula_table_enhancement":"always"}}),Some(skill)).is_err());
    }
    #[test]
    fn missing_recorded_path_is_reported_for_targeted_refresh(){
        let d=root();let skill=d.path();
        let mut p=defaults();
        p["paths"]["literature_root"]=json!(skill.join("missing-root").to_string_lossy());
        save(p,Some(skill)).unwrap();
        let row=load(Some(skill),true).unwrap();
        assert_eq!(row["status"],"stale_paths");
        assert_eq!(row["validation"]["paths"]["literature_root"]["status"],"missing");
    }
    #[test]
    fn cli_command_is_valid_and_item_subdir_work_root_is_rejected(){
        let d=root();let skill=d.path();
        let vault=skill.join("vault");fs::create_dir(&vault).unwrap();
        let mut p=defaults();
        p["paths"]["obsidian_vault_root"]=json!(path_text(&vault).unwrap());
        p["paths"]["obsidian_cli"]=json!("obsidian");
        let work_base=skill.join("work-root");fs::create_dir(&work_base).unwrap();
        p["paths"]["work_root"]=json!(path_text(&work_base).unwrap());
        save(p.clone(),Some(skill)).unwrap();
        let loaded=load(Some(skill),true).unwrap();
        assert_eq!(loaded["status"],"configured");
        assert_eq!(loaded["validation"]["valid"],true);
        assert!(matches!(loaded["validation"]["paths"]["obsidian_cli"]["status"].as_str(),Some("ok"|"command_not_found")));

        let mut bad=p.clone();
        bad["paths"]["work_root"]=json!(path_text(&work_base.join("persistent-mayer-5U5RW3TA")).unwrap());
        assert!(save(bad,Some(skill)).is_err());

        let mut inside=p;
        inside["paths"]["work_root"]=json!(path_text(&vault.join("work")).unwrap());
        assert!(save(inside,Some(skill)).is_err());
    }
    #[test]
    fn sha1_bucket_locate_parent_root_and_author_year_helpers(){
        assert_eq!(sha1_hex(b"abc"),"a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(title_bucket("  Persistent Mayer path homology and Mayer path Laplacian\n"),"deebf3806b1cd4a8");
        assert_eq!(extract_year("2024-03-15"),Some("2024".into()));
        assert_eq!(first_author_token(&json!([{"creatorType":"author","firstName":"Hao","lastName":"Fan"}])),"Fan");
        assert_eq!(first_author_token(&json!([{"creatorType":"author","firstName":"三","lastName":"张"}])),"张三");
        assert_eq!(filename_author_token(&json!([])),"未知作者");
        assert_eq!(filename_author_token(&json!([{"creatorType":"author","firstName":"Hao","lastName":"Fan"}])),"Fan");
        assert_eq!(filename_author_token(&json!([
            {"creatorType":"author","firstName":"Dilan","lastName":"Karaguler"},
            {"creatorType":"author","firstName":"Guo-Wei","lastName":"Wei"}
        ])),"Karaguler and Wei");
        assert_eq!(filename_author_token(&json!([
            {"creatorType":"author","firstName":"Dilan","lastName":"Karaguler"},
            {"creatorType":"author","firstName":"Guo-Wei","lastName":"Wei"},
            {"creatorType":"author","firstName":"Jane","lastName":"Smith"}
        ])),"Karaguler et al");
        assert_eq!(filename_author_token(&json!([
            {"creatorType":"author","firstName":"三","lastName":"张"},
            {"creatorType":"author","firstName":"四","lastName":"李"}
        ])),"张三 and 李四");

        let repo=tempfile::tempdir().unwrap();
        let tpl_dir=repo.path().join("tpl");fs::create_dir(&tpl_dir).unwrap();
        fs::write(tpl_dir.join("preprint.md"),"<%*\nconst yaml=['---','类型: '+JSON.stringify(type),'年份: '+year,'关键词: '+JSON.stringify(profile.keywords||[])];if(d.itemType==='journalArticle')yaml.push('期刊名: '+JSON.stringify(d.publicationTitle||''));\n%>\n").unwrap();
        let sk_pre=extract_template_skeleton(tpl_dir.to_str(),Some("preprint.md"),"preprint");
        assert_eq!(sk_pre["template_kind"],"templater_js");
        assert_eq!(sk_pre["type_label"],"预印本");
        assert_eq!(sk_pre["frontmatter_keys"],json!(["类型","年份","关键词"]));
        let sk_journal=extract_template_skeleton(tpl_dir.to_str(),Some("preprint.md"),"journalArticle");
        assert_eq!(sk_journal["type_label"],"期刊论文");
        assert_eq!(sk_journal["frontmatter_keys"],json!(["类型","年份","关键词","期刊名"]));

        let nested_skill=repo.path().join(SKILL);
        fs::create_dir_all(&nested_skill).unwrap();
        fs::write(nested_skill.join("SKILL.md"),"---\nname: literature-zotero2obsidian\nversion: v0\n---\n").unwrap();
        let loc=locate(Some(repo.path())).unwrap();
        assert_eq!(loc["skill_root"],path_text(&files::canonical(&nested_skill).unwrap()).unwrap());
    }
}
