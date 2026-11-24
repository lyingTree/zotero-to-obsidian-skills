//! Prepare one or more real DOCX sources using the common inventory contract.
//! PDF requests must remain blocked until full page/image provenance is native.
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
pub fn prepare_docx_sources(sources:&Value,root:&Path,roles:&Value,glossary:Option<&Path>,dpi:u64,mut progress:impl FnMut(usize,usize,&str))->Result<PathBuf>{
    let map=sources.as_object().filter(|m|!m.is_empty()).context("Provide at least one source PDF or DOCX")?;
    ensure!((72..=600).contains(&dpi),"dpi must be between 72 and 600");
    ensure!(roles.is_null()||roles.is_object(),"source_roles must be an object");
    let mut ids=Vec::with_capacity(map.len());
    let mut bound=Vec::with_capacity(map.len());
    // Reject non-native inputs before creating any evidence output.
    for (id,v) in map{
        ensure!(id_ok(id),"Invalid source ID: {id}");
        let p=files::canonical(&path(v)?)?;
        ensure!(p.extension().and_then(|s|s.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("docx")),
            "UNMIGRATED_PDF_INVENTORY: {id}; full PDF provenance is required and there is no Python fallback");
        let role=roles.get(id).cloned().unwrap_or(json!("pending"));
        ensure!(role.is_string(),"source role must be a string");
        ids.push((id,p.clone(),role));bound.push(binding(&p)?);
    }
    let glossary=match glossary{Some(p)=>Some(binding(p)?),None=>None};
    let root=safe_dir(root)?;
    let output=root.join("source-inventory.json");let terminology=root.join("terminology.json");
    no_existing(&output)?;if glossary.is_some(){no_existing(&terminology)?;}
    let evidence=safe_dir(&root.join("source-evidence"))?;
    ensure!(evidence.starts_with(&root),"source evidence directory escapes work root");
    // Preflight all destinations, including pre-existing partial preparations.
    for (id,_,_) in &ids{no_existing(&evidence.join(id))?;}
    let mut rows=Vec::with_capacity(ids.len());
    for (i,(id,p,role)) in ids.iter().enumerate(){
        let destination=evidence.join(id);
        fs::create_dir(&destination)?;
        let mut native=docx::prepare_docx_source(p,id,&destination,|n,total,label|progress(n,total,label))?;
        ensure!(binding(p)?==bound[i],"{id}: source changed during evidence preparation");
        native["id"]=json!(id);native["role"]=role.clone();
        native["path"]=bound[i]["path"].clone();native["sha256"]=bound[i]["sha256"].clone();
        let headers:Map<String,Value>=["title","authors","affiliations","publication_date"].iter().map(|key|((*key).into(),json!({"status":"pending"}))).collect();
        native["header_fields"]=json!(headers);rows.push(native);
        progress(i+1,ids.len(),"DOCX sources");
    }
    let mut inventory=json!({"schema_version":1,"evidence_boundary":"Rendered files do not prove that a model viewed or understood them.",
        "sources":rows,"units":[],"excluded_blocks":[],"excluded_pages":[]});
    if let Some(g)=glossary{
        ensure!(binding(Path::new(g["path"].as_str().unwrap()))?==g,"glossary changed during inventory preparation");
        let value=json!({"schema_version":1,"glossary":g,"review_status":"pending","covered_units":[],"adopted":[],"unlisted":[],
            "no_unlisted_reason":null,"evidence_boundary":"A ledger records decisions; it does not prove translation correctness."});
        exclusive(&terminology,&value)?;inventory["terminology_path"]=json!(path_text(&terminology)?);
    }
    for (i,(_,p,_)) in ids.iter().enumerate(){ensure!(binding(p)?==bound[i],"source changed before inventory publication");}
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
