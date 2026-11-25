//! Source URL leads and one-to-one occurrence assignment, not source fidelity.
use anyhow::{anyhow,ensure,Result};
use serde_json::{json,Value};
use std::collections::{HashMap,HashSet,VecDeque};

fn url_tokens(text:&str)->Vec<(usize,usize,&str)> {
    let bytes=text.as_bytes();
    let mut result=vec![];
    let mut cursor=0usize;
    let mut offset=0usize;
    let mut offset_byte=0usize;
    while cursor<bytes.len(){
        if cursor>0&&(bytes[cursor-1].is_ascii_alphanumeric()||bytes[cursor-1]==b'_'){
            cursor+=1;continue;
        }
        let prefix:[&[u8];3]=[b"https://",b"http://",b"www."];
        let Some(size)=prefix.iter().find_map(|p|bytes.get(cursor..cursor+p.len())
            .filter(|s|s.eq_ignore_ascii_case(p)).map(|_|p.len()))else{cursor+=1;continue;};
        let start=cursor;
        let mut end=start+size;
        let mut brackets=Vec::new();
        for ch in text[end..].chars(){
            if ch.is_whitespace()||matches!(ch,'\u{001c}'..='\u{001f}'|'<'|'>'|'"'|'\''){break;}
            match ch{
                '('=>brackets.push(')'), '['=>brackets.push(']'), '{'=>brackets.push('}'),
                ')'|']'|'}'=>{if brackets.last()!=Some(&ch){break;}brackets.pop();}
                _=>{}
            }
            end+=ch.len_utf8();
        }
        offset+=text[offset_byte..start].chars().count();
        let finish=offset+text[start..end].chars().count();
        result.push((offset,finish,&text[start..end]));
        offset=finish;offset_byte=end;cursor=end;
    }
    result
}
fn token(mut text:&str)->&str{
    const END:&[char]=&['.',',',';',':','!','?'];
    text=text.trim_end_matches(END);
    for (open,close) in [('(',')'),('[',']'),('{','}') ]{
        let mut excess=text.matches(close).count().saturating_sub(text.matches(open).count());
        while excess>0&&text.ends_with(close){
            text=text.strip_suffix(close).unwrap().trim_end_matches(END);excess-=1;
        }
    }
    text
}
pub fn scan(inventory:&Value,excluded_blocks:&Value,excluded_pages:&Value)->Result<Value>{
    let sources=inventory.get("sources").and_then(Value::as_array).ok_or_else(||anyhow!("provide an inventory containing its actual sources"))?;
    let blocks=excluded_blocks.as_array().ok_or_else(||anyhow!("excluded_blocks must be an array"))?;
    let pages=excluded_pages.as_array().ok_or_else(||anyhow!("excluded_pages must be an array"))?;
    let mut ignored_blocks=HashSet::new();
    for block in blocks{ignored_blocks.insert(block.as_str().ok_or_else(||anyhow!("excluded block ID must be a string"))?);}
    let mut ignored_pages=HashSet::new();
    for page in pages{
        let pair=page.as_array().filter(|p|p.len()==2).ok_or_else(||anyhow!("excluded page must be [source,page]"))?;
        ignored_pages.insert((pair[0].as_str().ok_or_else(||anyhow!("excluded source must be a string"))?,
            pair[1].as_u64().ok_or_else(||anyhow!("excluded page must be an integer"))?));
    }
    let mut result=vec![];
    for source in sources{
        let Some(sid)=source.get("id").and_then(Value::as_str).filter(|v|!v.is_empty())else{continue;};
        if source["format"]=="docx"{
            let Some(blocks)=source["native_blocks"].as_array()else{continue;};
            for block in blocks{
                let (Some(id),Some(text),Some(locator))=(block["id"].as_str(),block["text"].as_str(),block.get("locator").filter(|v|v.is_object()))else{continue;};
                if ignored_blocks.contains(id){continue;}
                for (number,(start,end,raw)) in url_tokens(text).into_iter().enumerate(){
                    result.push(json!({"id":format!("{id}:url:{}",number+1),"source":sid,"native_locator":locator,"block":id,
                        "start":start,"end":end,"raw_text":raw,"lookup_text":token(raw),
                        "boundary":"Native XML text lead; verify its source context before using a URL."}));
                }
            }
            continue;
        }
        let Some(pages)=source["pages"].as_array()else{continue;};
        for page in pages{
            let Some(number)=page["page"].as_u64().filter(|n|*n>0)else{continue;};
            if ignored_pages.contains(&(sid,number)){continue;}
            let Some(blocks)=page["blocks"].as_array()else{continue;};
            for block in blocks{
                let (Some(id),Some(text))=(block["id"].as_str(),block["text"].as_str())else{continue;};
                if ignored_blocks.contains(id){continue;}
                for (index,(start,end,raw)) in url_tokens(text).into_iter().enumerate(){
                    result.push(json!({"id":format!("{id}:url:{}",index+1),"source":sid,"page":number,"block":id,
                        "start":start,"end":end,"raw_text":raw,"lookup_text":token(raw),
                        "boundary":"Text-layer lead; verify the visible source before using a URL."}));
                }
            }
        }
    }
    Ok(json!(result))
}
fn at_location(unit:&Value,occ:&Value)->bool{
    if unit["source"]!=occ["source"]{return false;}
    let Some(records)=unit["evidence"].as_array()else{return false;};
    for record in records{
        if !record.is_object(){continue;}
        let blocks=record["blocks"].as_array().filter(|v|!v.is_empty());
        if occ.get("native_locator").is_some(){
            if record["basis"]!="native"||record.get("page").is_some()||record["native_locator"]!=occ["native_locator"]{continue;}
            if let Some(blocks)=blocks{
                if blocks.len()==1&&blocks[0]==occ["block"]{return true;}continue;
            }
            return true;
        }
        if record.get("native_locator").is_some()||record["page"]!=occ["page"]{continue;}
        if blocks.is_some_and(|v|!v.contains(&occ["block"])){continue;}
        return true;
    }
    false
}
fn forms(unit:&Value)->HashSet<String>{
    let mut result=HashSet::new();
    for field in ["display_text","value"]{
        if let Some(v)=unit[field].as_str().map(str::trim).filter(|v|!v.is_empty()){result.insert(token(v).to_owned());}
    }
    if unit["field"]=="doi"{
        for v in result.clone(){
            if v.as_bytes().get(..4).is_some_and(|s|s.eq_ignore_ascii_case(b"doi:")){
                let suffix=v[4..].trim_start();
                if suffix.starts_with("10."){result.insert(suffix.to_owned());}
            }
        }
        for v in result.clone(){
            if v.starts_with("10."){
                for prefix in ["https://doi.org/","http://doi.org/","https://dx.doi.org/","http://dx.doi.org/"]{
                    result.insert(format!("{prefix}{v}"));
                }
            }
        }
    }
    for v in result.clone(){
        for prefix in ["https://","http://"]{
            if v.as_bytes().get(..prefix.len()).is_some_and(|s|s.eq_ignore_ascii_case(prefix.as_bytes()))&&
               v.as_bytes().get(prefix.len()..prefix.len()+4).is_some_and(|s|s.eq_ignore_ascii_case(b"www.")){
                result.insert(v[prefix.len()..].to_owned());break;
            }
        }
    }
    result
}
pub fn coverage(inventory:&Value,excluded_blocks:&Value,excluded_pages:&Value)->Result<Value>{
    let scanned=scan(inventory,excluded_blocks,excluded_pages)?;
    let occurrences=scanned.as_array().unwrap();
    let mut occurrence_ids=HashMap::new();
    for (i,row) in occurrences.iter().enumerate(){ensure!(occurrence_ids.insert(row["id"].as_str().unwrap(),i).is_none(),"duplicate occurrence ID");}
    let mut units:Vec<&Value>=vec![];
    let mut unit_ids=HashMap::new();
    if let Some(rows)=inventory["units"].as_array(){
        for row in rows{
            let Some(id)=row["id"].as_str()else{continue;};
            if row["kind"]!="metadata"||!matches!(row["field"].as_str(),Some("external_url"|"doi")){continue;}
            ensure!(!unit_ids.contains_key(id),"duplicate metadata unit ID");
            unit_ids.insert(id,units.len());units.push(row);
        }
    }
    let mut assigned=vec![None;occurrences.len()];
    let mut used=vec![false;units.len()];
    let mut explicit=vec![false;units.len()];
    let mut invalid=vec![];
    for (i,unit) in units.iter().enumerate(){
        if unit["source_url_occurrence"].is_null(){continue;}
        explicit[i]=true;
        let reference=unit["source_url_occurrence"].as_str().and_then(|v|occurrence_ids.get(v)).copied();
        let reason=match reference{
            None=>Some("source_url_occurrence must identify one actual text-layer occurrence."),
            Some(j) if !at_location(unit,&occurrences[j])=>Some("The explicit occurrence is outside this unit's source location."),
            Some(j) if assigned[j].is_some()=>Some("Another source unit already binds this same occurrence."),
            Some(j)=>{assigned[j]=Some(i);used[i]=true;None}
        };
        if let Some(reason)=reason{invalid.push(json!({"unit":unit["id"],"reason":reason}));}
    }
    // Index plausible candidates by (source,page,raw-token). Preserve input
    // unit order for deterministic augmenting paths and repeated occurrences.
    let mut by_form:HashMap<(String,Option<u64>,String),Vec<usize>>=HashMap::new();
    for (i,unit) in units.iter().enumerate(){
        if explicit[i]{continue;}
        let (Some(source),Some(records))=(unit["source"].as_str(),unit["evidence"].as_array())else{continue;};
        let mut pages=HashSet::new();
        for record in records{
            if let Some(page)=record["page"].as_u64().filter(|v|*v>0){pages.insert(Some(page));}
            if record["basis"]=="native"&&record["native_locator"].is_object()&&record.get("page").is_none(){pages.insert(None);}
        }
        for page in pages{
            for form in forms(unit){by_form.entry((source.to_owned(),page,form)).or_default().push(i);}
        }
    }
    let mut edges=vec![vec![];occurrences.len()];
    for (i,row) in occurrences.iter().enumerate(){
        if assigned[i].is_some(){continue;}
        let key=(row["source"].as_str().unwrap().to_owned(),row["page"].as_u64(),row["lookup_text"].as_str().unwrap().to_owned());
        if let Some(candidates)=by_form.get(&key){
            edges[i]=candidates.iter().copied().filter(|u|!used[*u]&&at_location(units[*u],row)).collect();
        }
    }
    let mut unit_to_lead=vec![None;units.len()];
    for start in 0..edges.len(){
        if assigned[start].is_some(){continue;}
        let mut queue=VecDeque::from([start]);
        let mut seen=HashSet::from([start]);
        let mut parent=HashMap::new();
        let mut free=None;
        while let Some(lead)=queue.pop_front(){
            for &unit in &edges[lead]{
                if parent.contains_key(&unit){continue;}
                parent.insert(unit,lead);
                match unit_to_lead[unit]{
                    None=>{free=Some(unit);break;},
                    Some(old) if seen.insert(old)=>queue.push_back(old),
                    _=>{}
                }
            }
            if free.is_some(){break;}
        }
        while let Some(unit)=free{
            let lead=parent[&unit];
            let previous=assigned[lead].replace(unit);
            unit_to_lead[unit]=Some(lead);
            free=previous;
        }
    }
    let mut recorded=vec![];let mut unmapped=vec![];
    for (i,row) in occurrences.iter().enumerate(){
        if let Some(unit)=assigned[i]{recorded.push(json!({"occurrence":row["id"],"unit":units[unit]["id"]}));}
        else{unmapped.push(row.clone());}
    }
    Ok(json!({"occurrences":occurrences,"recorded":recorded,"unmapped":unmapped,"invalid_bindings":invalid,"source_fidelity":"not_assessed"}))
}
