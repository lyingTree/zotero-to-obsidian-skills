//! Prepare real PDF and DOCX sources using the common inventory contract.
//! PdfCraft native line records and OCR hypotheses remain explicitly pending.
use crate::{docx,files,path,path_text};
use anyhow::{anyhow,ensure,Context,Result};
use serde_json::{json,Value,Map};
use std::{fs,io::Write,path::{Path,PathBuf}};

fn binding(p:&Path)->Result<Value>{
    let b=files::snapshot(p)?;Ok(json!({"path":b["path"],"sha256":b["sha256"]}))
}
fn no_existing(p:&Path)->Result<()>{
    match fs::symlink_metadata(p){
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(()),
        Err(e)=>Err(e.into()),Ok(_)=>Err(anyhow!("Keep the existing evidence record: {}",p.display())),
    }
}
fn exclusive(p:&Path,value:&Value)->Result<()>{
    no_existing(p)?;
    let mut bytes=serde_json::to_vec_pretty(value)?;bytes.push(b'\n');
    let mut f=tempfile::Builder::new().prefix(".pending-source-").tempfile_in(p.parent().context("source parent")?)?;
    f.write_all(&bytes)?;f.as_file().sync_all()?;
    f.persist_noclobber(p).map_err(|e|anyhow!("Cannot exclusively save {}: {}",p.display(),e.error))?;
    ensure!(crate::pipeline::read_json(p)?==*value,"Source inventory did not round-trip");Ok(())
}
fn safe_dir(dir:&Path)->Result<PathBuf>{
    if let Ok(m)=fs::symlink_metadata(dir){ensure!(!m.file_type().is_symlink()&&m.is_dir(),"Refusing a symlink or non-directory source evidence root");}
    fs::create_dir_all(dir)?;files::canonical(dir)
}
fn id_ok(id:&str)->bool{id.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)&&id.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-'))}
pub fn prepare_docx_sources(sources:&Value,root:&Path,roles:&Value,glossary:Option<&Path>,dpi:u64,progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    prepare_impl(sources,root,roles,glossary,dpi,None,true,progress)
}
pub fn prepare_sources(sources:&Value,root:&Path,roles:&Value,glossary:Option<&Path>,dpi:u64,ocr:Option<&crate::pdf_inventory::OcrSettings>,progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    prepare_impl(sources,root,roles,glossary,dpi,ocr,false,progress)
}
#[allow(clippy::too_many_arguments)]
fn prepare_impl(sources:&Value,root:&Path,roles:&Value,glossary:Option<&Path>,dpi:u64,ocr:Option<&crate::pdf_inventory::OcrSettings>,docx_only:bool,mut progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    let map=sources.as_object().filter(|m|!m.is_empty()).context("source_pdfs must be a nonempty JSON object mapping source_id to file path, e.g. {\"main\": \"path/to/paper.pdf\"}")?;
    ensure!((72..=600).contains(&dpi),"dpi must be between 72 and 600");
    ensure!(roles.is_null()||roles.is_object(),"source_roles must be an object");
    // Cheap no-work rejection for a completed prior run. Repeat under the
    // writer lock after validating all selected inputs.
    no_existing(&root.join("source-inventory.json"))?;
    if glossary.is_some(){no_existing(&root.join("terminology.json"))?;}
    #[derive(Clone, Copy)]
    enum Format { Docx, Pdf, Html, Epub, Odt, Text }
    let mut portable_ids=std::collections::BTreeSet::new();
    let mut actual_sources=std::collections::BTreeSet::new();
    if let Some(roles)=roles.as_object(){ensure!(roles.keys().all(|id|map.contains_key(id)),"Source role names an absent source");}
    let mut inputs=Vec::with_capacity(map.len());
    for (id,v) in map{
        ensure!(id_ok(id)&&portable_ids.insert(id.to_ascii_lowercase()),"Invalid or case-conflicting source ID: {id}");
        let p=files::canonical(&path(v)?)?;
        ensure!(actual_sources.insert(p.clone()),"Duplicate actual source attachment: {id}");
        let extension=p.extension().and_then(|s|s.to_str()).unwrap_or("").to_ascii_lowercase();
        let format=match extension.as_str(){
            "docx"=>Format::Docx,
            "pdf" if docx_only=>anyhow::bail!("UNMIGRATED_PDF_INVENTORY: {id}; this entrypoint is DOCX-only"),
            "pdf"=>Format::Pdf,
            "html"|"htm"|"xhtml" if !docx_only=>Format::Html,
            "epub" if !docx_only=>Format::Epub,
            "odt" if !docx_only=>Format::Odt,
            "txt"|"md"|"markdown" if !docx_only=>Format::Text,
            _=>anyhow::bail!("Unsupported source format: {id}"),
        };
        let role=roles.get(id).cloned().unwrap_or(json!("pending"));
        ensure!(matches!(role.as_str(),Some("main"|"supplement"|"pending")),"Source role must be main, supplement or pending");
        let record=binding(&p)?;
        inputs.push((id,p,role,format,record));
    }
    if let Some(o)=ocr{o.binding()?;}
    let glossary=glossary.map(binding).transpose()?;

    // Read-only preflight for the entire batch. A malformed later DOCX or
    // oversized later PDF page must not strand an earlier source's evidence.
    // Process one attachment at a time instead of retaining all parsed PDFs
    // and XML/media indexes while waiting to write.
    for (id,p,_,format,record) in &inputs{
        let sha=record["sha256"].as_str().context("Source digest required")?;
        match format{
            Format::Docx=>{
                docx::read_bound_docx(p,id,Some(sha),|_,_,_|{})
                    .with_context(||format!("{id}: accessible DOCX required"))?;
            }
            Format::Pdf=>{
                let pdf=crate::pdfcraft_backend::Backend::open(p,Some(sha))
                    .with_context(||format!("{id}: accessible PDF required"))?;
                pdf.preflight_whole_pages(dpi)
                    .with_context(||format!("{id}: PDF page raster preflight failed"))?;
            }
            Format::Html=>{
                crate::text_formats::read_bound_document(p,id,crate::text_formats::FormatKind::Html,Some(sha))
                    .with_context(||format!("{id}: accessible HTML required"))?;
            }
            Format::Epub=>{
                crate::text_formats::read_bound_document(p,id,crate::text_formats::FormatKind::Epub,Some(sha))
                    .with_context(||format!("{id}: accessible EPUB required"))?;
            }
            Format::Odt=>{
                crate::text_formats::read_bound_document(p,id,crate::text_formats::FormatKind::Odt,Some(sha))
                    .with_context(||format!("{id}: accessible ODT required"))?;
            }
            Format::Text=>{
                crate::text_formats::read_bound_document(p,id,crate::text_formats::FormatKind::Text,Some(sha))
                    .with_context(||format!("{id}: accessible text/Markdown source required"))?;
            }
        }
    }
    let root=safe_dir(root)?;
    let output=root.join("source-inventory.json");let terminology=root.join("terminology.json");
    let _lock=crate::check_write::lock_target(&output)?;
    no_existing(&output)?;if glossary.is_some(){no_existing(&terminology)?;}
    let evidence=safe_dir(&root.join("source-evidence"))?;
    ensure!(evidence.starts_with(&root),"source evidence directory escapes work root");
    // Preflight all destinations, including pre-existing partial preparations.
    for (id,_,_,_,_) in &inputs{no_existing(&evidence.join(id))?;}
    let verify_inputs=|| -> Result<()> {
        for (id,p,_,_,record) in &inputs{
            ensure!(binding(p)?==*record,"{id}: source changed before inventory publication");
        }
        if let Some(o)=ocr{o.binding()?;}
        Ok(())
    };
    let mut rows=Vec::with_capacity(inputs.len());
    for (i,(id,p,role,format,record)) in inputs.iter().enumerate(){
        let destination=evidence.join(id);
        let sha=record["sha256"].as_str().context("Source digest required")?;
        // The backend compares the actual input against this snapshot again
        // before writing bytes, not just after completion.
        let mut native=match format{
            Format::Docx=>docx::prepare_bound_docx_source(p,id,&destination,Some(sha),&mut progress),
            Format::Pdf=>crate::pdf_inventory::prepare_bound_source(p,id,&destination,dpi,ocr,Some(sha),&mut progress),
            Format::Html=>crate::text_formats::prepare_bound_document(p,id,&destination,crate::text_formats::FormatKind::Html,Some(sha)),
            Format::Epub=>crate::text_formats::prepare_bound_document(p,id,&destination,crate::text_formats::FormatKind::Epub,Some(sha)),
            Format::Odt=>crate::text_formats::prepare_bound_document(p,id,&destination,crate::text_formats::FormatKind::Odt,Some(sha)),
            Format::Text=>crate::text_formats::prepare_bound_document(p,id,&destination,crate::text_formats::FormatKind::Text,Some(sha)),
        }.with_context(||format!("{id}: source evidence preparation failed"))?;
        ensure!(binding(p)?==*record,"{id}: source changed during evidence preparation");
        native["id"]=json!(id);native["role"]=role.clone();
        native["path"]=record["path"].clone();native["sha256"]=record["sha256"].clone();
        let headers:Map<String,Value>=["title","authors","affiliations","publication_date"].iter().map(|key|((*key).into(),json!({"status":"pending"}))).collect();
        native["header_fields"]=json!(headers);rows.push(native);
        progress(i+1,inputs.len(),if docx_only{"DOCX sources"}else{"PDF/DOCX sources"});
    }
    let mut inventory=json!({"schema_version":1,"evidence_boundary":"Rendered files do not prove that a model viewed or understood them.",
        "sources":rows,"units":[],"excluded_blocks":[],"excluded_pages":[]});
    verify_inputs()?;
    if let Some(g)=glossary{
        ensure!(binding(Path::new(g["path"].as_str().unwrap()))?==g,"glossary changed during inventory preparation");
        let value=json!({"schema_version":1,"glossary":g,"review_status":"pending","covered_units":[],"adopted":[],"unlisted":[],
            "no_unlisted_reason":null,"evidence_boundary":"A ledger records decisions; it does not prove translation correctness."});
        exclusive(&terminology,&value)?;inventory["terminology_path"]=json!(path_text(&terminology)?);
        verify_inputs()?;
    }
    exclusive(&output,&inventory)?;
    Ok(output)
}


// Edge observations are explicit caller decisions, not inferred pixel checks.
const FIGURE_EDGES: &[&str] = &["left", "right", "top", "bottom"];
const EDGE_STATUSES: &[&str] = &["pending", "pass", "fail", "blocked", "not_tested", "ambiguous"];

fn figure_reference(record: &Value, base: &Path, label: &str) -> Result<Value> {
    ensure!(record.is_object(), "{label}: a file record is required");
    let raw = record["path"].as_str().filter(|s| !s.trim().is_empty())
        .with_context(|| format!("{label}: a file path is required"))?;
    let sha = record["sha256"].as_str().filter(|s| s.len() == 64 &&
        s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        .with_context(|| format!("{label}: a lowercase SHA-256 is required"))?;
    let p = base.join(raw);
    let actual = binding(&p)?;
    ensure!(actual["sha256"] == sha, "{label}: the file changed after its inventory record");
    Ok(actual)
}

pub fn record_figure_edges(
    review_path: &Path, unit_id: &str, asset_index: usize,
    source_location_indices: &Value, observations: &Value,
) -> Result<PathBuf> {
    use std::collections::HashSet;
    crate::check_write::no_link(review_path)?;
    let review_path = files::canonical(review_path)?;
    let _lock = crate::check_write::lock_target(&review_path)?;
    let (raw, previous) = files::read_bound(&review_path)?;
    let mut ledger = crate::parse_json(std::str::from_utf8(&raw)?)?;
    ensure!(ledger.is_object() && ledger["schema_version"].as_u64() == Some(2),
        "Explicit edge recording requires figure-review schema_version 2");
    let base = review_path.parent().context("figure review parent")?;
    let figures = ledger["figures"].as_array_mut().context("figures must be an array of objects")?;
    let row_index = {
        let mut ids = HashSet::new();
        let mut found = None;
        for (i, row) in figures.iter().enumerate() {
            ensure!(row.is_object(), "figures must contain objects");
            let id = row["unit"].as_str().filter(|s| !s.trim().is_empty())
                .context("figures requires nonempty unit values")?;
            ensure!(ids.insert(id), "Duplicate figures unit: {id}");
            if id == unit_id { found = Some(i); }
        }
        found.context("Choose an actual figure unit from this ledger")?
    };
    let row = &mut figures[row_index];
    let assets = row["final_assets"].as_array().context("This figure needs final assets")?;
    let asset_count = assets.len();
    ensure!(asset_index < asset_count, "asset_index must identify this figure's final image");
    let locations = row["source_locations"].as_array()
        .context("This figure needs complete source-page location records")?;
    ensure!(locations.iter().all(Value::is_object), "This figure needs complete source-page location records");
    let indices = source_location_indices.as_array().filter(|a| !a.is_empty())
        .context("Explicitly choose the source-page locations used for this asset")?;
    let obs = observations.as_object().filter(|o| !o.is_empty())
        .context("Supply explicit observations keyed by left, right, top, or bottom")?;
    ensure!(obs.keys().all(|k| FIGURE_EDGES.contains(&k.as_str())),
        "Supply explicit observations keyed by left, right, top, or bottom");
    for supplied in obs.values() {
        let fields = supplied.as_object().context("Each edge needs explicit status and observation fields")?;
        ensure!(fields.len() == 2 && fields.contains_key("status") && fields.contains_key("observation"),
            "Each edge needs explicit status and observation fields");
        ensure!(supplied["status"].as_str().is_some_and(|s| EDGE_STATUSES.contains(&s)),
            "An edge status must be explicit; geometrical edges are not inapplicable");
        ensure!(supplied["observation"].as_str().is_some_and(|s| !s.trim().is_empty()),
            "Record the actual comparison observation for each supplied edge");
    }

    let source = figure_reference(&row["source_file"], base, &format!("{unit_id}:source"))?;
    let asset = figure_reference(&assets[asset_index], base, &format!("{unit_id}:final-asset"))?;
    let mut seen = HashSet::new();
    let mut selected = Vec::with_capacity(indices.len());
    let mut evidence = vec![source.clone(), asset.clone()];
    for index in indices {
        let i = index.as_u64().and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < locations.len())
            .context("Source-page indices must identify source_locations")?;
        ensure!(seen.insert(i), "Source-page indices must be distinct");
        let mut location = locations[i].as_object().unwrap().clone();
        let file_key = if location.contains_key("native_locator") { "source_media" } else { "page_render" };
        let b = figure_reference(&locations[i][file_key], base, &format!("{unit_id}:source-location-{i}"))?;
        location.insert(file_key.into(), b.clone());
        evidence.push(b);
        // Preserve source location fields exactly, including the reference merge order.
        let mut selected_location = Map::new();
        selected_location.insert("index".into(), json!(i));
        selected_location.extend(location);
        selected.push(Value::Object(selected_location));
    }

    let entries = row["edge_reviews"].as_array_mut().context("Retain every generated edge-review slot")?;
    let slots_count = asset_count.checked_mul(FIGURE_EDGES.len()).context("Too many edge slots")?;
    ensure!(entries.len() == slots_count, "Retain exactly four edge-review slots for every final asset");
    let mut slots = vec![None; slots_count];
    for (i, entry) in entries.iter().enumerate() {
        ensure!(entry.is_object(), "Retain every generated edge-review slot");
        let index = entry["asset_index"].as_u64().and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < asset_count).context("Each edge slot needs its actual asset index")?;
        let side = entry["edge"].as_str().and_then(|s| FIGURE_EDGES.iter().position(|e| *e == s))
            .context("Each edge slot needs one of the four sides")?;
        ensure!(slots[index * FIGURE_EDGES.len() + side].replace(i).is_none(),
            "Do not duplicate an asset's edge-review slot");
    }
    ensure!(slots.iter().all(Option::is_some), "Retain exactly four edge-review slots for every final asset");
    let selected = Value::Array(selected);
    for (side, supplied) in obs {
        let side_index = FIGURE_EDGES.iter().position(|s| *s == side.as_str()).unwrap();
        let entry = &mut entries[slots[asset_index * FIGURE_EDGES.len() + side_index].unwrap()];
        entry["status"] = supplied["status"].clone();
        entry["observation"] = supplied["observation"].clone();
        entry["source_file"] = source.clone();
        entry["final_asset"] = asset.clone();
        entry["source_locations"] = selected.clone();
    }

    // A second independent byte binding prevents changed evidence from being
    // published as a new observation; no visual outcome is certified here.
    for record in &evidence {
        ensure!(figure_reference(record, base, "figure evidence before publication")? == *record,
            "Figure evidence changed before publication");
    }
    let mut bytes = serde_json::to_vec_pretty(&ledger)?;
    bytes.push(b'\n');
    crate::check_write::replace_bytes(&review_path, previous["sha256"].as_str().context("ledger SHA-256")?, &bytes)?;
    Ok(review_path)
}


pub(crate) const FIGURE_GOALS: &[&str] = &[
    "子图面板齐全。","面板号完整。","图内文字完整。","轴线完整。","刻度完整。","单位完整。",
    "图例完整。","色标完整。","图内要素可读。","图外题注没有混入图件。","相邻图件没有混入。",
    "正文没有混入图件。","页眉没有混入图件。","页脚没有混入图件。","页码没有混入图件。",
    "图件没有大块页边空白。","图号与图件对应。","题注与图件对应。","来源页码已登记。",
    "来源附件哈希已登记。","图框或原图对象定位已登记。","适用的坐标系已登记。",
    "最终图片哈希已登记。","视觉证据位置已登记。","最终嵌入图片与已验资源哈希一致。",
    "输出图像的透明度效果与源页一致。","输出图像的颜色与源页一致。","图内数据表在原图中完整保留。",
];

fn figure_index<'a>(rows:&'a Value,key:&str,label:&str)
    ->Result<std::collections::HashMap<&'a str,&'a Value>>{
    let mut index=std::collections::HashMap::new();
    for row in rows.as_array().with_context(||format!("{label} must be an array of objects"))?{
        ensure!(row.is_object(),"{label} must contain objects");
        let id=row[key].as_str().filter(|s|!s.trim().is_empty())
            .with_context(||format!("{label} requires nonempty {key} values"))?;
        ensure!(index.insert(id,row).is_none(),"Duplicate {label} {key}: {id}");
    }
    Ok(index)
}

fn json_bytes(v:&Value)->Result<Vec<u8>>{
    let mut b=serde_json::to_vec_pretty(v)?;b.push(b'\n');Ok(b)
}

/// Initialize the pending schema-2 review from real PDF/DOCX locations and bound
/// image files. PDF page renders must already exist; this does not render pages
/// or certify pixel content, visual completeness, or any review observation.
pub fn prepare_figure_reviews(inventory_path:&Path,final_assets:&Value,work_directory:&Path,
    resource_root:Option<&Path>,mut progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    use std::collections::{HashMap,HashSet};
    use crate::{pdf_structure::Pdf,check_write,pipeline};
    check_write::no_link(inventory_path)?;
    let inventory_path=files::canonical(inventory_path)?;
    let _lock=check_write::lock_target(&inventory_path)?;
    let (old,bound)=files::read_bound(&inventory_path)?;
    let mut inventory=crate::parse_json(std::str::from_utf8(&old)?)?;
    ensure!(inventory.is_object()&&inventory["schema_version"].as_u64()==Some(1),
        "Expected source inventory schema_version 1");
    ensure!(inventory.get("figure_reviews_path").is_none(),"This inventory already names a figure-review ledger; preserve that review history");
    let base=inventory_path.parent().context("inventory parent")?;
    let root=files::canonical(resource_root.unwrap_or(base))?;
    ensure!(root.is_dir(),"resource root must be a directory");
    let _units=figure_index(&inventory["units"],"id","units")?;
    let sources=figure_index(&inventory["sources"],"id","sources")?;
    let figures:Vec<_>=inventory["units"].as_array().unwrap().iter().filter(|u|u["kind"]=="figure").collect();
    let assets=final_assets.as_object().context("final_assets must map figure unit IDs to image path lists")?;
    ensure!(assets.len()==figures.len()&&figures.iter().all(|u|assets.contains_key(u["id"].as_str().unwrap())),
        "final_assets must cover exactly the inventory's figure unit IDs");
    let mut pdfs:HashMap<String,Pdf>=HashMap::new();
    let mut source_files:HashMap<String,Value>=HashMap::new();
    let mut evidence:HashMap<String,Value>=HashMap::new();
    fn remember(evidence:&mut HashMap<String,Value>,b:&Value)->Result<()>{
        let key=b["path"].as_str().context("evidence path")?.to_owned();
        if let Some(previous)=evidence.get(&key){ensure!(previous==b,"Evidence changed during preparation");}
        else{evidence.insert(key,b.clone());}
        Ok(())
    }
    let mut rows=Vec::with_capacity(figures.len());
    for (i,unit) in figures.iter().enumerate(){
        let id=unit["id"].as_str().unwrap();
        let sid=unit["source"].as_str().context("choose a source ID from the inventory")?;
        let source=sources.get(sid).with_context(||format!("{id}: choose a source ID from the inventory"))?;
        if !source_files.contains_key(sid){
            let b=figure_reference(source,base,sid)?;
            remember(&mut evidence,&b)?;source_files.insert(sid.to_owned(),b);
        }
        let source_file=&source_files[sid];
        let locs=unit["evidence"].as_array().filter(|v|!v.is_empty()&&v.iter().all(Value::is_object))
            .with_context(||format!("{id}: source locations are required"))?;
        let mut locations=Vec::with_capacity(locs.len());
        if source["format"]=="docx"{
            for location in locs{
                let mut locator=docx::figure_location(source,location)?;
                let b=figure_reference(&locator["source_media"],base,&format!("{id}:native-media"))?;
                remember(&mut evidence,&b)?;locator["source_media"]=b;locations.push(locator);
            }
        }else{
            if !pdfs.contains_key(sid){
                let pdf=Pdf::open(&path(&source_file["path"])?)?;
                pdfs.insert(sid.to_owned(),pdf);
            }
            let pdf=&pdfs[sid];
            let pages=source["pages"].as_array().with_context(||format!("{sid}: pages must be an array of objects"))?;
            ensure!(pages.len()==pdf.len()&&pages.iter().enumerate().all(|(j,v)|v.is_object()&&v["page"].as_u64()==Some((j+1) as u64)),
                "{sid}: record every actual PDF page once in source order");
            for location in locs{
                ensure!(location["basis"]=="visual","{id}: figure locations require visible-page evidence");
                let mut locator=pdf.figure_review_locator(location,source["inventory_backend"]==crate::pdf_inventory::BACKEND)
                    .with_context(||format!("{id}: invalid figure source location"))?;
                let page=locator["page"].as_u64().unwrap();
                let b=figure_reference(&pages[page as usize-1]["render"],base,&format!("{id}:page-{page}"))?;
                remember(&mut evidence,&b)?;locator["page_render"]=b;locations.push(locator);
            }
        }
        let values=assets[id].as_array().filter(|a|!a.is_empty())
            .with_context(||format!("{id}: provide a nonempty sequence of final image paths"))?;
        let mut final_files=Vec::with_capacity(values.len());
        let mut seen=HashSet::new();
        for raw in values{
            let p=path(raw)?;let p=if p.is_absolute(){p}else{root.join(p)};
            let b=binding(&p)?;
            ensure!(seen.insert(b["path"].as_str().unwrap().to_owned()),"{id}: do not repeat final image paths");
            remember(&mut evidence,&b)?;final_files.push(b);
        }
        let mut edge_reviews=Vec::new();
        for (index,b) in final_files.iter().enumerate(){
            for side in FIGURE_EDGES{
                edge_reviews.push(json!({"asset_index":index,"edge":side,"status":"pending","observation":"",
                    "source_file":source_file,"final_asset":b,"source_locations":[]}));
            }
        }
        let checks:Vec<_>=FIGURE_GOALS.iter().enumerate().map(|(j,g)|json!({
            "id":format!("FIG-A{:02}",j+1),"goal":g,"status":"pending","reason":"","evidence":[]
        })).collect();
        rows.push(json!({"unit":id,"source":sid,"source_file":source_file,
            "source_locations":locations,"final_assets":final_files,"edge_reviews":edge_reviews,"checks":checks}));
        progress(i+1,figures.len(),&format!("prepare pending figure review {id}"));
    }
    // Check the immutable evidence again immediately before publishing.
    for b in evidence.values(){
        ensure!(binding(&path(&b["path"])?)?==*b,"Figure evidence changed during preparation");
    }
    ensure!(files::snapshot(&inventory_path)?["sha256"]==bound["sha256"],"Inventory changed during preparation");
    // Freeze the exact input bytes, including every numeric coordinate and its original encoding.
    let snapshot_bytes=old.clone();
    let root=safe_dir(work_directory)?;
    let review=root.join("figure-reviews.json");
    let snapshot=root.join("figure-inventory-snapshot.json");
    ensure!(review!=inventory_path&&snapshot!=inventory_path,"Review output must not replace its inventory");
    no_existing(&review)?;no_existing(&snapshot)?;
    let snapshot_binding=json!({"path":path_text(&snapshot)?,"sha256":files::digest(&snapshot_bytes)});
    let ledger=json!({"schema_version":2,"inventory":snapshot_binding,"inventory_path":path_text(&inventory_path)?,
        "evidence_boundary":"These are reviewer-supplied records. Their existence or self-reported passes do not prove independent visual inspection.",
        "figures":rows});
    inventory["figure_reviews_path"]=json!(path_text(&review)?);
    let after=json_bytes(&inventory)?;
    // Independent new files first, mutable pointer last. A durable recovery
    // receipt preserves the old bytes even if a process is interrupted.
    let recovery=tempfile::Builder::new().prefix(".z2o-figure-transaction-").tempdir_in(&root)?.keep();
    pipeline::publish(&recovery.join("inventory.before"),&old)?;
    pipeline::publish(&recovery.join("inventory.after"),&after)?;
    pipeline::publish(&recovery.join("plan.json"),&json_bytes(&json!({
        "inventory":bound,"after_sha256":files::digest(&after),"review":path_text(&review)?,
        "snapshot":snapshot_binding,"boundary":"Inspect current file hashes before any manual recovery."
    }))?)?;
    let mut frozen=tempfile::Builder::new().prefix(".pending-figure-snapshot-").tempfile_in(&root)?;
    frozen.write_all(&snapshot_bytes)?;frozen.as_file().sync_all()?;
    frozen.persist_noclobber(&snapshot).map_err(|e|anyhow!("Cannot exclusively save {}: {}",snapshot.display(),e.error))?;
    ensure!(binding(&snapshot)?==snapshot_binding,"Snapshot bytes changed before publication");
    exclusive(&review,&ledger)?;
    check_write::replace_bytes(&inventory_path,bound["sha256"].as_str().unwrap(),&after)
        .with_context(||format!("Inventory pointer update failed; retained review and recovery files at {}",recovery.display()))?;
    pipeline::publish(&recovery.join("completed.json"),b"{\"status\":\"completed\"}\n")?;
    Ok(review)
}
