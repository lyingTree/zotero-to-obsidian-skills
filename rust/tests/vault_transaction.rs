use serde_json::{json,Value};
use std::{fs,path::{Path,PathBuf}};
use z2o_native::{vault_publish::{self,CommitOptions},files};
struct Fixture{_tmp:tempfile::TempDir,vault:PathBuf,backup:PathBuf,candidate:PathBuf,note:PathBuf,sha:String}
impl Fixture{
    fn new()->Self{
        let tmp=tempfile::tempdir().unwrap();let vault=tmp.path().join("vault");let backup=tmp.path().join("backup");
        fs::create_dir_all(vault.join(".obsidian")).unwrap();fs::create_dir_all(vault.join("文献")).unwrap();fs::create_dir(&backup).unwrap();
        let candidate=tmp.path().join("candidate.md");fs::write(&candidate,"---\n年份: 2020\n---\n# Paper\n\nActual note body.\n").unwrap();
        let note=vault.join("文献/Paper.md");let sha=files::snapshot(&candidate).unwrap()["sha256"].as_str().unwrap().to_owned();
        Self{_tmp:tmp,vault,backup,candidate,note,sha}
    }
    fn commit(&self,prior:Option<&str>,auth:bool)->anyhow::Result<Value>{
        vault_publish::commit(CommitOptions{vault:&self.vault,backup_root:&self.backup,literature_relative:"文献",
            assets_relative:"文献/attachments",note_relative:"文献/Paper.md",candidate:&self.candidate,candidate_sha:&self.sha,
            expected_existing_sha:prior,resources:&json!([]),authorized:auth,scope:"standalone_markdown",workflow_run:None},|_,_,_|{})
    }
}
fn rollback(r:&Value)->Value{vault_publish::rollback(Path::new(r["transaction"]["path"].as_str().unwrap()),r["transaction"]["sha256"].as_str().unwrap(),true).unwrap()}
#[test]
fn actual_create_and_rollback(){
    let f=Fixture::new();assert!(f.commit(None,false).is_err());assert!(!f.note.exists());
    let r=f.commit(None,true).unwrap();assert_eq!(r["status"],"verified_file_write","{r}");
    assert_eq!(fs::read(&f.note).unwrap(),fs::read(&f.candidate).unwrap());assert_eq!(r["full_workflow_accepted"],false);
    assert_eq!(rollback(&r)["note_disposition"],"removed_unchanged_created_note");assert!(!f.note.exists());
    assert_eq!(rollback(&r)["note_disposition"],"already_restored_or_not_written");
}
#[test]
fn restore_prior_bytes_and_keep_user_changes(){
    let f=Fixture::new();let original=b"Original user Markdown\r\n";fs::write(&f.note,original).unwrap();
    let h=files::digest(original);assert!(f.commit(Some(&"0".repeat(64)),true).is_err());
    let r=f.commit(Some(&h),true).unwrap();assert_eq!(r["status"],"verified_file_write","{r}");
    assert_eq!(rollback(&r)["note_disposition"],"restored_original");assert_eq!(fs::read(&f.note).unwrap(),original);
    let r=f.commit(Some(&h),true).unwrap();fs::write(&f.note,b"User modified after commit").unwrap();
    assert_eq!(rollback(&r)["note_disposition"],"preserved_newer_or_unknown_note");assert_eq!(fs::read(&f.note).unwrap(),b"User modified after commit");
}
