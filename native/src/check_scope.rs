//! Derive scopes only from explicitly selected source objects and actual
//! candidate text. This is neither source coverage nor semantic acceptance.
use crate::{files,path_text,parse_json,syntax,validate};
use anyhow::{ensure,Context,Result};
use regex::Regex;
use serde_json::{json,Map,Value};
use std::{collections::{HashMap,HashSet},path::Path,sync::OnceLock};

fn index<'a>(rows:&'a Value,key:&str,label:&str)->Result<HashMap<&'a str,&'a Value>>{
    let rows=rows.as_array().with_context(||format!("{label} must be an object sequence."))?;
    let mut out=HashMap::new();
    for row in rows{
        let id=row[key].as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("{label} requires unique nonempty {key} values."))?;
        ensure!(out.insert(id,row).is_none(),"{label} requires unique nonempty {key} values.");
    }Ok(out)
}
fn visible_anchors(text:&str)->HashMap<String,usize>{
    let lines:Vec<_>=text.strip_prefix('\u{feff}').unwrap_or(text).split('\n').collect();
    let first=validate::frontmatter(&lines,&mut Vec::new());
    let mut found=HashMap::new();
    let mut fence:Option<(u8,usize)>=None;
    let mut equation=false;let mut comment=None;let mut list_indent:Option<usize>=None;
    let mut code_lines:Vec<Option<String>>=vec![None;lines.len()];
    static INLINE:OnceLock<Regex>=OnceLock::new();
    let inline=INLINE.get_or_init(||Regex::new(r"^ {0,3}\$\$.+\$\$[ \t]*$").unwrap());
    for pos in first..lines.len(){
        let raw=lines[pos];
        if let Some((marker,width))=fence{if validate::closing(raw,marker,width){fence=None;}continue;}
        if equation{if raw.trim()=="$$"{equation=false;}continue;}
        let indent=validate::indentation(raw);
        let continuation=list_indent.is_some_and(|n|indent>=n);
        if !raw.trim().is_empty()&&!continuation{list_indent=None;}
        if comment.is_none()&&indent>=4&&!(continuation&&list_indent.is_some_and(|n|indent<n+4)){continue;}
        if comment.is_none(){if let Some((_,end))=validate::entry(raw){list_indent=Some(end);}}
        if comment.is_none(){if let Some((marker,width,_))=validate::fence(raw){fence=Some((marker,width));list_indent=None;continue;}}
        if code_lines[pos].is_none(){
            let mut end=pos+1;
            if !raw.trim().is_empty()&&validate::heading(raw).is_none(){
                while end<lines.len()&&!lines[end].trim().is_empty()&&validate::fence(lines[end]).is_none()&&
                    validate::heading(lines[end]).is_none()&&validate::entry(lines[end]).is_none(){end+=1;}
            }
            let masked=validate::mask_paragraph_code(&lines[pos..end].join("\n"),comment);
            for(i,line)in masked.split('\n').enumerate(){code_lines[pos+i]=Some(line.to_owned());}
        }
        let safe=code_lines[pos].as_deref().unwrap_or(raw);
        if comment.is_none()&&inline.is_match(safe){continue;}
        let (line,next)=validate::mask_comments(&validate::mask_delimited(safe,'$'),comment);comment=next;
        if let Some((marker,width,_))=validate::fence(&line){fence=Some((marker,width));continue;}
        if line.trim()=="$$"{equation=true;continue;}
        if let Some(id)=validate::anchor(&line){*found.entry(id.to_owned()).or_insert(0)+=1;}
    }found
}
fn box_valid(v:&Value)->bool{
    let Some(a)=v.as_array().filter(|a|a.len()==4)else{return false;};
    let values:Option<Vec<f64>>=a.iter().map(Value::as_f64).collect();
    values.is_some_and(|b|b.iter().all(|v|v.is_finite())&&b[0]<b[2]&&b[1]<b[3])
}
struct Targets<'a>{
    candidate:&'a str,lines:Vec<syntax::Line>,line_hashes:HashMap<(usize,usize),String>,anchors:Option<HashMap<String,usize>>,
}
impl<'a> Targets<'a>{
    fn new(candidate:&'a str)->Self{
        Self{candidate,lines:syntax::lines(candidate),line_hashes:HashMap::new(),anchors:None}
    }
    fn check(&mut self,target:&Value,id:&str)->Result<()>{
        let choices:Vec<_>=["anchor","quote","lines"].into_iter().filter(|k|target.get(*k).is_some()).collect();
        ensure!(choices.len()==1,"{id}: select exactly one candidate target: anchor, quote or lines.");
        match choices[0]{
            "quote"=>{
                let quote=target["quote"].as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("{id}: candidate quote must be nonempty and unique."))?;
                let first=self.candidate.find(quote).with_context(||format!("{id}: candidate quote must be nonempty and unique."))?;
                // Python searches from one Unicode character after the first
                // match, including overlapping repeated quotes.
                let next=first+self.candidate[first..].chars().next().unwrap().len_utf8();
                ensure!(!self.candidate[next..].contains(quote),"{id}: candidate quote must be nonempty and unique.");
            }
            "lines"=>{
                let bounds=target["lines"].as_array().filter(|a|a.len()==2)
                    .and_then(|a|a[0].as_u64().zip(a[1].as_u64()))
                    .and_then(|(a,b)|usize::try_from(a).ok().zip(usize::try_from(b).ok()))
                    .filter(|(a,b)|*a>0&&a<=b&&*b<=self.lines.len())
                    .with_context(||format!("{id}: candidate lines need a valid one-based inclusive range."))?;
                let digest=self.line_hashes.entry(bounds).or_insert_with(||{
                    let (a,b)=bounds;
                    files::digest(self.candidate[self.lines[a-1].start..self.lines[b-1].end].as_bytes())
                });
                ensure!(target["sha256"]==digest.as_str(),"{id}: candidate line-range sha256 does not match the actual text.");
            }
            _=>{
                let anchors=self.anchors.get_or_insert_with(||visible_anchors(self.candidate));
                ensure!(target["anchor"].as_str().is_some_and(|s|anchors.get(s)==Some(&1)),
                    "{id}: candidate anchor must name one visible block definition.");
            }
        }Ok(())
    }
}
pub fn from_deliveries(inventory_path:&Path,candidate_path:&Path,deliveries:&Value,unit_ids:&Value)->Result<Value>{
    let ids=unit_ids.as_array().filter(|v|!v.is_empty()).context("Select explicit source unit IDs; record an absent-object condition separately.")?;
    let mut unique=HashSet::new();
    for id in ids{
        let id=id.as_str().filter(|s|!s.trim().is_empty()).context("Every selected unit ID must be a nonempty string.")?;
        ensure!(unique.insert(id),"Do not repeat selected unit IDs.");
    }
    let (raw,inventory_binding)=files::read_bound(inventory_path)?;
    let inventory=parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(inventory.is_object(),"The source inventory must be an object.");
    let (raw,candidate_binding)=files::read_bound(candidate_path)?;
    let candidate=std::str::from_utf8(&raw)?.strip_prefix('\u{feff}').unwrap_or(std::str::from_utf8(&raw)?).replace("\r\n","\n").replace('\r',"\n");
    let candidate_path=files::canonical(candidate_path)?;
    let units=index(&inventory["units"],"id","units")?;
    let sources=index(&inventory["sources"],"id","sources")?;
    let deliveries=index(deliveries,"unit","deliveries")?;
    let mut targets=Targets::new(&candidate);
    let mut scopes=Vec::with_capacity(ids.len());
    for id in ids{
        let identity=id.as_str().unwrap();
        let unit=units.get(identity).with_context(||format!("Selected objects lack source units or delivery locators: {identity}"))?;
        let target=&deliveries.get(identity).with_context(||format!("Selected objects lack source units or delivery locators: {identity}"))?["target"];
        let source_id=unit["source"].as_str().filter(|s|sources.contains_key(s)).with_context(||format!("{identity}: unknown source identity."))?;
        let source=sources[source_id];
        let locations=unit["evidence"].as_array().filter(|a|!a.is_empty()&&a.iter().all(Value::is_object))
            .with_context(||format!("{identity}: explicit source locations are required."))?;
        let mut source_locations=Vec::with_capacity(locations.len());
        if source["format"]=="docx"{
            // Extend the old PDF-only helper for native documents without
            // inventing page coordinates; retain the exact XML locator.
            for loc in locations{
                let errors=crate::docx::evidence_errors(source,loc);
                ensure!(errors.is_empty(),"{identity}: {}",errors.join(" "));
                source_locations.push(loc.clone());
            }
        }else{
            let page_count=source["page_count"].as_u64().filter(|n|*n>0).with_context(||format!("{identity}: the source needs a positive inventory page_count."))?;
            for loc in locations{
                ensure!(loc["page"].as_u64().is_some_and(|n|n>0&&n<=page_count),"{identity}: source page must fall within inventory page_count.");
                ensure!(["bbox","quote","image_xref"].iter().any(|k|loc.get(k).is_some()),"{identity}: each source location needs an object locator.");
                if let Some(b)=loc.get("bbox"){ensure!(box_valid(b),"{identity}: source bbox must be a finite nonempty rectangle.");}
                if let Some(q)=loc.get("quote"){ensure!(q.as_str().is_some_and(|s|!s.trim().is_empty()),"{identity}: source quote must be a nonempty string.");}
                if let Some(x)=loc.get("image_xref"){ensure!(x.as_u64().is_some_and(|n|n>0),"{identity}: source image_xref must be a positive integer.");}
                let filtered:Map<String,Value>=["page","bbox","coordinate_system","image_xref","quote"].into_iter()
                    .filter_map(|key|loc.get(key).map(|v|(key.to_owned(),v.clone()))).collect();
                source_locations.push(Value::Object(filtered));
            }
        }
        targets.check(target,identity)?;
        scopes.push(json!({"object_id":identity,"locator":{"source_id":source_id,"source_locations":source_locations,
            "candidate_path":path_text(&candidate_path)?,"candidate_target":target}}));
    }
    for binding in [&inventory_binding,&candidate_binding]{
        let current=files::snapshot(Path::new(binding["path"].as_str().unwrap()))?;
        ensure!(current["sha256"]==binding["sha256"],"Source inventory or candidate changed while deriving scopes");
    }
    Ok(json!(scopes))
}
