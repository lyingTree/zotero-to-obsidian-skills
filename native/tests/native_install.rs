use serde_json::{json,Value};
use std::{fs,path::Path,process::{Command,Output}};
use z2o_native::{native_install::SKILLS,files};
fn run(exe:&Path,args:&[&str])->Output{Command::new(exe).args(args).env("PATH","").output().unwrap()}
fn success(output:Output)->Value{
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}
fn path(p:&Path)->&str{p.to_str().unwrap()}
#[test]
fn actual_offline_native_install_reuse_backup_and_tamper_protection(){
    let root=tempfile::tempdir().unwrap();let base=root.path();let source=base.join("source");
    fs::create_dir(&source).unwrap();
    for &name in SKILLS{
        let p=source.join(name);fs::create_dir(&p).unwrap();
        fs::write(p.join("SKILL.md"),format!("---\nname: {name}\ndescription: test\n---\n\n# Actual source skill\n")).unwrap();
        fs::write(p.join("reference.py"),b"raise RuntimeError('must never execute')\n").unwrap();
    }
    let exe=Path::new(env!("CARGO_BIN_EXE_z2o"));
    let bundle=base.join("bundle");let result=success(run(exe,&["package",path(&source),path(&bundle)]));
    assert_eq!(result["skill_count"],12);
    let manifest=bundle.join("package.json");
    let sha=files::snapshot(&manifest).unwrap()["sha256"].as_str().unwrap().to_owned();
    assert_eq!(result["package_manifest"]["sha256"],sha);
    let installed=base.join("skills");fs::create_dir(&installed).unwrap();
    let old=installed.join(SKILLS[0]);fs::create_dir(&old).unwrap();
    fs::write(old.join("SKILL.md"),format!("---\nname: {}\ndescription: legacy\n---\n",SKILLS[0])).unwrap();
    fs::write(old.join("local-user-data.md"),b"Keep this original user data").unwrap();
    let receipt=success(run(exe,&["install",path(&manifest),&sha,path(&installed)]));
    assert_eq!(receipt["installed"].as_array().unwrap().len(),12);
    assert_eq!(receipt["qualified"],false);
    let backup=Path::new(receipt["backup_root"].as_str().unwrap());
    assert_eq!(fs::read(backup.join("previous").join(SKILLS[0]).join("local-user-data.md")).unwrap(),b"Keep this original user data");
    for &name in SKILLS{
        assert!(fs::read_to_string(installed.join(name).join("SKILL.md")).unwrap().contains("Native runtime only"));
    }
    let installed_exe=installed.join(SKILLS[0]).join("runtime").join(if cfg!(windows){"z2o.exe"}else{"z2o"});
    let desc=success(run(&installed_exe,&["describe"]));
    assert_eq!(desc["system_python_required"],false);assert_eq!(desc["python_fallback"],false);
    let req=base.join("request.json");
    fs::write(&req,serde_json::to_vec(&json!({"schema_version":1,"operation":"deterministic_stage_checks.author_token","args":[["Smith","Jones"]]})).unwrap()).unwrap();
    assert_eq!(success(run(&installed_exe,&["invoke",path(&req)]))["result"],"Smith and Jones");
    let repeat=success(run(exe,&["install",path(&manifest),&sha,path(&installed)]));
    assert!(repeat["installed"].as_array().unwrap().is_empty());
    assert_eq!(repeat["skipped"].as_array().unwrap().len(),12);
    let untouched=fs::read(installed.join(SKILLS[0]).join("SKILL.md")).unwrap();
    fs::write(bundle.join("skills").join(SKILLS[0]).join("reference.py"),b"tampered").unwrap();
    assert!(!run(exe,&["install",path(&manifest),&sha,path(&installed)]).status.success());
    assert_eq!(fs::read(installed.join(SKILLS[0]).join("SKILL.md")).unwrap(),untouched);
    let bad_root=base.join("should-not-exist");
    assert!(!run(exe,&["install",path(&manifest),&"0".repeat(64),path(&bad_root)]).status.success());
    assert!(!bad_root.exists());
}
