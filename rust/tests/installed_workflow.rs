//! Exercise the actual packaged executable through the public CLI, without PATH.
use lopdf::{dictionary, Document, Object, Stream};
use serde_json::{json, Value};
use std::{env, fs, io::Write, path::Path, process::{Command, Output}};
use z2o_native::files;

fn digest(path: &Path) -> String {
    files::digest(&fs::read(path).unwrap())
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn cli(exe: &Path, args: &[&str]) -> Output {
    Command::new(exe).args(args).env("PATH", "").output().unwrap()
}

fn passed(output: Output) -> Value {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn request(file: &Path, exe: &Path, operation: &str, args: Value, kwargs: Value) -> Output {
    fs::write(file, serde_json::to_vec(&json!({
        "schema_version": 1, "operation": operation, "args": args, "kwargs": kwargs
    })).unwrap()).unwrap();
    cli(exe, &["invoke", file.to_str().unwrap()])
}

fn pdf(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages = doc.new_object_id();
    let font = doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let content = doc.add_object(Stream::new(dictionary! {},
        b"BT /F1 12 Tf 1 0 0 1 10 85 Tm (Packaged PDF source) Tj ET".to_vec()));
    let page = doc.add_object(dictionary! {
        "Type"=>"Page", "Parent"=>pages, "Contents"=>content,
        "Resources"=>dictionary! {"Font"=>dictionary! {"F1"=>font}}
    });
    doc.objects.insert(pages, Object::Dictionary(dictionary! {
        "Type"=>"Pages", "Kids"=>vec![Object::Reference(page)], "Count"=>1,
        "MediaBox"=>vec![0.into(),0.into(),200.into(),100.into()]
    }));
    let catalog = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", catalog);
    doc.save(path).unwrap();
}

fn docx(path: &Path) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>补充材料 Packaged DOCX source</w:t></w:r></w:p></w:body></w:document>"#;
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rDoc" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    for (name, text) in [("[Content_Types].xml", types), ("_rels/.rels", rels), ("word/document.xml", xml)] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

// Run after packaging/installing the real checkout with the release-profile binary.
// The Cargo harness creates fixtures; none of the installed CLI child processes
// receives legacy, Git, Rust or other executables on PATH.
#[test]
#[ignore = "requires the exact packaged native candidate installation from CI"]
fn installed_mixed_source_workflow_without_path() {
    let skills = std::path::PathBuf::from(env::var_os("Z2O_INSTALLED_SKILLS").expect("installed skills root"));
    let manifest = std::path::PathBuf::from(env::var_os("Z2O_INSTALL_MANIFEST").expect("bundle manifest"));
    let install_receipt = std::path::PathBuf::from(env::var_os("Z2O_INSTALL_RECEIPT").expect("installation receipt"));
    let receipt_out = std::path::PathBuf::from(env::var_os("Z2O_WORKFLOW_RECEIPT").expect("workflow receipt output"));
    let package = read(&manifest);
    let install = read(&install_receipt);
    let manifest_digest = digest(&manifest);
    assert_eq!(install["package_manifest"]["sha256"], manifest_digest);
    let name = package["runtime"].as_str().unwrap();
    let exe = skills.join(name);
    let binary_digest = digest(&exe);
    assert_eq!(package["files"][name]["sha256"], binary_digest);
    let verified = passed(cli(&exe, &["verify-install", manifest.to_str().unwrap(),
        &manifest_digest, skills.to_str().unwrap()]));
    assert_eq!(verified["qualified"], false);
    assert_eq!(verified["runtime_sha256"], binary_digest);
    let desc = passed(cli(&exe, &["describe"]));
    assert_eq!(desc["engine"], "rust");
    assert_eq!(desc["version"], format!("v{}", z2o_native::ENGINE_VERSION));

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("验收 中文 work");
    fs::create_dir(&path).unwrap();
    let main = path.join("正文 PDF.pdf");
    let si = path.join("补充 DOCX.docx");
    let candidate = path.join("候选 note.md");
    let resources = path.join("引用 resources");
    fs::create_dir(&resources).unwrap();
    pdf(&main);
    docx(&si);
    fs::write(&candidate, "## Packaged candidate\r\n\r\n保留内容。\r\n").unwrap();
    let original = [digest(&main), digest(&si), digest(&candidate)];
    let req = path.join("请求 request.json");

    let run = passed(request(&req, &exe, "complete_workflow.initialize_workflow", json!([]), json!({
        "skills_root": skills, "work_root": path.join("证据 workflow"),
        "sources": {"main": main, "si": si}, "source_roles": {"main": "main", "si": "supplement"},
        "candidate_path": candidate, "resource_root": resources, "translation_required": false,
        "run_id": "installed-mixed-sources-smoke"
    })));
    let run_path = Path::new(run["result"].as_str().unwrap());
    let state = read(run_path);
    let source_path = Path::new(state["records"]["source_inventory"].as_str().unwrap());
    let evidence = read(source_path);
    let sources = evidence["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 2);
    let p = sources.iter().find(|s| s["id"] == "main").unwrap();
    let d = sources.iter().find(|s| s["id"] == "si").unwrap();
    assert_eq!(p["role"], "main");
    assert_eq!(p["sha256"], original[0]);
    assert!(p["pages"][0]["blocks"].as_array().unwrap().iter()
        .any(|b| b["text"].as_str().unwrap().contains("Packaged PDF source")));
    assert_eq!(d["role"], "supplement");
    assert_eq!(d["sha256"], original[1]);
    assert_eq!(d["native_blocks"][0]["text"], "补充材料 Packaged DOCX source");

    let pending = passed(request(&req, &exe, "complete_workflow.inspect_workflow",
        json!([run_path]), json!({})));
    assert_eq!(pending["result"]["records_ready"], false);
    assert_eq!(pending["result"]["source_fidelity"], "not_assessed");
    assert!(!request(&req, &exe, "complete_workflow.require_completed_records",
        json!([run_path]), json!({})).status.success());

    let mechanical = passed(request(&req, &exe, "note_pipeline.process_markdown",
        json!([candidate, path.join("离线 Markdown outputs")]), json!({})));
    let r = &mechanical["result"];
    assert_eq!(r["source_fidelity"], "not_assessed");
    assert_eq!(r["status"], "needs_attention"); // No external evidence contract supplied.
    let normalized = &r["outputs"]["normalized.md"];
    assert_eq!(digest(Path::new(normalized["path"].as_str().unwrap())), normalized["sha256"]);
    assert_eq!([digest(&main), digest(&si), digest(&candidate)], original);
    assert_eq!(digest(&exe), binary_digest);
    assert_eq!(digest(&manifest), manifest_digest);

    let receipt = json!({
        "schema_version": 1, "boundary": "Installed artifact smoke only; not complete acceptance or clean host qualification.",
        "checkout": env::var("GITHUB_SHA").ok(), "host_os": env::consts::OS, "host_arch": env::consts::ARCH,
        "package_manifest_sha256": manifest_digest, "installed_runtime_sha256": binary_digest,
        "installed_payload_verified": true, "installed_file_count": verified["file_count"],
        "installed_workflow_run_sha256": digest(run_path), "source_inventory_sha256": digest(source_path),
        "main_pdf_sha256": original[0], "supplement_docx_sha256": original[1], "markdown_input_sha256": original[2],
        "normalized_markdown_sha256": normalized["sha256"], "mechanical_status": r["status"],
        "missing_completion_evidence_rejected": true, "source_fidelity": "not_assessed"
    });
    let mut out = fs::OpenOptions::new().create_new(true).write(true).open(receipt_out).unwrap();
    serde_json::to_writer_pretty(&mut out, &receipt).unwrap();
    writeln!(out).unwrap();
}
