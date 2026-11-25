//! Rich Table merge ranges, validated against concrete worksheet IDs and
//! optional source grid coordinates. No inferred or rewritten cell identities.
use serde_json::{json,Value};
use std::collections::{HashMap,HashSet};
type Rect=(usize,usize,usize,usize);
#[derive(Clone,Debug,serde::Serialize,PartialEq,Eq)]
pub struct GridFinding{pub code:&'static str,pub identity:String,pub message:String}
fn fail(out:&mut Vec<GridFinding>,code:&'static str,id:&str,message:impl Into<String>){out.push(GridFinding{code,identity:id.into(),message:message.into()});}
fn ids(items:&[Value],rows:bool)->Option<Vec<&str>>{
    let mut result=Vec::with_capacity(items.len());let mut seen=HashSet::new();
    for row in items{
        let key=row["id"].as_str().filter(|s|!s.is_empty()&&(!rows||*s!="header"))?;
        if !seen.insert(key){return None;}result.push(key);
    }Some(result)
}
fn coordinate(v:&Value)->Option<usize>{v.as_u64().and_then(|n|usize::try_from(n).ok())}
fn endpoint(value:&Value,rows:&HashMap<&str,usize>,columns:&HashMap<&str,usize>)->Option<(usize,usize)>{
    let s=value.as_str()?;
    let mut candidates=s.match_indices('.').filter_map(|(i,_)|Some((*rows.get(&s[..i])?,*columns.get(&s[i+1..])?)));
    let first=candidates.next()?;if candidates.next().is_some(){None}else{Some(first)}
}
fn nonempty(v:&Value)->bool{!v.is_null()&&!v.as_str().is_some_and(|s|s.trim().is_empty())}
fn rects(ranges:&[Rect])->String{format!("[{}]",ranges.iter().map(|(a,b,c,d)|format!("({a}, {b}, {c}, {d})")).collect::<Vec<_>>().join(", "))}
pub fn check_merges(identity:&str,unit:Option<&Value>,data:&Value)->Vec<GridFinding>{
    let mut out=vec![];
    let(Some(rows),Some(columns))=(data["rows"].as_array(),data["columns"].as_array())else{return out;};
    let Some(row_ids)=ids(rows,true)else{fail(&mut out,"TABLE_ROW_IDS",identity,"Every data row needs a unique nonempty ID other than the header sentinel.");return out;};
    let Some(column_ids)=ids(columns,false)else{fail(&mut out,"TABLE_COLUMN_IDS",identity,"Every column needs a unique nonempty ID.");return out;};
    let row_index:HashMap<_,_>=row_ids.iter().enumerate().map(|(i,s)|(*s,i+1)).chain(std::iter::once(("header",0))).collect();
    let col_index:HashMap<_,_>=column_ids.iter().enumerate().map(|(i,s)|(*s,i+1)).collect();
    for row in rows{
        let Some(cells)=row.get("cells")else{continue;};
        let Some(cells)=cells.as_object()else{fail(&mut out,"TABLE_ROW_CELLS",identity,"Each row's cells must be a mapping keyed by column ID.");continue;};
        if cells.keys().any(|c|!col_index.contains_key(c.as_str())){
            fail(&mut out,"TABLE_CELL_COLUMN",identity,"A cell refers to an unknown column ID in this worksheet.");
        }
    }
    let empty=vec![];
    let actual=if let Some(v)=data.get("merges"){
        let Some(v)=v.as_array()else{fail(&mut out,"TABLE_MERGE_TYPE",identity,"merges must be an array of anchor/end objects.");return out;};v
    }else{&empty};
    let mut ranges=Vec::new();
    for merge in actual{
        if !merge.is_object(){fail(&mut out,"TABLE_MERGE_ENDPOINT",identity,"A merge needs string anchor and end endpoints.");continue;}
        let(Some(a),Some(b))=(endpoint(&merge["anchor"],&row_index,&col_index),endpoint(&merge["end"],&row_index,&col_index))else{
            fail(&mut out,"TABLE_MERGE_ENDPOINT",identity,"The merged-cell endpoint does not identify a cell in this worksheet.");continue;
        };
        if a.0>b.0||a.1>b.1||a==b||(a.0==0&&b.0!=0){
            fail(&mut out,"TABLE_MERGE_RANGE",identity,"Use an ordered nontrivial rectangular range wholly in the header or data grid.");continue;
        }
        ranges.push((a.0,b.0,a.1,b.1));
        for row in a.0..=b.0{
            // Inspect only populated cells, avoiding area-proportional scans of
            // sparse tables. Reference emits at most one covered-value finding per row.
            let covered=if row==0{
                (a.1..=b.1).any(|col|(row,col)!=a&&nonempty(&columns[col-1]["name"]))
            }else{
                rows[row-1]["cells"].as_object().is_some_and(|cells|cells.iter().any(|(key,v)|
                    col_index.get(key.as_str()).is_some_and(|col|a.1<=*col&&*col<=b.1&&(row,*col)!=a)&&nonempty(v)))
            };
            if covered{fail(&mut out,"TABLE_MERGE_COVERED_VALUE",identity,"Only the top-left anchor of a merged range may carry visible content.");}
        }
    }
    ranges.sort_unstable();
    for(i,left)in ranges.iter().enumerate(){
        for right in &ranges[i+1..]{
            if right.0>left.1{break;}
            if left.2<=right.3&&right.2<=left.3{fail(&mut out,"TABLE_MERGE_OVERLAP",identity,"Merged ranges must not overlap.");}
        }
    }
    if let Some(expected)=unit.and_then(|u|u["merges"].as_array()){
        let mut expected_ranges=Vec::new();
        for merge in expected{
            let(Some(a),Some(b),Some(c),Some(d))=(coordinate(&merge["row_start"]),coordinate(&merge["row_end"]),coordinate(&merge["column_start"]),coordinate(&merge["column_end"]))else{
                // A negative integer is syntactically an integer but out of the grid.
                let keys=["row_start","row_end","column_start","column_end"];
                let integers=keys.iter().all(|k|merge[*k].as_i64().is_some()||merge[*k].as_u64().is_some());
                fail(&mut out,"SOURCE_MERGE_RANGE",identity,if integers{"A source merged range falls outside the recorded grid."}else{"Source merges need integer row_start/row_end/column_start/column_end."});continue;
            };
            if a>b||b>rows.len()||c==0||c>d||d>columns.len(){fail(&mut out,"SOURCE_MERGE_RANGE",identity,"A source merged range falls outside the recorded grid.");continue;}
            expected_ranges.push((a,b,c,d));
        }
        expected_ranges.sort_unstable();
        if expected_ranges!=ranges{fail(&mut out,"TABLE_MERGES_MISMATCH",identity,format!("Source merged ranges {} differ from target ranges {}.",rects(&expected_ranges),rects(&ranges)));}
    }out
}
pub fn report(identity:&str,unit:Option<&Value>,data:&Value)->Value{
    let findings=check_merges(identity,unit,data);json!({"findings":findings,"source_fidelity":"not_assessed"})
}
