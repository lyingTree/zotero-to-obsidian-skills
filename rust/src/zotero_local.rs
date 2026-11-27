//! Zotero Desktop local API v3 over loopback. Ordinary reads are unauthenticated.
//! Explicit note writes use Zotero 10's in-client authorization.
//! HTTP/1.1 parser specifically for the loopback server.
use anyhow::{bail,ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,BTreeSet,HashMap,VecDeque},io::{BufRead,BufReader,Read,Write},net::{SocketAddr,Ipv4Addr,TcpStream},path::{Path,PathBuf},time::{Duration,Instant},fs};
use crate::{files,path,path_text,metadata,pdf_structure::Pdf};
const MAX_ROWS:usize=100_000;
const MAX_REQUESTS:usize=4096;
const MAX_HTTP_BODY:usize=512*1024*1024;
const PAGE:usize=100;
const BOUNDARY:&str="Actual read-only local Zotero API records, declared fields and independently inspected local attachment bytes. No attachment downloaded, no source-language or document-version decision inferred. Older server versions do not provide a coherent local version token; this is not a database transaction.";
fn key(s:&str)->Result<&str>{ensure!(s.len()==8&&s.bytes().all(|b|b.is_ascii_uppercase()||b.is_ascii_digit()),"Invalid Zotero object key");Ok(s)}
fn library(kind:&str,id:u64)->Result<String>{
    match kind{
        "user"=>Ok(format!("/api/users/{id}")),
        "group"=>{ensure!(id>0,"Group requires its actual ID");Ok(format!("/api/groups/{id}"))},
        _=>bail!("library_type must be user or group")
    }
}
pub(crate) struct Response{pub(crate) headers:BTreeMap<String,String>,pub(crate) body:Vec<u8>}
fn line<R:BufRead>(r:&mut R,limit:usize)->Result<Vec<u8>>{
    let mut out=Vec::new();
    loop{
        let bytes=r.fill_buf()?;
        ensure!(!bytes.is_empty(),"Truncated HTTP response");
        let count=bytes.iter().position(|b|*b==b'\n').map_or(bytes.len(),|n|n+1);
        ensure!(count<=limit.saturating_sub(out.len()),"HTTP header or chunk line too large");
        let end=bytes[count-1]==b'\n';
        out.extend_from_slice(&bytes[..count]);r.consume(count);
        if end{break;}
    }
    ensure!(out.ends_with(b"\r\n"),"Malformed HTTP line");out.truncate(out.len()-2);Ok(out)
}
pub(crate) fn decode_http<R:BufRead>(reader:R)->Result<Response>{
    decode_http_with_limit(reader,MAX_HTTP_BODY)
}
fn decode_http_with_limit<R:BufRead>(reader:R,max_body:usize)->Result<Response>{
    decode_http_status(reader,max_body,200,true)
}
fn decode_http_status<R:BufRead>(mut reader:R,max_body:usize,expected:u16,reading:bool)->Result<Response>{
    let status=line(&mut reader,8192)?;
    let status=std::str::from_utf8(&status)?;
    let mut fields=status.splitn(3,' ');
    ensure!(matches!(fields.next(),Some("HTTP/1.1"|"HTTP/1.0")),"Unexpected HTTP protocol");
    let code=fields.next().context("Missing HTTP status")?.parse::<u16>()?;
    if reading&&code==403{bail!("ZOTERO_LOCAL_API_DISABLED: enable Zotero local API in Advanced settings");}
    ensure!(code==expected,"ZOTERO_HTTP_STATUS_{code}: expected {expected}");
    let mut headers=BTreeMap::<String,String>::new();let mut budget:usize=64*1024;
    loop{
        let row=line(&mut reader,8192)?;budget=budget.checked_sub(row.len()+2).context("HTTP headers exceed budget")?;
        if row.is_empty(){break;}
        let row=std::str::from_utf8(&row)?;
        let (name,value)=row.split_once(':').context("Malformed response header")?;
        ensure!(!name.is_empty()&&name.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-'),"Invalid header field");
        let name=name.to_ascii_lowercase();let value=value.trim();
        ensure!(!value.chars().any(|c|c.is_control()&&c!='\t'),"Invalid HTTP header value");
        if matches!(name.as_str(),"link"|"vary"|"cache-control"|"pragma"|"access-control-expose-headers"|"warning"){
            headers.entry(name).and_modify(|s|{s.push_str(", ");s.push_str(value);}).or_insert_with(||value.into());
        }else if let Some(old)=headers.insert(name.clone(),value.into()){
            ensure!(old==value,"Conflicting response header: {name}");
        }
    }
    if code==204{return Ok(Response{headers,body:Vec::new()});}
    ensure!(headers.get("content-encoding").is_none_or(|s|s.eq_ignore_ascii_case("identity")),"Unexpected compressed HTTP response");
    let mut body=Vec::new();
    if let Some(transfer)=headers.get("transfer-encoding"){
        ensure!(transfer.eq_ignore_ascii_case("chunked")&&!headers.contains_key("content-length"),"Unsupported or ambiguous HTTP framing");
        let mut chunks=0usize;
        loop{
            chunks+=1;ensure!(chunks<=1_000_000,"HTTP chunk count exceeds budget");
            let row=line(&mut reader,1024)?;
            let size=std::str::from_utf8(&row)?.split(';').next().unwrap().trim();
            let n=usize::from_str_radix(size,16)?;
            if n==0{
                let mut budget=64*1024usize;
                loop{let tail=line(&mut reader,8192)?;budget=budget.checked_sub(tail.len()+2).context("HTTP trailers too large")?;if tail.is_empty(){break;}}
                break;
            }
            let next_len=body.len().checked_add(n).filter(|&total|total<=max_body).context("HTTP body exceeds budget")?;
            let start=body.len();body.resize(next_len,0);reader.read_exact(&mut body[start..])?;
            let mut crlf=[0;2];reader.read_exact(&mut crlf)?;ensure!(crlf==*b"\r\n","Malformed chunk terminator");
        }
    }else if let Some(length)=headers.get("content-length"){
        let n=length.parse::<usize>()?;
        ensure!(n<=max_body,"HTTP body exceeds budget");
        body.resize(n,0);reader.read_exact(&mut body)?;
    }else{
        (&mut reader).take(u64::try_from(max_body).unwrap_or(u64::MAX).saturating_add(1)).read_to_end(&mut body)?;
        ensure!(body.len()<=max_body,"HTTP body exceeds budget");
    }
    Ok(Response{headers,body})
}
#[derive(Clone,Copy,PartialEq,Eq)]
enum VersionScope{Object,Library}
pub struct SearchOptions<'a>{
    pub query:&'a str,
    pub qmode:&'a str,
    pub item_type:Option<&'a str>,
    pub top_only:bool,
    pub limit:usize,
    pub start:usize,
}
struct Client{addr:SocketAddr,started:Instant,requests:usize,connect_timeout:Duration,read_timeout:Duration,server_id:Option<String>,library_version:Option<String>,observations:Vec<Value>,bytes:usize}
impl Client{
    fn new(addr:SocketAddr)->Self{
        Self{addr,started:Instant::now(),requests:0,connect_timeout:Duration::from_secs(5),read_timeout:Duration::from_secs(60),server_id:None,library_version:None,observations:vec![],bytes:0}
    }
    fn with_timeouts(addr:SocketAddr,connect_secs:u64,read_secs:u64)->Self{
        Self{addr,started:Instant::now(),requests:0,connect_timeout:Duration::from_secs(connect_secs),read_timeout:Duration::from_secs(read_secs),server_id:None,library_version:None,observations:vec![],bytes:0}
    }
    fn connect(&mut self,method:&str,route:&str)->Result<(TcpStream,String)>{
        ensure!(self.addr.ip().is_loopback(),"Local API may only access loopback");
        ensure!(route.starts_with("/api/")&&!route.chars().any(char::is_whitespace)&&!route.contains(['#','\\','\r','\n']),"Unsafe local API route");
        ensure!(self.requests<MAX_REQUESTS&&self.started.elapsed()<Duration::from_secs(300),"Local API request budget exceeded");
        self.requests+=1;
        let stream=TcpStream::connect_timeout(&self.addr,self.connect_timeout).context("ZOTERO_LOCAL_API_UNAVAILABLE: open Zotero and enable local API")?;
        stream.set_read_timeout(Some(self.read_timeout))?;stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut header=format!("{method} {route} HTTP/1.1\r\nHost: localhost:{}\r\nUser-Agent: z2o-rust\r\nZotero-Allowed-Request: true\r\nZotero-API-Version: 3\r\nAccept: application/json\r\nAccept-Encoding: identity\r\nConnection: close\r\n",self.addr.port());
        if let Some(id)=&self.server_id{
            ensure!(!id.contains(['\r','\n']),"Invalid server identity");header.push_str(&format!("Zotero-Server-ID: {id}\r\n"));
        }
        Ok((stream,header))
    }
    fn get(&mut self,route:&str,scope:VersionScope)->Result<(Value,BTreeMap<String,String>)>{
        let (mut stream,mut header)=self.connect("GET",route)?;
        header.push_str("\r\n");stream.write_all(header.as_bytes())?;
        let response=decode_http(BufReader::new(stream))?;
        ensure!(response.headers.get("zotero-api-version").is_none_or(|s|s=="3"),"Unsupported local Zotero API version");
        let id=response.headers.get("zotero-server-id").cloned();
        let version=response.headers.get("last-modified-version").cloned();
        if let Some(prior)=&self.server_id{ensure!(id.as_ref()==Some(prior),"Zotero server identity changed during read");}
        if scope==VersionScope::Library{
            if let Some(prior)=&self.library_version{
                if self.server_id.is_some()&&version.is_some(){ensure!(version.as_ref()==Some(prior),"Zotero library version changed during read; retry");}
            }
            if self.library_version.is_none(){self.library_version=version.clone();}
        }
        if self.server_id.is_none(){self.server_id=id.clone();}
        self.bytes=self.bytes.saturating_add(response.body.len());
        let body=crate::parse_json(std::str::from_utf8(&response.body)?)?;
        let mut obs=json!({"route":route,"sha256":files::digest(&response.body),"bytes":response.body.len(),
            "api_version":response.headers.get("zotero-api-version"),"server_id":id});
        match scope{
            VersionScope::Library=>{
                obs["version_scope"]=json!("library");
                obs["library_version"]=json!(version);
            }
            VersionScope::Object=>{
                obs["version_scope"]=json!("object");
                obs["object_version"]=json!(version);
                obs["library_version"]=Value::Null;
            }
        }
        self.observations.push(obs);
        Ok((body,response.headers))
    }
    fn object(&mut self,route:&str,wanted:&str,kind:&str,id:u64)->Result<Value>{
        let (v,_)=self.get(&format!("{route}?format=json&include=data"),VersionScope::Object)?;
        check_object(&v,wanted,kind,id)?;Ok(v)
    }
    fn all(&mut self,route:&str,kind:&str,id:u64,mut progress:impl FnMut(usize,usize,&str))->Result<Vec<Value>>{
        let mut out=vec![];let mut seen=BTreeSet::new();let mut total=None;
        loop{
            let (v,h)=self.get(&format!("{route}?format=json&include=data&limit={PAGE}&start={}",out.len()),VersionScope::Library)?;
            let rows=v.as_array().context("Zotero paginated result must be an array")?;
            ensure!(rows.len()<=PAGE&&out.len()+rows.len()<=MAX_ROWS,"Zotero returned invalid page size or exceeded row budget");
            if let Some(n)=h.get("total-results"){
                let n=n.parse::<usize>()?;ensure!(n<=MAX_ROWS&&total.is_none_or(|t|t==n),"Zotero total changed or exceeded budget");total=Some(n);
            }
            let more=h.get("link").is_some_and(|s|s.contains("rel=\"next\"")||s.contains("rel=next"));
            for row in rows{
                let k=key(row["key"].as_str().context("Missing Zotero key")?)?;
                check_object(row,k,kind,id)?;
                ensure!(seen.insert(k.to_owned()),"Repeated Zotero result; pagination or library changed");
                out.push(row.clone());
            }
            if let Some(t)=total{ensure!(out.len()<=t,"Zotero pagination exceeds declared total");}
            progress(out.len(),total.unwrap_or(out.len()),"Zotero local API records");
            if total==Some(out.len()){
                ensure!(!more,"Inconsistent Zotero pagination");break;
            }
            if rows.is_empty(){ensure!(!more&&total.is_none(),"Zotero pagination incomplete");break;}
            if total.is_none()&&!more&&rows.len()<PAGE{break;}
        }
        Ok(out)
    }
}
fn check_object(v:&Value,wanted:&str,kind:&str,id:u64)->Result<()>{
    ensure!(v.is_object()&&v["key"]==wanted&&v["data"].is_object()&&v["data"]["key"]==wanted,"Zotero object identity mismatch");
    ensure!(v["library"]["type"]==kind,"Zotero object library differs");
    let actual=v["library"]["id"].as_u64().context("Actual Zotero library identity missing")?;
    ensure!(actual>0&&(id==0&&kind=="user"||actual==id),"Zotero library identity differs");Ok(())
}

impl Client {
    fn send_json(
        &mut self,
        method: &str,
        route: &str,
        payload: &Value,
        condition: Option<(&str, u64)>,
        expected_status: u16,
    ) -> Result<Response> {
        let server_id = self.server_id.clone().context("Zotero 10+ local API is required for writes")?;
        let (mut stream, mut header) = self.connect(method, route)?;
        if let Some((credential, version)) = condition {
            header.push_str(&format!(
                "Zotero-API-Key: {credential}\r\nIf-Unmodified-Since-Version: {version}\r\n"
            ));
        } else {
            // The desktop authorization dialog waits for the user's choice.
            stream.set_read_timeout(Some(Duration::from_secs(180)))?;
        }
        let body = serde_json::to_vec(payload)?;
        header.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()));
        stream.write_all(header.as_bytes())?;
        stream.write_all(&body)?;
        let response_limit = if condition.is_some() { 32 * 1024 * 1024 } else { 16 * 1024 };
        let response = decode_http_status(BufReader::new(stream), response_limit, expected_status, false)?;
        ensure!(response.headers.get("zotero-server-id") == Some(&server_id), "Zotero server identity changed");
        Ok(response)
    }

    fn authorize_write(&mut self) -> Result<String> {
        let response = self.send_json("POST", "/api/local/authorize", &json!({"appName":"z2o"}), None, 200)
            .context("Zotero write authorization did not succeed; check the Zotero Desktop dialog and local API settings")?;
        // Keep the credential in this invocation only. Never include response data in an error.
        let grant: Value = serde_json::from_slice(&response.body).ok().context("Invalid Zotero authorization response")?;
        let credential = grant["key"].as_str().filter(|value| value.len() == 32
            && value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            .context("Invalid Zotero authorization credential")?;
        Ok(credential.to_owned())
    }
}

pub struct NoteSyncOptions<'a> {
    pub sync_id: &'a str,
    pub html_path: &'a Path,
    pub authorized: bool,
    pub expected_version: Option<u64>,
    pub expected_server_id: Option<&'a str>,
}

fn is_synced_note(item: &Value, parent_key: &str, tag: &str) -> bool {
    let data = &item["data"];
    data["itemType"] == "note" && data["parentItem"] == parent_key
        && data["tags"].as_array().is_some_and(|tags| tags.iter().any(|row| row["tag"] == tag))
}

fn note_content(html: &str) -> &str {
    let html = html.trim();
    // Zotero may add the standard note editor wrapper to an HTML fragment.
    html.strip_prefix("<div data-schema-version=\"9\">")
        .and_then(|inner| inner.strip_suffix("</div>"))
        .map_or(html, str::trim)
}

pub fn sync_child_note(kind: &str, id: u64, parent_key: &str, options: NoteSyncOptions<'_>) -> Result<Value> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 23119));
    sync_note(Client::new(addr), kind, id, parent_key, options)
}

fn sync_note(mut client: Client, kind: &str, id: u64, parent_key: &str, options: NoteSyncOptions<'_>) -> Result<Value> {
    const MAX_NOTE_BYTES: u64 = 4 * 1024 * 1024;
    key(parent_key)?;
    ensure!(!options.sync_id.is_empty() && options.sync_id.len() <= 80
        && options.sync_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "sync_id must be a stable identifier of up to 80 ASCII letters, digits, dots, underscores or hyphens");
    let mut bytes = Vec::new();
    fs::File::open(options.html_path)?.take(MAX_NOTE_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_NOTE_BYTES, "Zotero note HTML exceeds 4 MiB");
    let html = std::str::from_utf8(&bytes)?.trim_start_matches('\u{feff}').trim();
    ensure!(!html.is_empty(), "Zotero note HTML is empty");
    ensure!(!html.bytes().any(|byte| matches!(byte, 0..=8 | 11..=12 | 14..=31 | 127)),
        "Zotero note HTML contains control characters");

    let parent = client.object(&format!("{}/items/{parent_key}", library(kind, id)?), parent_key, kind, id)?;
    let actual_id = parent["library"]["id"].as_u64().context("Missing Zotero library ID")?;
    let item_type = parent["data"]["itemType"].as_str().context("Missing Zotero parent item type")?;
    ensure!(!matches!(item_type, "note" | "attachment" | "annotation") && parent["data"]["deleted"] != true,
        "Zotero parent must be a live bibliographic item");
    let server_id = client.server_id.clone().context("Zotero 10+ local API is required for note sync")?;
    if let Some(expected) = options.expected_server_id {
        ensure!(expected == server_id.as_str(), "Zotero database differs from the reviewed note");
    }
    let items = format!("{}/items", library(kind, actual_id)?);
    let children = client.all(&format!("{items}/{parent_key}/children"), kind, actual_id, |_, _, _| {})?;
    let tag = format!("z2o-sync:{}", options.sync_id);
    let mut matches = children.iter().filter(|item| is_synced_note(item, parent_key, &tag));
    let existing = matches.next();
    ensure!(matches.next().is_none(), "Multiple Zotero notes share this sync_id; resolve the conflict before writing");
    let desired_sha = files::digest(note_content(html).as_bytes());
    let mut receipt = json!({
        "server_id":server_id, "library_type":kind, "library_id":actual_id, "parent_key":parent_key,
        "sync_id":options.sync_id, "requested_content_sha256":desired_sha
    });
    let mut existing_version = None;
    if let Some(item) = existing {
        ensure!(item["data"]["deleted"] != true, "The matched Zotero note is in the trash");
        let current = item["data"]["note"].as_str().context("Missing Zotero note content")?;
        let version = item["version"].as_u64().context("Missing Zotero note version")?;
        receipt["note_key"] = item["key"].clone();
        receipt["version"] = json!(version);
        receipt["current_content_sha256"] = json!(files::digest(note_content(current).as_bytes()));
        if note_content(current) == note_content(html) {
            receipt["status"] = json!("unchanged");
            return Ok(receipt);
        }
        existing_version = Some(version);
    }
    if !options.authorized {
        receipt["status"] = json!(if existing.is_some() {"would_update"} else {"would_create"});
        return Ok(receipt);
    }

    let (method, route, payload, version, status) = if let Some(item) = existing {
        ensure!(options.expected_server_id.is_some() && options.expected_version.is_some()
            && options.expected_version == existing_version,
            "ZOTERO_NOTE_CONFLICT: review the existing note and pass its current version and server ID before updating");
        let note_key = item["key"].as_str().context("Missing Zotero note key")?;
        ("PATCH", format!("{items}/{note_key}"), json!({"note":html}),
            existing_version.context("Missing Zotero note version")?, 204)
    } else {
        ensure!(options.expected_version.is_none(), "The previously reviewed Zotero note no longer exists");
        let library_version = client.library_version.as_deref().context("Missing Zotero library version")?.parse::<u64>()?;
        ("POST", items.clone(), json!([{
            "itemType":"note", "parentItem":parent_key, "note":html,
            "tags":[{"tag":tag}], "collections":[], "relations":{}
        }]), library_version, 200)
    };
    let credential = client.authorize_write()?;
    let write = client.send_json(method, &route, &payload, Some((&credential, version)), status)
        .context("Zotero note write was not confirmed; inspect the current note before retrying")?;
    let note_key = if let Some(item) = existing {
        item["key"].as_str().context("Missing Zotero note key")?.to_owned()
    } else {
        let result: Value = serde_json::from_slice(&write.body).context("Invalid Zotero creation receipt; inspect children before retrying")?;
        ensure!(result["failed"]["0"].is_null(), "Zotero rejected child note creation (item code {})", result["failed"]["0"]["code"]);
        let created = result["successful"]["0"]["key"].as_str().or_else(|| result["success"]["0"].as_str())
            .context("Zotero creation response lacks a success key; inspect children before retrying")?;
        key(created)?.to_owned()
    };
    let saved = client.object(&format!("{items}/{note_key}"), &note_key, kind, actual_id)
        .with_context(|| format!("Zotero accepted the write to {note_key}, but readback failed"))?;
    ensure!(is_synced_note(&saved, parent_key, &tag) && saved["data"]["deleted"] != true
        && saved["data"]["note"].as_str().is_some_and(|current| note_content(current) == note_content(html)),
        "Zotero accepted the write to {note_key}, but the saved note differs from the request");
    receipt["status"] = json!("verified_zotero_note_write");
    receipt["action"] = json!(if existing.is_some() {"updated"} else {"created"});
    receipt["note_key"] = json!(note_key);
    receipt["version"] = json!(saved["version"].as_u64().context("Missing saved Zotero note version")?);
    receipt["current_content_sha256"] = json!(desired_sha);
    Ok(receipt)
}
fn local_version_consistent(client:&Client)->bool{
    let Some(expected_ver)=client.library_version.as_deref() else{return false;};
    if client.observations.is_empty(){return false;}
    let first_server=client.observations[0]["server_id"].as_str();
    let seen_any_server=client.server_id.is_some()||client.observations.iter().any(|o|!o["server_id"].is_null());
    if seen_any_server&&!client.observations.iter().all(|o|o["server_id"].as_str()==first_server&&first_server.is_some()){
        return false;
    }
    let expected_num=expected_ver.parse::<u64>().ok();
    let mut saw_library=false;
    let mut object_versions:HashMap<&str,Option<&str>>=HashMap::new();
    for o in &client.observations{
        if o.get("version_scope").and_then(Value::as_str)==Some("object"){
            let obj_ver=o.get("object_version").and_then(Value::as_str);
            if let (Some(lib_n),Some(obj_n))=(expected_num,obj_ver.and_then(|s|s.parse::<u64>().ok())){
                if obj_n>lib_n{return false;}
            }
            if let Some(route)=o.get("route").and_then(Value::as_str){
                if let Some(&prior)=object_versions.get(route){
                    if prior!=obj_ver{return false;}
                }else{
                    object_versions.insert(route,obj_ver);
                }
            }
        }else{
            saw_library=true;
            if o["library_version"].as_str()!=Some(expected_ver){return false;}
        }
    }
    saw_library
}
fn attachment_path(raw:&Value,root:Option<&Path>,paths:&Value)->Result<Option<PathBuf>>{
    let k=key(raw["key"].as_str().context("Attachment key")?)?;
    if let Some(p)=paths.get(k){return Ok(Some(path(p)?));}
    let mode=raw["data"]["linkMode"].as_str().unwrap_or("");
    if mode=="linked_file"{
        if let Some(raw_path)=raw["data"]["path"].as_str().filter(|s|!s.trim().is_empty()){
            let p=PathBuf::from(raw_path);
            if p.is_absolute()&&p.exists(){return Ok(Some(p));}
        }
        return Ok(None);
    }
    let Some(root)=root else{return Ok(None);};
    if !matches!(mode,"imported_file"|"imported_url"){return Ok(None);}
    let name=raw["data"]["filename"].as_str().filter(|s|!s.is_empty());
    let Some(name)=name else{return Ok(None);};
    ensure!(!name.contains(['/', '\\', ':','\0'])&&!matches!(name,"."|".."),"Unsafe stored attachment filename");
    let root=files::canonical(root)?;
    let folder=root.join(k);
    if !folder.exists(){return Ok(Some(folder.join(name)));}
    let folder=files::canonical(&folder)?;
    ensure!(folder.starts_with(&root),"Stored attachment directory escaped storage");
    let target=folder.join(name);
    if target.exists(){ensure!(files::canonical(&target)?.starts_with(&root),"Stored attachment escaped storage");}
    Ok(Some(target))
}
fn attachment(raw:&Value,root:Option<&Path>,paths:&Value)->Value{
    let result=(||->Result<Value>{
        let Some(p)=attachment_path(raw,root,paths)? else{
            return Ok(json!({"availability":if raw["data"]["linkMode"]=="linked_url"{"link_only"}else{"path_unresolved"},
                "format_validity":"not_assessed"}));
        };
        if !p.exists(){return Ok(json!({"availability":"missing","path":path_text(&p)?,"format_validity":"not_assessed"}));}
        let p=files::canonical(&p)?;let snapshot=files::snapshot(&p)?;
        let mut f=fs::File::open(&p)?;let mut signature=[0;16];let n=f.read(&mut signature)?;
        let signature=&signature[..n];
        let format=if signature.starts_with(b"%PDF-"){"pdf"}else if signature.starts_with(b"PK\x03\x04"){"zip_container"}
            else if signature.starts_with(b"\x89PNG\r\n\x1a\n"){"png"}else if signature.starts_with(b"\xff\xd8\xff"){"jpeg"}else{"other_or_unknown"};
        let ext=p.extension().and_then(|s|s.to_str()).unwrap_or("").to_ascii_lowercase();
        let validity=if n==0{json!("empty")}
            else if ext=="pdf"||format=="pdf"{match Pdf::open(&p){Ok(v)=>json!({"status":"parsed","format":"pdf","pages":v.len()}),Err(e)=>json!({"status":"invalid_or_unsupported","reason":format!("{e:#}")})}}
            else if ext=="docx"{match crate::docx::read_docx(&p,"attachment",|_,_,_|{}){Ok(_)=>json!({"status":"parsed","format":"docx"}),Err(e)=>json!({"status":"invalid_or_unsupported","reason":format!("{e:#}")})}}
            else{json!("not_assessed")};
        ensure!(files::snapshot(&p)?==snapshot,"Attachment changed during inspection");
        Ok(json!({"availability":if n==0{"empty"}else{"readable"},"file":snapshot,"signature_hex":signature.iter().map(|b|format!("{b:02x}")).collect::<String>(),
            "detected_signature":format,"format_validity":validity}))
    })();
    result.unwrap_or_else(|e|json!({"availability":"blocked","reason":format!("{e:#}"),"format_validity":"not_assessed"}))
}
fn inspect(mut client:Client,kind:&str,id:u64,item_key:&str,storage_root:Option<&Path>,paths:&Value,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    key(item_key)?;let prefix=library(kind,id)?;
    let paths=paths.as_object().context("attachment_paths must map actual attachment keys to explicit local files")?;
    for(k,v)in paths{key(k)?;path(v)?;}
    let paths=Value::Object(paths.clone());
    let item=client.object(&format!("{prefix}/items/{item_key}"),item_key,kind,id)?;
    ensure!(!matches!(item["data"]["itemType"].as_str(),Some("attachment"|"note"|"annotation")),"Select a parent literature item");
    let actual_id=item["library"]["id"].as_u64().unwrap();
    let direct=item["data"]["collections"].as_array().context("Original collection membership unavailable")?;
    let mut queue=VecDeque::<String>::new();let mut seen=BTreeSet::new();
    for k in direct{let k=key(k.as_str().context("Collection key required")?)?;queue.push_back(k.into());}
    let mut collections=Vec::new();let mut issues=Vec::new();
    while let Some(k)=queue.pop_front(){
        if !seen.insert(k.clone()){continue;}
        ensure!(seen.len()<=MAX_REQUESTS,"Collection ancestor budget exceeded");
        match client.object(&format!("{prefix}/collections/{k}"),&k,kind,actual_id){
            Ok(c)=>{
                match &c["data"]["parentCollection"]{Value::Bool(false)=>{},Value::String(p)=>{key(p)?;queue.push_back(p.clone());},_=>issues.push(json!({"collection":k,"reason":"parent_unavailable"}))}
                collections.push(c);
            }
            Err(e)=>issues.push(json!({"collection":k,"reason":format!("{e:#}")})),
        }
    }
    let ancestry=metadata::collection_ancestry(&item,&json!(collections),issues.is_empty())?;
    let children=client.all(&format!("{prefix}/items/{item_key}/children"),kind,actual_id,&mut progress)?;
    ensure!(children.iter().all(|c|c["data"]["parentItem"]==item_key),"Unexpected child parent identity");
    let mut attachments=vec![];let mut descendant=vec![];let mut used=BTreeSet::new();
    for (i,c) in children.iter().enumerate(){
        if c["data"]["itemType"]!="attachment"{continue;}
        let k=key(c["key"].as_str().unwrap())?;
        used.insert(k.to_owned());
        let local=attachment(c,storage_root,&paths);
        // Annotations and notes are records, not extra downloaded files.
        let rows=client.all(&format!("{prefix}/items/{k}/children"),kind,actual_id,&mut progress)?;
        ensure!(rows.iter().all(|r|r["data"]["parentItem"]==k),"Unexpected attachment child parent identity");
        descendant.extend(rows);
        attachments.push(json!({"key":k,"raw":c,"link_mode":c["data"]["linkMode"],"declared_mime":c["data"]["contentType"],
            "local":local,"role":if c["data"]["linkMode"]=="linked_url"{"link_only"}else{"pending_source_review"},
            "version_status":"not_assessed"}));
        progress(i+1,children.len(),"inspect local attachment records");
    }
    ensure!(paths.as_object().unwrap().keys().all(|k|used.contains(k)),"Explicit attachment path refers to an unregistered child");
    let final_item=client.object(&format!("{prefix}/items/{item_key}"),item_key,kind,actual_id)?;
    ensure!(final_item==item,"Zotero parent record changed during inspection; retry");
    let stable=local_version_consistent(&client);
    Ok(json!({"schema_version":1,"raw":item,"library":final_item["library"],"item_key":item_key,
        "collection_records":collections,"collection_ancestry":ancestry,"collection_issues":issues,
        "children":children,"attachment_children":descendant,"attachments":attachments,
        "declared_language":final_item["data"]["language"],"language":"unknown","language_status":"pending_fulltext_review",
        "main_attachment_status":"pending_version_review","api_reads":client.observations,"local_version_consistent":stable,
        "attachment_downloaded":false,"zotero_modified":false,"source_fidelity":"not_assessed","boundary":BOUNDARY}))
}
/// Full top-level catalog in one explicitly selected local library, with
/// actual page observations and no attachment downloads. For very large
/// libraries the budget fails explicitly rather than returning a false EOF.
pub fn catalog(kind:&str,id:u64,mut progress:impl FnMut(usize,usize,&str))->Result<Value>{
    let prefix=library(kind,id)?;
    let mut client=Client::new(SocketAddr::from((Ipv4Addr::LOCALHOST,23119)));
    let items=client.all(&format!("{prefix}/items/top"),kind,id,&mut progress)?;
    let actual=items.first().map(|r|r["library"]["id"].as_u64().unwrap()).or(if id>0{Some(id)}else{None});
    ensure!(items.iter().all(|r|r["library"]["id"].as_u64()==actual),"Zotero catalog spans more than one actual library");
    let stable=local_version_consistent(&client);
    Ok(json!({"schema_version":1,"library":{"type":kind,"id":actual},"items":items,
        "pagination_complete":true,"local_version_consistent":stable,"api_reads":client.observations,
        "attachment_downloaded":false,"zotero_modified":false,"source_fidelity":"not_assessed","boundary":BOUNDARY}))
}
pub fn inspect_item(kind:&str,id:u64,item_key:&str,storage_root:Option<&Path>,paths:&Value,progress:impl FnMut(usize,usize,&str))->Result<Value>{
    inspect(Client::new(SocketAddr::from((Ipv4Addr::LOCALHOST,23119))),kind,id,item_key,storage_root,paths,progress)
}
pub(crate) fn inspect_item_at(addr:SocketAddr,kind:&str,id:u64,item_key:&str,storage_root:Option<&Path>,paths:&Value,progress:impl FnMut(usize,usize,&str))->Result<Value>{
    inspect(Client::new(addr),kind,id,item_key,storage_root,paths,progress)
}
pub(crate) fn parse_loopback_api_addr(api_url:&str)->Result<SocketAddr>{
    let trimmed=api_url.trim().trim_end_matches('/');
    let rest=trimmed.strip_prefix("http://").context("zotero_local_api must start with http://")?;
    let host_port=rest.split('/').next().unwrap_or("");
    let (host,port)=host_port.split_once(':').context("zotero_local_api must specify explicit loopback port")?;
    ensure!(host=="127.0.0.1"||host.eq_ignore_ascii_case("localhost"),"zotero_local_api must target loopback host");
    let port:u16=port.parse().context("Invalid zotero_local_api port")?;
    Ok(SocketAddr::from((Ipv4Addr::LOCALHOST,port)))
}
pub fn probe_api(api_url:&str)->Value{
    let addr=match parse_loopback_api_addr(api_url){
        Ok(a)=>a,
        Err(e)=>return json!({"reachable":false,"api_url":api_url,"error":format!("{e:#}")}),
    };
    probe_client(addr,api_url)
}
pub(crate) fn probe_client(addr:SocketAddr,api_url:&str)->Value{
    let mut client=Client::with_timeouts(addr,2,5);
    match client.get("/api/users/0/items/top?format=json&include=data&limit=1",VersionScope::Library){
        Ok((v,_))=>{
            let lib=v.as_array().and_then(|a|a.first()).map(|row|row["library"].clone()).filter(|v|v.is_object());
            json!({"reachable":true,"api_url":api_url,"server_id":client.server_id,"library_version":client.library_version,"library":lib})
        }
        Err(e)=>json!({"reachable":false,"api_url":api_url,"error":format!("{e:#}")}),
    }
}
fn search(mut client:Client,kind:&str,id:u64,opt:SearchOptions<'_>)->Result<Value>{
    let prefix=library(kind,id)?;
    let query=opt.query.trim();
    ensure!(!query.is_empty()&&query.len()<=1024&&!query.chars().any(char::is_control),"query must be nonempty text without control characters");
    ensure!(matches!(opt.qmode,"titleCreatorYear"|"everything"),"qmode must be titleCreatorYear or everything");
    let item_type=opt.item_type.map(|s|{
        let s=s.trim();
        ensure!(!s.is_empty()&&s.len()<=256&&!s.chars().any(char::is_control),"item_type must be nonempty text without control characters");
        Ok::<_,anyhow::Error>(s)
    }).transpose()?;
    ensure!((1..=PAGE).contains(&opt.limit),"limit must be between 1 and {PAGE}");
    ensure!(opt.start<=MAX_ROWS,"start exceeds row budget");
    let base=if opt.top_only{format!("{prefix}/items/top")}else{format!("{prefix}/items")};
    let enc_q=percent_encoding::utf8_percent_encode(query,percent_encoding::NON_ALPHANUMERIC).to_string();
    let mut route=format!("{base}?format=json&include=data&q={enc_q}&qmode={}&limit={}&start={}",opt.qmode,opt.limit,opt.start);
    if let Some(t)=item_type{
        let enc_t=percent_encoding::utf8_percent_encode(t,percent_encoding::NON_ALPHANUMERIC).to_string();
        route.push_str(&format!("&itemType={enc_t}"));
    }
    let (v,h)=client.get(&route,VersionScope::Library)?;
    let rows=v.as_array().context("Zotero search result must be an array")?;
    ensure!(rows.len()<=opt.limit,"Zotero returned invalid search page size");
    let total=h.get("total-results").map(|n|->Result<usize>{
        let n=n.parse::<usize>()?;
        ensure!(n<=MAX_ROWS,"Zotero total exceeded budget");
        Ok(n)
    }).transpose()?;
    let link_more=h.get("link").is_some_and(|s|s.contains("rel=\"next\"")||s.contains("rel=next"));
    let mut items=Vec::with_capacity(rows.len());
    let mut seen=BTreeSet::new();
    for row in rows{
        let k=key(row["key"].as_str().context("Missing Zotero key")?)?;
        check_object(row,k,kind,id)?;
        ensure!(seen.insert(k.to_owned()),"Repeated Zotero search result");
        items.push(row.clone());
    }
    let actual=items.first().map(|r|r["library"]["id"].as_u64().unwrap()).or(if id>0{Some(id)}else{None});
    ensure!(items.iter().all(|r|r["library"]["id"].as_u64()==actual),"Zotero search spans more than one actual library");
    let has_more=total.is_some_and(|t|opt.start.saturating_add(items.len())<t)||link_more;
    let stable=local_version_consistent(&client);
    Ok(json!({
        "schema_version":1,"library":{"type":kind,"id":actual},
        "query":query,"qmode":opt.qmode,"item_type":item_type,"top_only":opt.top_only,
        "limit":opt.limit,"start":opt.start,"returned_count":items.len(),"total_results":total,"has_more":has_more,
        "items":items,"local_version_consistent":stable,"api_reads":client.observations,
        "attachment_downloaded":false,"zotero_modified":false,"source_fidelity":"not_assessed","boundary":BOUNDARY
    }))
}
pub fn search_items(kind:&str,id:u64,opt:SearchOptions<'_>)->Result<Value>{
    search(Client::new(SocketAddr::from((Ipv4Addr::LOCALHOST,23119))),kind,id,opt)
}
pub(crate) fn search_items_at(addr:SocketAddr,kind:&str,id:u64,opt:SearchOptions<'_>)->Result<Value>{
    search(Client::new(addr),kind,id,opt)
}
#[cfg(test)]
mod tests{
    use super::*;
    use std::{io::Cursor,net::TcpListener,thread};

    #[test]
    fn note_sync_previews_creates_updates_and_rejects_stale_versions() {
        const PARENT: &str = "/api/users/123/items/PARENT01?format=json&include=data";
        const CHILDREN: &str = "/api/users/123/items/PARENT01/children?format=json&include=data&limit=100&start=0";
        const ITEMS: &str = "/api/users/123/items";
        const NOTE: &str = "/api/users/123/items/NOTE0001";
        const HTML: &str = "<p>Current source</p>";
        const CREDENTIAL: &str = "0123456789abcdef0123456789abcdef";
        let directory = tempfile::tempdir().unwrap();
        let html_path = directory.path().join("note.html");
        fs::write(&html_path, HTML).unwrap();
        let parent = json!({"key":"PARENT01","version":1,"library":{"type":"user","id":123},
            "data":{"key":"PARENT01","itemType":"journalArticle"}});
        let note = |html: &str, version: u64| json!({
            "key":"NOTE0001", "version":version, "library":{"type":"user","id":123},
            "data":{"key":"NOTE0001","itemType":"note","parentItem":"PARENT01","note":html,
                "tags":[{"tag":"z2o-sync:paper"}]}
        });

        for (action, expected_version, current, expected_status) in [
            ("preview", None, None, "would_create"),
            ("create", None, None, "verified_zotero_note_write"),
            ("update", Some(4), Some("<p>Old source</p>"), "verified_zotero_note_write"),
            ("stale", Some(3), Some("<p>User edited</p>"), "conflict"),
            ("same", None, Some(HTML), "unchanged"),
        ] {
            let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let addr = server.local_addr().unwrap();
            let mut exchanges = vec![
                ("GET", PARENT.to_owned(), 200, None, parent.clone()),
                ("GET", CHILDREN.to_owned(), 200, None, json!(current.map(|html| note(html, 4)).into_iter().collect::<Vec<_>>())),
            ];
            if matches!(action, "create" | "update") {
                exchanges.push(("POST", "/api/local/authorize".to_owned(), 200,
                    Some(json!({"appName":"z2o"})), json!({"key":CREDENTIAL,"remember":false})));
                if action == "create" {
                    exchanges.push(("POST", ITEMS.to_owned(), 200, Some(json!([{
                        "itemType":"note","parentItem":"PARENT01","note":HTML,
                        "tags":[{"tag":"z2o-sync:paper"}],"collections":[],"relations":{}
                    }])), json!({"success":{"0":"NOTE0001"},"failed":{}})));
                } else {
                    exchanges.push(("PATCH", NOTE.to_owned(), 204, Some(json!({"note":HTML})), Value::Null));
                }
                exchanges.push(("GET", format!("{NOTE}?format=json&include=data"), 200, None, note(HTML, 8)));
            }
            let handle = thread::spawn(move || {
                for (method, route, status, payload, response) in exchanges {
                    let (mut stream, _) = server.accept().unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut reader = BufReader::new(&mut stream);
                    assert_eq!(String::from_utf8(line(&mut reader, 8192).unwrap()).unwrap(),
                        format!("{method} {route} HTTP/1.1"));
                    let mut headers = BTreeMap::new();
                    loop {
                        let row = line(&mut reader, 8192).unwrap();
                        if row.is_empty() { break; }
                        let row = std::str::from_utf8(&row).unwrap();
                        let (name, value) = row.split_once(':').unwrap();
                        headers.insert(name.to_owned(), value.trim().to_owned());
                    }
                    if let Some(payload) = payload {
                        let mut body = vec![0; headers["Content-Length"].parse().unwrap()];
                        reader.read_exact(&mut body).unwrap();
                        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), payload);
                        assert_eq!(headers["Zotero-Server-ID"], "test-instance");
                        if route != "/api/local/authorize" {
                            assert_eq!(headers["Zotero-API-Key"], CREDENTIAL);
                            assert_eq!(headers["If-Unmodified-Since-Version"], if method == "PATCH" {"4"} else {"7"});
                        }
                    }
                    drop(reader);
                    let body = if status == 204 { Vec::new() } else { serde_json::to_vec(&response).unwrap() };
                    write!(stream, "HTTP/1.1 {status} Response\r\nZotero-Server-ID: test-instance\r\nLast-Modified-Version: 7\r\nContent-Length: {}\r\n\r\n", body.len()).unwrap();
                    stream.write_all(&body).unwrap();
                }
            });
            let result = sync_note(Client::with_timeouts(addr, 3, 5), "user", 123, "PARENT01", NoteSyncOptions {
                sync_id: "paper", html_path: &html_path, authorized: action != "preview",
                expected_version, expected_server_id: expected_version.map(|_| "test-instance"),
            });
            if expected_status == "conflict" {
                assert!(result.unwrap_err().to_string().contains("ZOTERO_NOTE_CONFLICT"));
            } else {
                let receipt = result.unwrap();
                assert_eq!(receipt["status"], expected_status);
                assert!(!receipt.to_string().contains(CREDENTIAL));
            }
            handle.join().unwrap();
        }
    }

    #[test]
    fn bounded_http_framing(){
        let raw=b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n";
        assert_eq!(decode_http(Cursor::new(raw)).unwrap().body,b"{}");
        let joined=b"HTTP/1.1 200 OK\r\nVary: Origin\r\nVary: Accept\r\nCache-Control: no-cache\r\nCache-Control: no-store\r\nPragma: no-cache\r\nPragma: x\r\nAccess-Control-Expose-Headers: X-A\r\nAccess-Control-Expose-Headers: X-B\r\nWarning: 199 a\r\nWarning: 199 b\r\nContent-Length: 2\r\n\r\n{}";
        let res=decode_http(Cursor::new(joined)).unwrap();
        assert_eq!(res.headers["vary"],"Origin, Accept");
        assert_eq!(res.headers["cache-control"],"no-cache, no-store");
        assert_eq!(res.headers["pragma"],"no-cache, x");
        assert_eq!(res.headers["access-control-expose-headers"],"X-A, X-B");
        assert_eq!(res.headers["warning"],"199 a, 199 b");
        assert!(decode_http(Cursor::new(b"HTTP/1.1 302 Found\r\nLocation: http://other/\r\n\r\n")).is_err());
        assert!(decode_http(Cursor::new(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n{}")).is_err());
        assert!(decode_http(Cursor::new(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n")).is_err());
        let huge_cl=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{{}}",MAX_HTTP_BODY+1);
        assert!(decode_http(Cursor::new(huge_cl.as_bytes())).is_err());
        let huge_chunk=format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{{}}\r\n0\r\n\r\n",MAX_HTTP_BODY+1);
        assert!(decode_http(Cursor::new(huge_chunk.as_bytes())).is_err());
        assert!(decode_http_with_limit(Cursor::new(b"HTTP/1.1 200 OK\r\n\r\nabcde"),4).is_err());
        assert_eq!(decode_http_with_limit(Cursor::new(b"HTTP/1.1 200 OK\r\n\r\nabcd"),4).unwrap().body,b"abcd");
    }
    #[test]
    fn local_version_consistency_allows_absent_server_id_but_rejects_mismatch(){
        let addr=SocketAddr::from((Ipv4Addr::LOCALHOST,23119));
        let mut ok_absent=Client::new(addr);
        ok_absent.library_version=Some("12".into());
        ok_absent.observations=vec![
            json!({"server_id":null,"library_version":"12"}),
            json!({"server_id":null,"library_version":"12"}),
        ];
        assert!(local_version_consistent(&ok_absent));

        let mut mixed_present=Client::new(addr);
        mixed_present.server_id=Some("srv-1".into());
        mixed_present.library_version=Some("12".into());
        mixed_present.observations=vec![
            json!({"server_id":null,"library_version":"12"}),
            json!({"server_id":"srv-1","library_version":"12"}),
        ];
        assert!(!local_version_consistent(&mixed_present));

        let mut disappeared=Client::new(addr);
        disappeared.server_id=Some("srv-1".into());
        disappeared.library_version=Some("12".into());
        disappeared.observations=vec![
            json!({"server_id":"srv-1","library_version":"12"}),
            json!({"server_id":null,"library_version":"12"}),
        ];
        assert!(!local_version_consistent(&disappeared));

        let mut missing_ver=Client::new(addr);
        missing_ver.observations=vec![json!({"server_id":null,"library_version":null})];
        assert!(!local_version_consistent(&missing_ver));
    }
    #[test]
    fn linked_file_resolves_absolute_path_without_storage_root(){
        let dir=tempfile::tempdir().unwrap();
        let file=dir.path().join("linked.pdf");
        fs::write(&file,b"%PDF-1.4\n").unwrap();
        let raw=json!({"key":"ATTACH09","data":{"linkMode":"linked_file","path":path_text(&file).unwrap()}});
        let out=attachment(&raw,None,&json!({}));
        assert_eq!(out["availability"],"readable");
        assert_eq!(out["file"]["path"],path_text(&files::canonical(&file).unwrap()).unwrap());
        let missing=json!({"key":"ATTACH10","data":{"linkMode":"linked_file","path":dir.path().join("missing.pdf").to_string_lossy()}});
        assert_eq!(attachment(&missing,None,&json!({}))["availability"],"path_unresolved");
    }
    #[test]
    fn actual_loopback_read_retains_raw_item_children_ancestors_and_no_download(){
        let server=TcpListener::bind((Ipv4Addr::LOCALHOST,0)).unwrap();let addr=server.local_addr().unwrap();
        let obj=|k:&str,data:Value|{let mut d=data;d["key"]=json!(k);json!({"key":k,"library":{"type":"user","id":123},"data":d,"version":7})};
        let parent=obj("PARENT01",json!({"itemType":"journalArticle","title":"Real fixture","collections":["COLLECT1"],"language":"en"}));
        let coll=obj("COLLECT1",json!({"name":"Ocean","parentCollection":false}));
        let att=obj("ATTACH01",json!({"itemType":"attachment","parentItem":"PARENT01","filename":"paper.pdf","linkMode":"imported_file","contentType":"application/pdf"}));
        let expected=parent.clone();
        let handle=thread::spawn(move||{
            let routes=[
                ("/api/users/0/items/PARENT01?format=json&include=data",parent.clone(),None),
                ("/api/users/0/collections/COLLECT1?format=json&include=data",coll,None),
                ("/api/users/0/items/PARENT01/children?format=json&include=data&limit=100&start=0",json!([att]),Some(1)),
                ("/api/users/0/items/ATTACH01/children?format=json&include=data&limit=100&start=0",json!([]),Some(0)),
                ("/api/users/0/items/PARENT01?format=json&include=data",parent,None)
            ];
            for (route,value,total) in routes{
                let (mut socket,_)=server.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let mut r=BufReader::new(&mut socket);
                let first=String::from_utf8(line(&mut r,8192).unwrap()).unwrap();
                assert_eq!(first,format!("GET {route} HTTP/1.1"));
                loop{if line(&mut r,8192).unwrap().is_empty(){break;}}
                drop(r);
                let bytes=serde_json::to_vec(&value).unwrap();
                write!(socket,"HTTP/1.1 200 OK\r\nZotero-API-Version: 3\r\nZotero-Server-ID: test-instance\r\nLast-Modified-Version: 7\r\nContent-Length: {}\r\n",bytes.len()).unwrap();
                if let Some(n)=total{write!(socket,"Total-Results: {n}\r\n").unwrap();}
                write!(socket,"\r\n").unwrap();socket.write_all(&bytes).unwrap();
            }
        });
        let report=inspect(Client::new(addr),"user",0,"PARENT01",None,&json!({}),|_,_,_|{}).unwrap();
        handle.join().unwrap();
        assert_eq!(report["raw"],expected);assert_eq!(report["collection_ancestry"]["status"],"complete");
        assert_eq!(report["attachments"][0]["local"]["availability"],"path_unresolved");
        assert_eq!(report["local_version_consistent"],true);assert_eq!(report["language"],"unknown");
        assert_eq!(report["attachment_downloaded"],false);assert_eq!(report["zotero_modified"],false);
    }
    #[test]
    fn inspect_allows_distinct_object_and_library_versions_like_zotero_desktop(){
        let server=TcpListener::bind((Ipv4Addr::LOCALHOST,0)).unwrap();let addr=server.local_addr().unwrap();
        let obj=|k:&str,ver:u64,data:Value|{let mut d=data;d["key"]=json!(k);json!({"key":k,"library":{"type":"user","id":9849314},"data":d,"version":ver})};
        let parent=obj("5U5RW3TA",9802,json!({"itemType":"preprint","title":"Mayer path homology","collections":["ZTLKJLS5"],"language":"en"}));
        let coll=obj("ZTLKJLS5",1882,json!({"name":"拓扑","parentCollection":false}));
        let att=obj("RUCADIA2",9805,json!({"itemType":"attachment","parentItem":"5U5RW3TA","filename":"paper.pdf","linkMode":"imported_file","contentType":"application/pdf"}));
        let expected=parent.clone();
        let handle=thread::spawn(move||{
            let routes=[
                ("/api/users/0/items/5U5RW3TA?format=json&include=data",parent.clone(),"9802",None),
                ("/api/users/0/collections/ZTLKJLS5?format=json&include=data",coll,"1882",None),
                ("/api/users/0/items/5U5RW3TA/children?format=json&include=data&limit=100&start=0",json!([att]),"9818",Some(1)),
                ("/api/users/0/items/RUCADIA2/children?format=json&include=data&limit=100&start=0",json!([]),"9818",Some(0)),
                ("/api/users/0/items/5U5RW3TA?format=json&include=data",parent,"9802",None),
            ];
            for (route,value,ver,total) in routes{
                let (mut socket,_)=server.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let mut r=BufReader::new(&mut socket);
                let first=String::from_utf8(line(&mut r,8192).unwrap()).unwrap();
                assert_eq!(first,format!("GET {route} HTTP/1.1"));
                loop{if line(&mut r,8192).unwrap().is_empty(){break;}}
                drop(r);
                let bytes=serde_json::to_vec(&value).unwrap();
                write!(socket,"HTTP/1.1 200 OK\r\nZotero-API-Version: 3\r\nZotero-Server-Id: K8Ms6zL58J8z\r\nLast-Modified-Version: {ver}\r\nContent-Length: {}\r\n",bytes.len()).unwrap();
                if let Some(n)=total{write!(socket,"Total-Results: {n}\r\n").unwrap();}
                write!(socket,"\r\n").unwrap();socket.write_all(&bytes).unwrap();
            }
        });
        let report=inspect(Client::new(addr),"user",0,"5U5RW3TA",None,&json!({}),|_,_,_|{}).unwrap();
        handle.join().unwrap();
        assert_eq!(report["raw"],expected);
        assert_eq!(report["collection_ancestry"]["status"],"complete");
        assert_eq!(report["local_version_consistent"],true);
    }
    #[test]
    fn inspect_rejects_library_version_drift_across_list_reads(){
        let server=TcpListener::bind((Ipv4Addr::LOCALHOST,0)).unwrap();let addr=server.local_addr().unwrap();
        let obj=|k:&str,ver:u64,data:Value|{let mut d=data;d["key"]=json!(k);json!({"key":k,"library":{"type":"user","id":123},"data":d,"version":ver})};
        let parent=obj("PARENT01",9802,json!({"itemType":"journalArticle","title":"T","collections":[]}));
        let att=obj("ATTACH01",9805,json!({"itemType":"attachment","parentItem":"PARENT01","filename":"p.pdf","linkMode":"imported_file","contentType":"application/pdf"}));
        let handle=thread::spawn(move||{
            let routes=[
                ("/api/users/0/items/PARENT01?format=json&include=data",parent,"9802",None),
                ("/api/users/0/items/PARENT01/children?format=json&include=data&limit=100&start=0",json!([att]),"9818",Some(1)),
                ("/api/users/0/items/ATTACH01/children?format=json&include=data&limit=100&start=0",json!([]),"9819",Some(0)),
            ];
            for (route,value,ver,total) in routes{
                let (mut socket,_)=server.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let mut r=BufReader::new(&mut socket);
                let first=String::from_utf8(line(&mut r,8192).unwrap()).unwrap();
                assert_eq!(first,format!("GET {route} HTTP/1.1"));
                loop{if line(&mut r,8192).unwrap().is_empty(){break;}}
                drop(r);
                let bytes=serde_json::to_vec(&value).unwrap();
                write!(socket,"HTTP/1.1 200 OK\r\nZotero-API-Version: 3\r\nZotero-Server-ID: K8Ms6zL58J8z\r\nLast-Modified-Version: {ver}\r\nContent-Length: {}\r\n",bytes.len()).unwrap();
                if let Some(n)=total{write!(socket,"Total-Results: {n}\r\n").unwrap();}
                write!(socket,"\r\n").unwrap();socket.write_all(&bytes).unwrap();
            }
        });
        let err=inspect(Client::new(addr),"user",0,"PARENT01",None,&json!({}),|_,_,_|{}).unwrap_err();
        handle.join().unwrap();
        assert!(err.to_string().contains("Zotero library version changed during read; retry"),"{err}");
    }
    #[test]
    fn search_items_loopback_percent_encodes_and_reports_totals(){
        let server=TcpListener::bind((Ipv4Addr::LOCALHOST,0)).unwrap();let addr=server.local_addr().unwrap();
        let row=json!({"key":"5U5RW3TA","library":{"type":"user","id":9849314},"version":9802,"data":{"key":"5U5RW3TA","itemType":"preprint","title":"Mayer 同调"}});
        let handle=thread::spawn(move||{
            let (mut socket,_)=server.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut r=BufReader::new(&mut socket);
            let first=String::from_utf8(line(&mut r,8192).unwrap()).unwrap();
            let enc_q=percent_encoding::utf8_percent_encode("mayer 同调",percent_encoding::NON_ALPHANUMERIC).to_string();
            assert_eq!(first,format!("GET /api/users/0/items/top?format=json&include=data&q={enc_q}&qmode=everything&limit=5&start=1&itemType=preprint HTTP/1.1"));
            loop{if line(&mut r,8192).unwrap().is_empty(){break;}}
            drop(r);
            let bytes=serde_json::to_vec(&json!([row])).unwrap();
            write!(socket,"HTTP/1.1 200 OK\r\nZotero-API-Version: 3\r\nZotero-Server-Id: K8Ms6zL58J8z\r\nLast-Modified-Version: 9818\r\nTotal-Results: 3\r\nContent-Length: {}\r\n\r\n",bytes.len()).unwrap();
            socket.write_all(&bytes).unwrap();
        });
        let res=search(Client::new(addr),"user",0,SearchOptions{
            query:"mayer 同调",qmode:"everything",item_type:Some("preprint"),top_only:true,limit:5,start:1,
        }).unwrap();
        handle.join().unwrap();
        assert_eq!(res["returned_count"],1);
        assert_eq!(res["total_results"],3);
        assert_eq!(res["has_more"],true);
        assert_eq!(res["library"],json!({"type":"user","id":9849314}));
        assert_eq!(res["items"][0]["key"],"5U5RW3TA");
        assert_eq!(res["local_version_consistent"],true);
    }
}
