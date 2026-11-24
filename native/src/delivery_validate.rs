//! Native read-only delivery audit. Uses real input bytes and existing
//! validators; all unresolved comparisons remain findings. This candidate
//! does not certify the complete historical PDF/figure/translation contract.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Map,Value};
use std::{collections::{BTreeMap,BTreeSet,HashSet},path::{Path,PathBuf},fs};
use crate::{check_ledger,check_validate,delivery_provenance,docx,files,grid,handoff_native::{self,Bindings},
    handoff_scan::{Candidate,Span},path,pdf_structure::Pdf,source_urls,syntax,translation_validate,validate};

const BOUNDARY:&str="Native delivery record and file consistency checks; source fidelity, actual observation, scientific meaning and independent visual review are not certified. Full historical parity remains a release blocker.";
fn text(v:&Value)->bool{v.as_str().is_some_and(|s|!s.trim().is_empty())}
fn list<'a>(v:&'a Value,label:&str)->Result<&'a Vec<Value>>{v.as_array().filter(|a|a.iter().all(Value::is_object)).with_context(||format!("{label}: array of objects required"))}
fn index<'a>(v:&'a Value,key:&str)->Result<BTreeMap<String,&'a Value>>{
    let mut m=BTreeMap::new();
    for r in list(v,key)?{
        let k=r[key].as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("Missing {key}"))?;
        ensure!(m.insert(k.to_owned(),r).is_none(),"Duplicate {key}: {k}");
    }Ok(m)
}
fn norm(s:&str)->String{s.split_whitespace().collect::<Vec<_>>().join(" ")}
fn source_mailto(href:&str,address:&str)->bool{
    let Some((scheme,rest))=href.split_once(':')else{return false;};
    if !scheme.eq_ignore_ascii_case("mailto"){return false;}
    // Strip URI headers before decoding. A recipient supplied only in ?to=
    // (or another header) cannot replace the recorded source recipient.
    let recipient=rest.split('?').next().unwrap_or("");
    percent_encoding::percent_decode_str(recipient).decode_utf8()
        .is_ok_and(|decoded|decoded.eq_ignore_ascii_case(address))
}
fn ids(v:&Value)->Result<BTreeSet<&str>>{
    let a=v.as_array().context("Explicit ID list required")?;let mut result=BTreeSet::new();
    for i in a{ensure!(result.insert(i.as_str().filter(|s|!s.is_empty()).context("Invalid ID")?),"Duplicate ID");}Ok(result)
}
fn coord()->Value{json!({"space":"pdf_page","unit":"pt","origin":"top_left","x_axis":"right","y_axis":"down","rotation":"unrotated"})}
struct Audit{findings:Vec<Value>,bindings:Bindings,svg_assets:BTreeMap<PathBuf,std::result::Result<Value,String>>,svg_bytes:usize}
impl Audit{
    fn fail(&mut self,code:&str,id:&str,msg:impl AsRef<str>){self.findings.push(json!({"code":code,"object_id":id,"message":msg.as_ref(),"level":"error","line":null}));}
    fn result(&mut self,code:&str,id:&str,r:Result<()>){if let Err(e)=r{self.fail(code,id,format!("{e:#}"));}}
    fn import(&mut self,v:&Value,prefix:&str){
        if let Some(a)=v["findings"].as_array(){for f in a{
            if f["level"]!="error"{continue;}
            self.fail(&format!("{prefix}{}",f["code"].as_str().unwrap_or("VALIDATION")),f["object_id"].as_str().unwrap_or("record"),f["message"].as_str().unwrap_or("Unresolved validation finding"));
        }}
    }
    fn file(&mut self,row:&Value,base:&Path)->Result<Value>{self.bindings.recorded(row,base)}
    fn ledger(&mut self,inv:&Value,manifest:&Value,ib:&Path,mb:&Path,key:&str,pointer:&str)->Result<Option<(PathBuf,Value)>>{
        if manifest.get(key).is_none()&&(inv.get(pointer).is_none()||key=="terminology"&&(inv[pointer].is_null()||inv[pointer]=="")){return Ok(None);}
        let p=files::canonical(&ib.join(path(&inv[pointer])?))?;
        let b=self.file(&manifest[key],mb)?;
        ensure!(path(&b["path"])?==p,"{key}: bind the exact ledger named by the inventory");
        ensure!(fs::metadata(&p)?.len()<=128*1024*1024,"Ledger exceeds input budget");
        let (raw,actual)=files::read_bound(&p)?;
        ensure!(b==actual&&raw.len()<=128*1024*1024,"Ledger changed");
        let v=crate::parse_json(std::str::from_utf8(&raw)?.trim_start_matches('\u{feff}'))?;
        ensure!(v.is_object()&&(v["schema_version"].as_u64()==Some(1)||key=="figure_reviews"&&v["schema_version"].as_u64()==Some(2)),"Invalid ledger schema");
        Ok(Some((p,v)))
    }
}
fn evidence(a:&mut Audit,source:&Value,sid:&str,loc:&Value,visual:bool,kind:&str,pdf:Option<&Pdf>,base:&Path,
    used_blocks:&mut BTreeSet<String>,used_pages:&mut BTreeSet<(String,u64)>,used_media:&mut BTreeSet<(String,String)>)->Result<()>{
    if source["format"]=="docx"{
        let errors=docx::evidence_errors(source,loc);ensure!(errors.is_empty(),"{}",errors.join(" "));
        for b in loc["blocks"].as_array().unwrap(){used_blocks.insert(b.as_str().unwrap().to_owned());}
        if let Some(c)=loc["media_call"].as_str(){
            ensure!(kind=="figure","A native image call must map to a figure unit");
            used_media.insert((sid.into(),c.into()));
        }else{ensure!(kind!="figure","Native figure requires an actual image call");}
        return Ok(());
    }
    let pdf=pdf.context("Actual PDF source unavailable")?;
    let n=loc["page"].as_u64().filter(|n|*n>0&&*n<=pdf.len() as u64).context("Unknown physical source page")?;
    let pages=source["pages"].as_array().context("Source pages required")?;
    let rows:Vec<_>=pages.iter().filter(|r|r["page"].as_u64()==Some(n)).collect();
    ensure!(rows.len()==1,"Source page absent or duplicated");
    let row=rows[0];let basis=loc["basis"].as_str().context("Explicit evidence basis required")?;
    ensure!(matches!(basis,"text"|"visual")&&(!visual||basis=="visual"),"Visible source evidence is required for this unit");
    let mut has_locator=text(&loc["quote"]);
    if !loc["bbox"].is_null(){
        ensure!(pdf.contains_bbox(n,&loc["bbox"])?,"Source bbox outside actual page");
        if source["inventory_backend"]==crate::pdf_inventory::BACKEND{ensure!(loc["coordinate_system"]==coord(),"Declare native unrotated PDF coordinates");}
        has_locator=true;
    }
    if let Some(x)=loc.get("image_xref").filter(|v|!v.is_null()){
        ensure!(kind=="figure","Image XRef locator requires a figure unit");
        let x=u32::try_from(x.as_u64().context("Actual XRef required")?)?;
        ensure!(pdf.resource_image_xrefs(n)?.contains(&x),"Image XRef does not belong to this page");
        has_locator=true;
    }
    ensure!(has_locator,"Explicit source quote, bbox or image XRef required");
    let blocks=index(&row["blocks"],"id")?;
    if basis=="text"&&text(&loc["quote"]){
        let reading=row["blocks"].as_array().unwrap().iter().map(|b|b["text"].as_str().unwrap_or("")).collect::<Vec<_>>().join("\n");
        ensure!(norm(&reading).contains(&norm(loc["quote"].as_str().unwrap())),"Text quote is absent from registered page text");
        a.file(&row["text"],base)?;
    }
    if basis=="visual"{a.file(&row["render"],base)?;}
    if let Some(b)=loc.get("blocks"){for id in ids(b)?{ensure!(blocks.contains_key(id),"Block does not belong to this source page");used_blocks.insert(id.into());}}
    used_pages.insert((sid.into(),n));Ok(())
}
fn inventory_checks(a:&mut Audit,inventory:&Value,base:&Path)->Result<()>{
    let sources=index(&inventory["sources"],"id")?;let units=index(&inventory["units"],"id")?;
    ensure!(!sources.is_empty()&&!units.is_empty(),"Classify the actual source contents before validation");
    let mut pdfs=BTreeMap::<String,Pdf>::new();
    let mut used_blocks=BTreeSet::new();let mut used_pages=BTreeSet::new();let mut used_media=BTreeSet::new();
    let mut all_blocks=BTreeSet::new();let mut all_pages=BTreeSet::new();let mut all_media=BTreeMap::new();
    for(sid,s)in &sources{
        let b=a.file(s,base)?;
        ensure!(matches!(s["role"].as_str(),Some("main"|"supplement")),"Source role required");
        if s["format"]=="docx"{
            for id in index(&s["native_blocks"],"id")?.keys(){ensure!(all_blocks.insert(id.clone()),"Duplicate source block");}
            if let Some(calls)=s.get("media_calls"){for(id,c)in index(calls,"id")?{all_media.insert((sid.clone(),id),c["block_id"].as_str().context("Native media parent")?.to_owned());}}
        }else{
            let p=Pdf::open(&path(&b["path"])?)?;
            let rows=list(&s["pages"],"PDF pages")?;
            ensure!(rows.len()==p.len(),"Source inventory page count differs");
            for(i,row)in rows.iter().enumerate(){
                ensure!(row["page"].as_u64()==Some((i+1) as u64),"Keep every actual physical page");
                all_pages.insert((sid.clone(),(i+1) as u64));
                for id in index(&row["blocks"],"id")?.keys(){ensure!(all_blocks.insert(id.clone()),"Duplicate source block");}
            }pdfs.insert(sid.clone(),p);
        }
    }
    for(id,u)in &units{
        let r=(||->Result<()>{
            let sid=u["source"].as_str().context("Source ID")?;let s=*sources.get(sid).context("Unknown source")?;
            let kind=u["kind"].as_str().context("Source unit kind")?;
            let mut fields=u.as_object().unwrap().clone();
            for key in ["id","source","kind","evidence"]{fields.remove(key);}
            crate::delivery_prepare::unit(id,sid,kind,&u["evidence"],fields)?;
            if kind=="metadata"&&u["field"]=="author"{
                let affiliations=u["affiliation_units"].as_array().context("Record actual author-to-affiliation units")?;
                let numbers=u["affiliation_numbers"].as_array().filter(|n|n.len()==affiliations.len()).context("Retain parallel source markers and affiliation units")?;
                ensure!(numbers.iter().all(|n|n.is_null()||text(n)),"Preserve literal source affiliation markers or null");
                if affiliations.is_empty()||numbers.iter().any(Value::is_null){ensure!(text(&u["affiliation_reason"]),"Explain absent or unnumbered author affiliations");}
                let seen=ids(&u["affiliation_units"])?;
                for id in seen{
                    ensure!(units.get(id).is_some_and(|v|v["kind"]=="metadata"&&v["field"]=="affiliation"&&v["source"]==sid),"Author affiliation must be an actual same-source affiliation unit");
                }
            }
            for loc in list(&u["evidence"],"source evidence")?{
                evidence(a,s,sid,loc,matches!(kind,"metadata"|"figure"|"table"|"formula"),kind,pdfs.get(sid),base,&mut used_blocks,&mut used_pages,&mut used_media)?;
            }
            if kind=="figure"&&u["contains_table"]==true{
                ensure!(units.values().any(|v|v["kind"]=="table"&&v["source"]==sid&&v["parent_id"]==id.as_str()),"Embedded table requires separate editable unit");
            }
            if let Some(parent)=u.get("parent_id").filter(|v|!v.is_null()){
                ensure!(kind=="table"&&parent.as_str().and_then(|p|units.get(p)).is_some_and(|v|v["source"]==sid&&v["kind"]=="figure"),"Invalid embedded table parent");
            }
            if s["role"]=="supplement"&&matches!(kind,"figure"|"table"){
                let original=&u["original_number"];let display=u["display_number"].as_str().context("Supplement navigation number required")?;
                if let Some(o)=original.as_str(){
                    let expected=if o.to_ascii_lowercase().starts_with('s'){o.to_owned()}else{format!("S{o}")};
                    ensure!(display==expected,"Supplement source number changed");
                }else{ensure!(display.to_ascii_lowercase().starts_with('s')&&text(&u["original_number_reason"]),"Unnumbered supplement requires explained navigation number");}
            }
            Ok(())
        })();a.result("SOURCE_UNIT",id,r);
    }
    for(sid,s)in &sources{
        for(field,kind)in [("title","title"),("authors","author"),("affiliations","affiliation"),("publication_date","publication_date")]{
            let d=&s["header_fields"][field];let label=format!("{sid}:{field}");
            let expected:BTreeSet<_>=units.iter().filter(|(_,u)|u["source"]==sid.as_str()&&u["kind"]=="metadata"&&u["field"]==kind).map(|(id,_)|id.as_str()).collect();
            let r=(||->Result<()>{
                match d["status"].as_str(){
                    Some("present")=>ensure!(!expected.is_empty()&&ids(&d["units"])?==expected,"Header metadata unit coverage differs"),
                    Some("absent_in_source")=>{
                        ensure!(expected.is_empty()&&text(&d["reason"]),"Header absence conflicts or lacks source reason");
                        let rows=list(&d["evidence"],"absence evidence")?;ensure!(!rows.is_empty(),"Absence needs actual source location");
                        for loc in rows{evidence(a,s,sid,loc,true,"metadata",pdfs.get(sid),base,&mut used_blocks,&mut used_pages,&mut used_media)?;}
                    },
                    _=>anyhow::bail!("Resolve this field from the actual source"),
                }Ok(())
            })();a.result("HEADER_DECISION",&label,r);
        }
    }
    let mut excluded_blocks=BTreeSet::new();let mut excluded_pages=BTreeSet::new();let mut excluded_media=BTreeSet::new();
    if let Some(v)=inventory.get("excluded_blocks"){for r in list(v,"excluded_blocks")?{
        let b=r["block"].as_str().context("Excluded block ID")?;
        ensure!(all_blocks.contains(b)&&!used_blocks.contains(b)&&text(&r["reason"])&&excluded_blocks.insert(b.to_owned()),"Invalid or conflicting block exclusion");
    }}
    if let Some(v)=inventory.get("excluded_pages"){for r in list(v,"excluded_pages")?{
        let key=(r["source"].as_str().context("Excluded source")?.to_owned(),r["page"].as_u64().context("Excluded page")?);
        ensure!(all_pages.contains(&key)&&!used_pages.contains(&key)&&text(&r["reason"])&&excluded_pages.insert(key),"Invalid or conflicting page exclusion");
    }}
    if let Some(v)=inventory.get("excluded_media_calls"){for r in list(v,"excluded_media_calls")?{
        let key=(r["source"].as_str().context("Excluded source")?.to_owned(),r["media_call"].as_str().context("Excluded call")?.to_owned());
        ensure!(all_media.contains_key(&key)&&!used_media.contains(&key)&&text(&r["reason"])&&excluded_media.insert(key),"Invalid or conflicting media exclusion");
    }}
    for b in all_blocks.difference(&used_blocks){if !excluded_blocks.contains(b){a.fail("SOURCE_BLOCK_UNMAPPED",b,"Map this block or record its justified exclusion.");}}
    for(sid,p)in all_pages.difference(&used_pages){if !excluded_pages.contains(&(sid.clone(),*p)){a.fail("SOURCE_PAGE_UNMAPPED",&format!("{sid}:page-{p}"),"Record source content or an explicit page exclusion.");}}
    for(key,block)in all_media{if !used_media.contains(&key)&&!excluded_media.contains(&key)&&!excluded_blocks.contains(&block){a.fail("NATIVE_MEDIA_UNMAPPED",&key.1,"Map the actual native image call or explicitly exclude it.");}}
    let url=source_urls::coverage(inventory,&json!(excluded_blocks),&json!(excluded_pages))?;
    for row in url["unmapped"].as_array().unwrap(){a.fail("SOURCE_URL_UNMAPPED",row["id"].as_str().unwrap_or("url"),"Register the actual source URL occurrence.");}
    for row in url["invalid_bindings"].as_array().unwrap(){a.fail("SOURCE_URL_BINDING",row["unit"].as_str().unwrap_or("url"),row["reason"].as_str().unwrap_or("Invalid occurrence binding"));}
    Ok(())
}
fn asset_from(a:&mut Audit,b:&Value,base:&Path,root:&Path)->Result<Value>{
    let p=files::canonical(&base.join(path(&b["path"])?))?;
    ensure!(p.starts_with(root),"FIGURE_RESOURCE_OUTSIDE_ROOT: {}",p.display());
    a.file(&json!({"path":crate::path_text(&p)?,"sha256":b["sha256"]}),root)
}
fn asset(a:&mut Audit,b:&Value,root:&Path)->Result<Value>{
    asset_from(a,b,root,root)
}
fn local_image(a:&mut Audit,href:&str,root:&Path)->Result<Value>{
    let raw=percent_encoding::percent_decode_str(href).decode_utf8()?;
    ensure!(!regex::Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:")?.is_match(&raw)&&!raw.starts_with("//"),"Local image required");
    let p=files::canonical(&root.join(raw.as_ref()))?;
    ensure!(p.starts_with(root),"FIGURE_RESOURCE_OUTSIDE_ROOT: {}",p.display());
    if p.extension().and_then(|ext|ext.to_str()).is_some_and(|ext|ext.eq_ignore_ascii_case("svg")){
        use std::io::Read;
        if let Some(b)=a.svg_assets.get(&p){return b.clone().map_err(anyhow::Error::msg);}
        ensure!(a.svg_assets.len()<32,"FIGURE_SVG_BUDGET: too many unique SVG assets in one delivery");
        let checked=(||->Result<Value>{
            let meta=fs::metadata(&p)?;
            ensure!(meta.is_file()&&meta.len()<=crate::svg_asset::MAX_BYTES as u64,"FIGURE_SVG_BUDGET: byte limit exceeded");
            let mut file=fs::File::open(&p)?;
            let meta=file.metadata()?;
            ensure!(meta.is_file()&&meta.len()<=crate::svg_asset::MAX_BYTES as u64,"FIGURE_SVG_BUDGET: byte limit exceeded");
            let mut bytes=Vec::new();
            file.by_ref().take(crate::svg_asset::MAX_BYTES as u64+1).read_to_end(&mut bytes)?;
            a.svg_bytes+=bytes.len();
            ensure!(a.svg_bytes<=8*crate::svg_asset::MAX_BYTES,"FIGURE_SVG_BUDGET: aggregate byte limit exceeded");
            crate::svg_asset::validate(&bytes)?;
            let b=a.bindings.capture(&p)?;
            ensure!(b["sha256"]==files::digest(&bytes),"SVG asset bytes changed during validation");
            Ok(b)
        })().map_err(|e|format!("{e:#}"));
        a.svg_assets.insert(p,checked.clone());
        return checked.map_err(anyhow::Error::msg);
    }
    let b=a.bindings.capture(&p)?;
    ensure!(fs::metadata(&p)?.len()<=128*1024*1024,"Image exceeds decode budget");
    let mut reader=image::ImageReader::open(&p)?.with_guessed_format()?;
    let mut limits=image::Limits::default();limits.max_image_width=Some(20_000);limits.max_image_height=Some(20_000);limits.max_alloc=Some(256*1024*1024);
    reader.limits(limits);
    let image=reader.decode().context("Image format/codec must be supported by the actual native decoder")?;
    ensure!(image.width()>0&&image.height()>0&&u64::from(image.width())*u64::from(image.height())<=40_000_000,"Invalid image dimensions");
    Ok(b)
}
fn header_note_layout(candidate:&Candidate,span:Span)->Result<bool>{
    // Selectors may start mid-line, but a quote marker only applies at the
    // actual physical line start. Use the prose view to exclude literal
    // markers inside comments, code, Rich Table data or display math.
    let start=candidate.raw[..span.start].rfind(['\r','\n']).map_or(0,|i|i+1);
    let selected=candidate.visible_prose(span)?;
    if !selected.chars().any(|c|!c.is_whitespace()&&c!='>'){return Ok(false);}
    let prose=candidate.visible_prose(Span{start,end:span.end})?;
    for row in syntax::lines(prose){
        let line=syntax::body(prose,row);
        if line.trim().is_empty(){continue;}
        let rest=line.trim_start_matches(' ');
        if line.len()-rest.len()>3||!rest.starts_with('>'){return Ok(false);}
    }
    Ok(true)
}
fn targets(a:&mut Audit,inv:&Value,manifest:&Value,candidate:&Candidate,root:&Path)->Result<()>{
    let units=index(&inv["units"],"id")?;let deliveries=index(&manifest["deliveries"],"unit")?;
    ensure!(units.keys().eq(deliveries.keys()),"Every registered unit needs one explicit target");
    let tables=candidate.table_records()?;
    let mut covered=BTreeSet::new();let mut refs=BTreeSet::new();let mut bibliography=BTreeMap::<String,String>::new();let mut calls=Vec::<(String,Span)>::new();
    let formula=regex::Regex::new(r"(?ms)^ {0,3}\$\$[ \t]*(?:[^\r\n]+\$\$[ \t]*$|\r?\n.+?\r?\n {0,3}\$\$[ \t]*$)")?;
    let heading=regex::Regex::new(r"(?m)^ {0,3}#{1,6}[ \t]+\S")?;
    for(id,u)in &units{
        let d=deliveries[id];let kind=u["kind"].as_str().context("Unit kind")?;
        let r=(||->Result<()>{
            let span=candidate.resolve(&d["target"],kind)?;
            let visible=candidate.visible_target(span)?;
            let links:Vec<_>=candidate.links.iter().filter(|l|span.contains(l.span)).collect();
            let anchors:Vec<_>=candidate.anchors.iter().filter(|(_,s)|span.contains(**s)).collect();
            if kind=="figure"{
                let mut embedded=BTreeSet::new();
                for l in &links{if l.image{embedded.insert(local_image(a,&l.href,root)?["path"].as_str().unwrap().to_owned());}}
                ensure!(!embedded.is_empty(),"Figure needs local image embeds");
                let mut recorded=BTreeSet::new();
                for row in list(&d["assets"],"figure assets")?{
                    let b=asset(a,row,root)?;
                    ensure!(recorded.insert(b["path"].as_str().unwrap().to_owned()),"Duplicate figure asset");
                }
                ensure!(embedded==recorded,"Figure asset mapping differs from actual embeds");
            }
            if kind=="table"{
                let matches:Vec<_>=tables.iter().enumerate().filter(|(_,t)|span.contains(t.0)).collect();
                ensure!(matches.len()==1,"Select one actual editable Rich Table fence");
                let (block,(_,data))=matches[0];
                let (sheet_idx,sheet)=if data["version"]==3{
                    let selector=d["sheet"].as_object().filter(|s|s.len()==1).context("Select actual worksheet by id or name")?;
                    let(k,v)=selector.iter().next().unwrap();ensure!(matches!(k.as_str(),"id"|"name")&&text(v),"Invalid worksheet selector");
                    let found:Vec<_>=list(&data["sheets"],"sheets")?.iter().enumerate().filter(|(_,s)|s[k]==*v).collect();
                    ensure!(found.len()==1,"Worksheet absent or ambiguous");found[0]
                }else{ensure!(data["version"]==2,"Unsupported table version");(0,data)};
                ensure!(covered.insert((block,sheet_idx)),"Worksheet delivered more than once");
                for k in ["rows","columns"]{ensure!(sheet[k].as_array().is_some_and(|rows|u["shape"][k].as_u64()==Some(rows.len() as u64)),"Table {k} count differs from source");}
                for f in grid::check_merges(id,Some(u),sheet){a.fail(f.code,id,f.message);}
            }
            if kind=="formula"{ensure!(formula.is_match(visible),"Formula requires displayed editable LaTeX");}
            if kind=="section"{ensure!(heading.is_match(visible),"Section requires visible heading");}
            if kind=="reference"{
                ensure!(anchors.len()==1,"Reference needs exactly one bibliography anchor");
                ensure!(bibliography.insert(anchors[0].0.clone(),id.clone()).is_none(),"Distinct bibliography units cannot claim the same candidate anchor");
            }
            if kind=="citation"{calls.push((id.clone(),span));}
            if matches!(kind,"figure"|"table"|"formula"|"reference"){for (id,_)in anchors{refs.insert(id.clone());}}
            if kind=="metadata"{
                let field=u["field"].as_str().unwrap_or("");let value=u["value"].as_str().unwrap_or("");
                match field{
                    "external_url"=>{
                        ensure!(regex::Regex::new(r"(?i)^https?://\S+$")?.is_match(value)&&text(&u["display_text"]),"Retain actual source URL and display text");
                        ensure!(links.iter().any(|l|!l.image&&l.href==value&&norm(&l.label)==norm(u["display_text"].as_str().unwrap())),"Source URL target/label not delivered");
                    },
                    "doi"=>ensure!(regex::Regex::new(r"^10\.\d{4,9}/\S+$")?.is_match(value)&&links.iter().any(|l|!l.image&&percent_encoding::percent_decode_str(&l.href).decode_utf8_lossy().eq_ignore_ascii_case(&format!("https://doi.org/{value}"))),"Source DOI not delivered canonically"),
                    "email"=>ensure!(regex::Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$")?.is_match(value)&&links.iter().any(|l|!l.image&&source_mailto(&l.href,value)),"Literal source email not delivered"),
                    "header_note"=>{
                        if !header_note_layout(candidate,span)?{
                            a.fail("HEADER_NOTE_LAYOUT",id,"Deliver this source header note inside its source-specific visible metadata blockquote.");
                        }
                    },
                    _=>{},
                }
            }
            Ok(())
        })();a.result("DELIVERY_TARGET",id,r);
    }
    for link in &candidate.links{
        if link.image{continue;}
        let href=percent_encoding::percent_decode_str(&link.href).decode_utf8_lossy();
        let Some(anchor)=href.strip_prefix("#^")else{continue;};
        if !bibliography.contains_key(anchor)&&validate::citation_preview_anchor(anchor).is_none(){continue;}
        let owners:Vec<_>=calls.iter().filter(|(_,s)|s.contains(link.span)).collect();
        if owners.len()!=1{a.fail("OUTPUT_CITATION_UNMAPPED",anchor,"Every actual bibliography call, including in decoded table fields, must have one explicit source citation occurrence.");}
    }
    for(i,(_,data))in tables.iter().enumerate(){
        let n=if data["version"]==3{data["sheets"].as_array().context("Workbook sheets")?.len()}else{1};
        for s in 0..n{if !covered.contains(&(i,s)){a.fail("TABLE_WORKSHEET_UNMAPPED",&format!("{i}:{s}"),"Every actual worksheet needs an explicit source table unit.");}}
    }
    for id in candidate.anchors.keys(){if (id.starts_with("fig-")||id.starts_with("tbl-")||id.starts_with("eq-")||
        validate::citation_preview_anchor(id).is_some())&&!refs.contains(id){
        a.fail("OUTPUT_OBJECT_UNMAPPED",id,"Register the corresponding source object or bibliography entry.");
    }}
    Ok(())
}
fn terminology(a:&mut Audit,inv:&Value,ledger:&Value,base:&Path)->Result<()>{
    a.file(&ledger["glossary"],base)?;
    if ledger["review_status"]=="not_applicable"{ensure!(text(&ledger["reason"]),"Explain terminology applicability");return Ok(());}
    ensure!(ledger["review_status"]=="reviewed","Complete terminology review");
    let units=index(&inv["units"],"id")?;
    let wanted:BTreeSet<_>=units.iter().filter(|(_,u)|matches!(u["kind"].as_str(),Some("paragraph"|"section"|"figure"|"table"))||u["kind"]=="metadata"&&matches!(u["field"].as_str(),Some("title"|"header_note"))).map(|(id,_)|id.as_str()).collect();
    ensure!(ids(&ledger["covered_units"])?==wanted,"Terminology unit coverage differs");
    for key in ["adopted","unlisted"]{
        let rows=list(&ledger[key],key)?;
        if key=="unlisted"&&rows.is_empty(){ensure!(text(&ledger["no_unlisted_reason"]),"Explain the reviewed empty unlisted-term set");}
        for r in rows{
            for k in ["original","translation","context",if key=="adopted"{"glossary_locator"}else{"reason"}]{ensure!(text(&r[k]),"Missing terminology {k}");}
            let source=ids(&r["source_units"])?;ensure!(!source.is_empty()&&source.iter().all(|s|units.contains_key(*s)),"Unknown terminology source unit");
            if key=="unlisted"{ensure!(r["status"]=="resolved"&&text(&r["resolution_basis"]),"Unlisted terminology decision unresolved");}
        }
    }Ok(())
}
fn figures(a:&mut Audit,inv:&Value,manifest:&Value,ledger:&Value,ledger_path:&Path,inventory_path:&Path,root:&Path)->Result<()>{
    let base=ledger_path.parent().unwrap();let ib=inventory_path.parent().unwrap();
    ensure!(matches!(ledger["schema_version"].as_u64(),Some(1|2)),"Invalid figure-review version");
    ensure!(files::canonical(&base.join(path(&ledger["inventory_path"])?))?==inventory_path,"Figure ledger belongs to another inventory");
    let snap=a.file(&ledger["inventory"],base)?;let p=path(&snap["path"])?;
    ensure!(p!=inventory_path,"Independent immutable figure inventory snapshot required");
    let (mut snapshot,b)=a.bindings.json(&p)?;ensure!(b==snap,"Figure snapshot changed");
    let mut live=inv.clone();for k in ["checks_path","terminology_path","figure_reviews_path"]{snapshot.as_object_mut().unwrap().remove(k);live.as_object_mut().unwrap().remove(k);}
    ensure!(snapshot==live,"Figure inventory content changed after review");
    let units=index(&inv["units"],"id")?;let sources=index(&inv["sources"],"id")?;let deliveries=index(&manifest["deliveries"],"unit")?;
    let rows=index(&ledger["figures"],"unit")?;
    let wanted:BTreeSet<_>=units.iter().filter(|(_,v)|v["kind"]=="figure").map(|(k,_)|k.as_str()).collect();
    ensure!(rows.keys().map(String::as_str).collect::<BTreeSet<_>>()==wanted,"Figure review unit coverage differs");
    for(id,row)in rows{
        let r=(||->Result<()>{
            let u=units[&id];let sid=u["source"].as_str().context("Figure source")?;let s=*sources.get(sid).context("Unknown figure source")?;
            ensure!(row["source"]==sid,"Figure source identity differs");
            let source=a.file(s,ib)?;ensure!(a.file(&row["source_file"],base)?==source,"Figure source binding differs");
            let locs=list(&row["source_locations"],"figure source locations")?;
            let expected=list(&u["evidence"],"source unit evidence")?;
            ensure!(!locs.is_empty()&&locs.len()==expected.len(),"Figure source location coverage differs");
            let native=s["format"]=="docx";
            let file_key=if native{"source_media"}else{"page_render"};
            let source_kind=if native{"source_location"}else{"source_page"};
            for (loc,original)in locs.iter().zip(expected){
                if native{
                    let actual=docx::figure_location(s,original)?;
                    let mut x=actual.clone();x["source_media"]=a.file(&actual["source_media"],ib)?;
                    let mut y=loc.clone();y["source_media"]=a.file(&loc["source_media"],base)?;
                    ensure!(x==y,"Native media source locator differs");
                }else{
                    for k in ["page","bbox","coordinate_system","image_xref"]{ensure!(loc[k]==original[k],"Figure source locator differs: {k}");}
                    let page=loc["page"].as_u64().context("Figure page")?;
                    let matching:Vec<_>=list(&s["pages"],"pages")?.iter().filter(|p|p["page"].as_u64()==Some(page)).collect();
                    ensure!(matching.len()==1&&a.file(&loc["page_render"],base)?==a.file(&matching[0]["render"],ib)?,"Wrong bound source page");
                }
            }
            let assets=list(&row["final_assets"],"final assets")?;
            let delivery=*deliveries.get(&id).context("Missing figure delivery")?;
            let delivered=list(&delivery["assets"],"delivery assets")?;
            ensure!(!assets.is_empty()&&assets.len()==delivered.len(),"Figure final asset coverage differs");
            let mut seen=HashSet::new();let mut bound=Vec::new();
            for(x,y)in assets.iter().zip(delivered){
                let b=asset_from(a,x,base,root)?;ensure!(b==asset(a,y,root)?&&seen.insert(b["path"].as_str().unwrap().to_owned()),"Figure reviewed asset differs or duplicates");bound.push(b);
            }
            let checks=index(&row["checks"],"id")?;
            ensure!(checks.len()==crate::source_prepare::FIGURE_GOALS.len(),"Retain all atomic figure goals");
            for(i,goal)in crate::source_prepare::FIGURE_GOALS.iter().enumerate(){
                let cid=format!("FIG-A{:02}",i+1);
                let c=*checks.get(&cid).context("Missing atomic figure goal")?;
                let result=(||->Result<()>{
                    ensure!(c["goal"]==*goal,"Atomic goal changed");
                    ensure!(matches!(c["status"].as_str(),Some("pass"|"not_applicable"))&&text(&c["reason"]),"Atomic figure check unresolved");
                    if c["status"]=="not_applicable"{ensure!(text(&c["applicability_condition"]),"Explicit applicability condition required");}
                    let evidence=list(&c["evidence"],"figure goal evidence")?;
                    ensure!(!evidence.is_empty(),"Atomic goal needs located evidence");
                    let mut kinds=BTreeSet::new();
                    for e in evidence{
                        let k=e["kind"].as_str().context("Evidence kind")?;let n=e["index"].as_u64().context("Evidence index")?;
                        ensure!((k==source_kind&&n<locs.len() as u64||k=="final_asset"&&n<assets.len() as u64)&&text(&e["observation"]),"Invalid figure evidence location or observation");
                        kinds.insert(k);
                    }
                    if c["status"]=="pass"&&(i<18||i>=25){ensure!(kinds==BTreeSet::from([source_kind,"final_asset"]),"Source and final asset comparison required");}
                    Ok(())
                })();a.result("FIG_REVIEW_CHECK",&format!("{id}:{cid}"),result);
            }
            if manifest["figure_edge_reviews_required"]==true||ledger["schema_version"]==2||row.get("edge_reviews").is_some(){
                let edges=list(&row["edge_reviews"],"edge reviews")?;
                let mut seen=BTreeSet::new();let mut covered=BTreeSet::new();
                for e in edges{
                    let i=usize::try_from(e["asset_index"].as_u64().context("Asset index")?)?;
                    let side=e["edge"].as_str().context("Asset edge")?;
                    ensure!(i<bound.len()&&matches!(side,"left"|"right"|"top"|"bottom")&&seen.insert((i,side)),"Duplicate or invalid edge identity");
                    let result=(||->Result<()>{
                        ensure!(e["status"]=="pass"&&text(&e["observation"]),"Edge comparison unresolved");
                        ensure!(a.file(&e["source_file"],base)?==source&&asset_from(a,&e["final_asset"],base,root)?==bound[i],"Edge file binding differs");
                        let locations=list(&e["source_locations"],"edge source locations")?;ensure!(!locations.is_empty(),"Edge needs actual source location");
                        let mut local=BTreeSet::new();
                        for v in locations{
                            let n=usize::try_from(v["index"].as_u64().context("Source location index")?)?;
                            ensure!(n<locs.len()&&local.insert(n),"Invalid or repeated edge source location");
                            let mut actual=v.clone();actual.as_object_mut().unwrap().remove("index");
                            actual[file_key]=a.file(&v[file_key],base)?;
                            let mut expected=locs[n].clone();expected[file_key]=a.file(&locs[n][file_key],base)?;
                            ensure!(actual==expected,"Edge source locator differs");
                        }
                        covered.extend(local);Ok(())
                    })();a.result("FIG_REVIEW_EDGE",&format!("{id}:asset-{i}:{side}"),result);
                }
                ensure!(seen.len()==bound.len()*4&&covered.len()==locs.len(),"Every asset edge and source location requires completed evidence");
            }Ok(())
        })();a.result("FIG_REVIEW_RECORD",&id,r);
    }Ok(())
}
fn checks(a:&mut Audit,inv:&Value,ledger:&Value,p:&Path,ib:&Path,candidate:&Value,progress:&mut impl FnMut(usize,usize,&str))->Result<()>{
    let r=check_validate::validate(p,&mut *progress)?;a.import(&r,"");
    for (key,code) in [("offline_unresolved_ids","CHECK_OFFLINE_INCOMPLETE"),("unclassified_ids","CHECK_BOUNDARY_UNCLASSIFIED")]{
        for id in r[key].as_array().context("Acceptance report schema")?{a.fail(code,id.as_str().unwrap_or("check"),"Resolve the original atomic check without changing its scope.");}
    }
    let base=p.parent().unwrap();let mut expected=BTreeMap::new();
    for(id,s)in index(&inv["sources"],"id")?{expected.insert(id,a.file(s,ib)?);}
    let mut actual=BTreeMap::new();
    for(id,s)in index(&ledger["bindings"]["sources"],"id")?{actual.insert(id,a.file(s,base)?);}
    ensure!(expected==actual,"Acceptance source bindings differ");
    let mut found=false;
    for s in list(&ledger["bindings"]["candidates"],"candidate bindings")?{found|=a.file(s,base)?==*candidate;}
    ensure!(found,"Acceptance ledger does not bind the current candidate");
    Ok(())
}
pub fn validate_delivery(manifest_path:&Path,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let mut a=Audit{findings:vec![],bindings:Bindings::new(),svg_assets:BTreeMap::new(),svg_bytes:0};
    let mut stages=Map::new();
    let result=(||->Result<()>{
        let (manifest,m)=a.bindings.json(manifest_path)?;
        let mp=path(&m["path"])?;let mb=mp.parent().context("Manifest directory")?;
        let ib=a.file(&manifest["inventory"],mb)?;let cb=a.file(&manifest["candidate"],mb)?;
        let ip=path(&ib["path"])?;let cp=path(&cb["path"])?;
        ensure!(ip!=cp&&mp!=ip&&mp!=cp,"Manifest aliases an input");
        let (inv,actual)=a.bindings.json(&ip)?;ensure!(ib==actual,"Inventory changed");
        let base=ip.parent().unwrap();
        let root=if manifest.get("resource_root").is_some(){files::canonical(&mb.join(path(&manifest["resource_root"])?))?}else{cp.parent().unwrap().to_owned()};
        ensure!(root.is_dir(),"Resource root must be a directory");
        let (raw,current)=files::read_bound(&cp)?;ensure!(current==cb,"Candidate changed");
        let candidate=Candidate::new(&raw)?;
        for f in validate::validate_note(&candidate.raw){if f.level=="error"{a.findings.push(json!({"code":format!("NOTE_{}",f.code),"object_id":"candidate","message":f.message,"level":"error","line":f.line}));}}
        let r=delivery_provenance::validate(&ip,&mut progress)?;
        ensure!(r["inventory"]==ib,"Source inventory changed between validation stages");
        for b in r["checked_files"].as_array().context("Source evidence bindings")?{a.bindings.recorded(b,base)?;}
        stages.insert("source_provenance".into(),r.clone());a.import(&r,"");
        let r=inventory_checks(&mut a,&inv,base);a.result("INVENTORY_ACCOUNTING","inventory",r);
        let r=targets(&mut a,&inv,&manifest,&candidate,&root);a.result("DELIVERY_TARGETS","manifest",r);
        // Re-derive link/source relationships from actual input and selectors,
        // with the same resource root as the delivery manifest.
        let derived=(||->Result<_>{
            let mut ds=manifest["deliveries"].clone();
            if let Some(rows)=ds.as_array_mut(){for row in rows{if row.is_object(){
                if let Some(bound)=row.get("resource_root"){
                    ensure!(files::canonical(&mb.join(path(bound)?))?==root,"DELIVERY_RESOURCE_ROOT: per-unit root differs from manifest");
                }
                row["resource_root"]=json!(crate::path_text(&root)?);
            }}}
            handoff_native::derive_for_delivery(&ip,&cp,&ds,&mut progress)
        })();
        match derived{Ok((docs,b))=>{a.bindings.merge(&b)?;stages.insert("handoff_derivation".into(),json!({"map_names":docs.as_object().unwrap().keys().collect::<Vec<_>>(),"records_consistent":true}));},Err(e)=>a.fail("DELIVERY_HANDOFF","manifest",format!("{e:#}"))}
        let r=(||->Result<()>{
            if let Some((p,l))=a.ledger(&inv,&manifest,base,mb,"terminology","terminology_path")?{terminology(&mut a,&inv,&l,p.parent().unwrap())?;}Ok(())
        })();a.result("TERMINOLOGY","terminology",r);
        let r=(||->Result<()>{
            if let Some((p,l))=a.ledger(&inv,&manifest,base,mb,"figure_reviews","figure_reviews_path")?{figures(&mut a,&inv,&manifest,&l,&p,&ip,&root)?;}
            else if manifest["figure_edge_reviews_required"]==true&&list(&inv["units"],"units")?.iter().any(|u|u["kind"]=="figure"){anyhow::bail!("Final figure ledger and four-edge records required");}Ok(())
        })();a.result("FIG_REVIEW_REQUIRED","figure_reviews",r);
        if let Some(v)=manifest.get("translation_review"){
            let r=(||->Result<()>{
                let b=a.file(v,mb)?;let p=path(&b["path"])?;
                let report=translation_validate::validate(&p,Some(&ip),Some(&cp))?;a.import(&report,"");
                ensure!(report["records_ready"]==true,"Translation review remains unresolved");Ok(())
            })();a.result("TRANSLATION_REVIEW","translation_review",r);
        }
        let r=(||->Result<()>{
            if let Some((p,l))=a.ledger(&inv,&manifest,base,mb,"checks","checks_path")?{checks(&mut a,&inv,&l,&p,base,&cb,&mut progress)?;}Ok(())
        })();a.result("CHECK_LEDGER","checks",r);
        a.bindings.verify()?;Ok(())
    })();
    a.result("MANIFEST_OR_EVIDENCE_INVALID","manifest",result);
    // Keep the original full gate fail-closed until the remaining historical
    // delivery behaviors are migrated and independently regression tested.
    let subset_consistent=a.findings.is_empty();
    let checked_files=a.bindings.all();
    a.fail("NATIVE_DELIVERY_PARITY_PENDING","manifest","Native source, target and review subsets ran; complete historical delivery equivalence is not yet qualified.");
    Ok(json!({"findings":a.findings,"structurally_consistent":false,"native_subset_consistent":subset_consistent,
        "stage_reports":stages,"checked_files":checked_files,"source_fidelity":"not_assessed","independent_review_required":true,"evidence_boundary":BOUNDARY}))
}
