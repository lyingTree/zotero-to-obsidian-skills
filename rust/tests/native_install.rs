use serde_json::{json,Value};
use std::{fs,path::Path,process::{Command,Output}};
use z2o_native::{native_install::SKILLS,files,ENGINE_VERSION};
fn run(exe:&Path,args:&[&str])->Output{Command::new(exe).args(args).env("PATH","").output().unwrap()}
fn success(output:Output)->Value{
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}
fn path(p:&Path)->&str{p.to_str().unwrap()}
#[test]
fn actual_offline_install_reuse_backup_and_tamper_protection(){
    let root=tempfile::tempdir().unwrap();let base=root.path();let source=base.join("source");
    fs::create_dir(&source).unwrap();
    for &name in SKILLS{
        let p=source.join(name);fs::create_dir(&p).unwrap();
        fs::write(p.join("SKILL.md"),format!("---\nname: {name}\nversion: v{ENGINE_VERSION}\ndescription: test\n---\n\n# Actual source skill\n")).unwrap();
        fs::write(p.join("reference.py"),b"raise RuntimeError('must never execute')\n").unwrap();
        fs::create_dir(p.join("nested")).unwrap();
        fs::write(p.join("nested").join("reference.PYW"),b"legacy").unwrap();
        fs::write(p.join("nested").join("reference.pyi"),b"legacy stub").unwrap();
        fs::write(p.join("nested").join("reference.pyz"),b"legacy archive").unwrap();
        fs::write(p.join("nested").join("reference.PYD"),b"legacy extension").unwrap();
        fs::create_dir(p.join("__PYCACHE__")).unwrap();
        fs::write(p.join("__PYCACHE__").join("reference.PYC"),b"legacy cache").unwrap();
        fs::write(p.join("nested").join("reference.md"),b"Keep documentation").unwrap();
    }
    let exe=Path::new(env!("CARGO_BIN_EXE_z2o"));
    let bundle=base.join("bundle");let result=success(run(exe,&["package",path(&source),path(&bundle)]));
    assert_eq!(result["skill_count"],12);
    let manifest=bundle.join("package.json");
    let sha=files::snapshot(&manifest).unwrap()["sha256"].as_str().unwrap().to_owned();
    assert_eq!(result["package_manifest"]["sha256"],sha);
    let manifest_data:Value=serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(manifest_data["script_files_included"],false);
    for &name in SKILLS{
        let skill=bundle.join("skills").join(name);
        assert!(!skill.join("reference.py").exists());
        assert!(!skill.join("nested").join("reference.PYW").exists());
        assert!(!skill.join("nested").join("reference.pyi").exists());
        assert!(!skill.join("nested").join("reference.pyz").exists());
        assert!(!skill.join("nested").join("reference.PYD").exists());
        assert!(!skill.join("__PYCACHE__").exists());
        assert_eq!(fs::read(skill.join("nested").join("reference.md")).unwrap(),b"Keep documentation");
        assert!(source.join(name).join("reference.py").is_file());
    }
    let installed=base.join("skills");fs::create_dir(&installed).unwrap();
    let old=installed.join(SKILLS[0]);fs::create_dir(&old).unwrap();
    fs::write(old.join("SKILL.md"),format!("---\nname: {}\nversion: v{ENGINE_VERSION}\ndescription: legacy\n---\n",SKILLS[0])).unwrap();
    fs::write(old.join("local-user-data.md"),b"Keep this original user data").unwrap();
    fs::write(old.join("reference.py"),b"Legacy user installation").unwrap();
    let receipt=success(run(exe,&["install",path(&manifest),&sha,path(&installed)]));
    assert_eq!(receipt["installed"].as_array().unwrap().len(),12);
    assert_eq!(receipt["qualified"],true);
    let backup=Path::new(receipt["backup_root"].as_str().unwrap());
    assert_eq!(fs::read(backup.join("previous").join(SKILLS[0]).join("local-user-data.md")).unwrap(),b"Keep this original user data");
    assert_eq!(fs::read(backup.join("previous").join(SKILLS[0]).join("reference.py")).unwrap(),b"Legacy user installation");
    for &name in SKILLS{
        assert!(fs::read_to_string(installed.join(name).join("SKILL.md")).unwrap().contains("Rust runtime only"));
        assert!(!installed.join(name).join("reference.py").exists());
        assert!(!installed.join(name).join("nested").join("reference.PYW").exists());
    }
    let installed_exe=installed.join(SKILLS[0]).join("runtime").join(if cfg!(windows){"z2o.exe"}else{"z2o"});
    let desc=success(run(&installed_exe,&["describe"]));
    assert_eq!(desc["engine"],"rust");assert_eq!(desc["version"],format!("v{ENGINE_VERSION}"));
    let req=base.join("request.json");
    fs::write(&req,serde_json::to_vec(&json!({"schema_version":1,"operation":"deterministic_stage_checks.author_token","args":[["Smith","Jones"]]})).unwrap()).unwrap();
    assert_eq!(success(run(&installed_exe,&["invoke",path(&req)]))["result"],"Smith and Jones");
    let verified=success(run(&installed_exe,&["verify-install",path(&manifest),&sha,path(&installed)]));
    assert_eq!(verified["status"],"installed_candidate_files_verified");
    assert_eq!(verified["package_manifest"]["sha256"],sha);
    assert_eq!(verified["runtime_sha256"],manifest_data["files"][manifest_data["runtime"].as_str().unwrap()]["sha256"]);
    assert_eq!(verified["file_count"],manifest_data["files"].as_object().unwrap().len());
    assert_eq!(verified["qualified"],true);
    let from_uninstalled_exe=run(exe,&["verify-install",path(&manifest),&sha,path(&installed)]);
    assert!(!from_uninstalled_exe.status.success());
    assert!(String::from_utf8_lossy(&from_uninstalled_exe.stderr).contains("NATIVE_INSTALLED_RUNTIME_PATH_MISMATCH"));
    let resource=installed.join(SKILLS[1]).join("nested").join("reference.md");
    fs::write(&resource,b"Keep documentatioN").unwrap();
    let mismatch=run(&installed_exe,&["verify-install",path(&manifest),&sha,path(&installed)]);
    assert!(!mismatch.status.success());
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("NATIVE_INSTALLED_FILE_MISMATCH"));
    assert_eq!(fs::read_to_string(&resource).unwrap(),"Keep documentatioN");
    fs::write(&resource,b"Keep documentation").unwrap();
    let repeat=success(run(exe,&["install",path(&manifest),&sha,path(&installed)]));
    assert!(repeat["installed"].as_array().unwrap().is_empty());
    assert_eq!(repeat["skipped"].as_array().unwrap().len(),12);
    let untouched=fs::read(installed.join(SKILLS[0]).join("SKILL.md")).unwrap();
    fs::write(bundle.join("skills").join(SKILLS[0]).join("reference.py"),b"tampered").unwrap();
    assert!(!run(exe,&["install",path(&manifest),&sha,path(&installed)]).status.success());
    assert_eq!(fs::read(installed.join(SKILLS[0]).join("SKILL.md")).unwrap(),untouched);
    let moved=bundle.join("skills");let absent=bundle.join("unpacked-payload-moved");
    fs::rename(&moved,&absent).unwrap();
    let verify_op=json!({"schema_version":1,"operation":"native_runtime.verify_install","args":[manifest,sha,installed]});
    fs::write(&req,serde_json::to_vec(&verify_op).unwrap()).unwrap();
    let via_invoke=success(run(&installed_exe,&["invoke",path(&req)]));
    assert_eq!(via_invoke["result"],verified);
    fs::rename(&absent,&moved).unwrap();
    let injected=bundle.join("skills").join(SKILLS[0]).join("reference.py");
    let binding=files::snapshot(&injected).unwrap();
    let mut forged=manifest_data;
    forged["files"].as_object_mut().unwrap().insert(
        format!("{}/reference.py",SKILLS[0]),
        json!({"sha256":binding["sha256"],"size":binding["size"],"executable":false})
    );
    fs::write(&manifest,serde_json::to_vec(&forged).unwrap()).unwrap();
    let forged_sha=files::snapshot(&manifest).unwrap()["sha256"].as_str().unwrap().to_owned();
    let denied=base.join("denied-script-payload");
    let failure=run(exe,&["install",path(&manifest),&forged_sha,path(&denied)]);
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("BUNDLE_DISALLOWED_SCRIPT_FILE"));
    assert!(!denied.exists());
    let bad_root=base.join("should-not-exist");
    assert!(!run(exe,&["install",path(&manifest),&"0".repeat(64),path(&bad_root)]).status.success());
    assert!(!bad_root.exists());
}

#[test]
fn repository_skill_entrypoints_are_rust_only(){
    let root=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let repo_ver=fs::read_to_string(root.join("VERSION")).unwrap().trim().to_owned();
    assert_eq!(repo_ver,format!("v{ENGINE_VERSION}"));
    let guide=root.join("literature-zotero2obsidian").join("references").join("rust-entrypoints.md");
    assert!(guide.is_file());
    for &name in SKILLS{
        let skill=fs::read_to_string(root.join(name).join("SKILL.md")).unwrap();
        assert!(skill.contains(&format!("version: {repo_ver}")),"missing version {repo_ver} in {name}");
        assert!(skill.contains("rust-entrypoints.md"),"missing rust entry guide: {name}");
        assert!(!skill.contains(".py)"),"legacy script link: {name}");
    }
}
