//! Public report projections and explicit native script launchers.
//! Report accessors always run the underlying file-bound validator again.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,BTreeSet},io::{self,IsTerminal,Write},path::Path,fs};
use crate::files;

pub fn progress_line(done:usize,total:usize,label:&str){
    // stderr preserves stdout's JSON protocol; strip terminal control input.
    let label:String=label.chars().map(|c|if c.is_control(){' '}else{c}).take(512).collect();
    let stderr=io::stderr();
    let is_tty=stderr.is_terminal();
    if !is_tty && total>0 && done<total {return;}
    let mut out=stderr.lock();
    if is_tty {
        let width=20;
        let filled=if total==0{width}else{((done.min(total) as u128*width as u128)/total as u128) as usize};
        let _=write!(out,"\r[{}{}] {done}/{total} {label}{}","#".repeat(filled),".".repeat(width-filled),if done>=total{"\n"}else{""});
    } else {
        let _=writeln!(out,"[{done}/{total}] {label}");
    }
    let _=out.flush();
}
pub fn throttled_progress(done:usize,total:usize,label:&str){
    if total==0 || done>=total {
        progress_line(done,total,label);
    } else if io::stderr().is_terminal() && (done==1 || done%64==0) {
        progress_line(done,total,label);
    }
}

pub fn project_report(r:&Value,method:&str,finding_index:Option<usize>)->Result<Value>{
    if let Some(i)=finding_index{
        ensure!(method=="to_dict","Finding only supports to_dict");
        return r["findings"].as_array().context("Actual findings required")?.get(i).filter(|v|v.is_object()).cloned().context("Finding index outside actual report");
    }
    if matches!(method,"to_dict"|"as_dict"){return Ok(r.clone());}
    ensure!(matches!(method,"records_ready"|"offline_records_ready"|"recorded_checks_complete"|"structurally_consistent"),"Unknown report projection");
    Ok(json!(r[method].as_bool().context("Expected boolean from actual validator")?))
}
pub fn completion_summary(r:&Value,n:usize)->Result<Value>{
    ensure!(n>0,"examples_per_code must be positive");
    let findings=r["findings"].as_array().context("Actual workflow findings required")?;
    let pending=r["pending_acceptance"].as_array().context("Actual pending acceptance required")?;
    let mut seen=BTreeSet::new();
    let mut counts=BTreeMap::<String,usize>::new();let mut examples=BTreeMap::<String,Vec<Value>>::new();
    for f in findings{
        let code=f["code"].as_str().context("Finding code")?;let target=f["target"].as_str().context("Finding target")?;let message=f["message"].as_str().context("Finding message")?;
        if !seen.insert((code,target,message)){continue;}
        *counts.entry(code.to_owned()).or_default()+=1;
        let rows=examples.entry(code.to_owned()).or_default();
        if rows.len()<n{rows.push(json!({"target":target,"message":message}));}
    }
    Ok(json!({"records_ready":r["records_ready"],"source_fidelity":r["source_fidelity"],
        "finding_count":findings.len(),"distinct_finding_count":seen.len(),"counts_by_code":counts,"examples_by_code":examples,
        "pending_file_goal_count":pending.len(),"next_pending_file_goals":pending.iter().take(n).collect::<Vec<_>>(),
        "boundary":"This is a bounded display of the full report, not a smaller acceptance scope."}))
}
pub fn save_new_report(p:&Path,report:&Value)->Result<Value>{
    ensure!(!p.exists()&&!p.is_symlink(),"Keep existing report or choose a new version");
    let parent=p.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let dest=files::canonical(parent)?.join(p.file_name().context("Report filename")?);
    crate::handoff_native::publish(&dest,report)
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn summary_deduplicates_for_display_without_changing_pending_goals(){
        let f=json!({"code":"X","target":"one","message":"pending"});
        let report=json!({"findings":[f.clone(),f,{"code":"X","target":"two","message":"different"}],
            "pending_acceptance":[1,2,3],"records_ready":false,"source_fidelity":"not_assessed"});
        let summary=completion_summary(&report,1).unwrap();
        assert_eq!(summary["finding_count"],3);assert_eq!(summary["distinct_finding_count"],2);
        assert_eq!(summary["counts_by_code"]["X"],2);assert_eq!(summary["examples_by_code"]["X"].as_array().unwrap().len(),1);
        assert_eq!(summary["pending_file_goal_count"],3);assert_eq!(summary["records_ready"],false);
        assert!(completion_summary(&report,0).is_err());
        assert_eq!(project_report(&report,"records_ready",None).unwrap(),false);
    }
}
