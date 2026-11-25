//! Apply only explicitly observed readings to exact source spans. The original
//! file is untouched, and the recorded observation is never certified.
use crate::{files,path,path_text};
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::{HashMap,HashSet},fs,path::{Path,PathBuf}};

fn text<'a>(v:&'a Value,label:&str,allow_empty:bool)->Result<&'a str>{
    let s=v.as_str().with_context(||format!("{label} must be a string"))?;
    ensure!(allow_empty||!s.trim().is_empty(),"{label} must not be empty");Ok(s)
}
fn bind(record:&Value,cache:&mut HashMap<PathBuf,String>)->Result<Value>{
    ensure!(record.is_object(),"file evidence must contain path and sha256");
    let expected=text(&record["sha256"],"sha256",false)?;
    ensure!(expected.len()==64&&expected.bytes().all(|b|b.is_ascii_hexdigit()),"file evidence requires a SHA-256 digest");
    let p=files::canonical(&path(&record["path"])?)?;
    let hash=if let Some(h)=cache.get(&p){h.clone()}else{
        let h=files::snapshot(&p)?["sha256"].as_str().context("file hash")?.to_owned();
        cache.insert(p.clone(),h.clone());h
    };
    ensure!(hash==expected.to_ascii_lowercase(),"evidence file SHA-256 mismatch: {}",p.display());
    Ok(json!({"path":path_text(&p)?,"sha256":hash}))
}
pub fn apply(file:&Value,edits:&Value)->Result<Value>{
    let mut cache=HashMap::new();let binding=bind(file,&mut cache)?;
    let raw=fs::read(Path::new(binding["path"].as_str().unwrap()))?;
    ensure!(files::digest(&raw)==binding["sha256"].as_str().unwrap(),"text snapshot changed while being read");
    let original=std::str::from_utf8(&raw)?;
    let offsets:Vec<usize>=original.char_indices().map(|(i,_)|i).chain(std::iter::once(original.len())).collect();
    let edits=edits.as_array().context("edits must be a sequence of explicit edit records")?;
    let mut prepared=Vec::with_capacity(edits.len());
    let mut ids=HashSet::new();
    for edit in edits{
        ensure!(edit.is_object(),"each edit must be a mapping");
        let id=text(&edit["id"],"edit.id",false)?;
        ensure!(ids.insert(id),"duplicate edit ID: {id}");
        let (start,end)=edit["start"].as_u64().zip(edit["end"].as_u64())
            .and_then(|(a,b)|usize::try_from(a).ok().zip(usize::try_from(b).ok()))
            .filter(|(a,b)|a<b&&*b<offsets.len()).context("edit range must be a nonempty original-text code-point range")?;
        let expected=text(&edit["expected_text"],"expected_text",true)?;
        ensure!(&original[offsets[start]..offsets[end]]==expected,"stale or incorrect original text for edit: {id}");
        let visible=&edit["visible_page"];
        ensure!(visible.is_object(),"an edit requires visible-page evidence");
        let page_file=bind(&visible["file"],&mut cache)?;
        let page=visible["page"].as_u64().filter(|n|*n>0).context("visible_page.page must be a positive integer")?;
        let bbox=visible["bbox"].as_array().filter(|a|a.len()==4).context("visible_page.bbox requires four coordinates")?;
        let numbers:Option<Vec<f64>>=bbox.iter().map(Value::as_f64).collect();
        let b=numbers.filter(|v|v.iter().all(|x|x.is_finite())).context("visible_page.bbox coordinates must be finite numbers")?;
        ensure!(b[0]>=0.0&&b[1]>=0.0&&b[0]<b[2]&&b[1]<b[3],"visible_page.bbox must have positive area in nonnegative coordinates");
        let coords=&visible["coordinate_system"];
        ensure!(coords.as_str().is_some_and(|s|!s.trim().is_empty())||coords.as_object().is_some_and(|m|!m.is_empty()),
            "visible_page.coordinate_system must be explicitly recorded");
        let reading=text(&visible["text"],"visible_page.text",true)?;
        let decision=&edit["decision"];
        ensure!(decision.is_object()&&decision["status"]=="resolved","only a caller-resolved decision can be applied");
        ensure!(decision["observed"]==true,"the caller must explicitly declare that the visible page was observed");
        let selected=text(&decision["selected_text"],"decision.selected_text",true)?;
        let reason=text(&decision["reason"],"decision.reason",false)?;
        let reviewer=text(&decision["reviewer"],"decision.reviewer",false)?;
        ensure!(selected==reading,"selected_text must equal the caller's recorded visible reading");
        prepared.push((start,end,selected,json!({"id":id,"text_layer":{"file":binding,
            "locator":{"kind":"characters","unit":"unicode_code_points","start":start,"end":end,"end_exclusive":true},"text":expected},
            "visible_page":{"file":page_file,"page":page,"bbox":bbox,"coordinate_system":coords,"text":reading},
            "decision":{"status":"resolved","selected_text":selected,"reason":reason,"reviewer":reviewer,"observed":true,"declaration_origin":"caller"}})));
    }
    prepared.sort_by_key(|row|row.0);
    let mut result=String::with_capacity(original.len());let mut cursor=0usize;let mut conflicts=Vec::new();
    for (start,end,selected,record) in prepared{
        ensure!(start>=cursor,"edit ranges overlap in the original text");
        result.push_str(&original[offsets[cursor]..offsets[start]]);
        result.push_str(selected);cursor=end;conflicts.push(record);
    }
    result.push_str(&original[offsets[cursor]..]);
    for(p,hash)in cache{ensure!(files::snapshot(&p)?["sha256"]==hash,"evidence changed while preparing edits");}
    Ok(json!({"text":result,"text_file":binding,"text_visual_conflicts":conflicts,
        "observation_certified":false,"semantic_review":{"status":"not_assessed"},
        "boundary":"Applies caller-declared readings only; hashes bind files, not observation or semantic correctness."}))
}
