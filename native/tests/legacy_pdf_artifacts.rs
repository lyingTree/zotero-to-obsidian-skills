use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use lopdf::{Document, Object, Stream, dictionary};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use z2o_native::{check_ledger, files};

fn pdf(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages = doc.new_object_id();
    let content = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
    let page = doc.add_object(dictionary! {"Type"=>"Page","Parent"=>pages,"Contents"=>content,"Resources"=>dictionary! {}});
    doc.objects.insert(pages, Object::Dictionary(dictionary! {
        "Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,"MediaBox"=>vec![0.into(),0.into(),100.into(),80.into()]
    }));
    let root = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root",root); doc.save(path).unwrap();
}

fn png(path: &Path, bytes: &[u8], color: ExtendedColorType, w: u32, h: u32) {
    PngEncoder::new(fs::File::create(path).unwrap()).write_image(bytes,w,h,color).unwrap();
}

#[test]
fn legacy_pdf_provenance_cli_checks_png_samples_without_certifying_the_pdf() {
    let tmp=tempfile::tempdir().unwrap();
    let root=tmp.path();
    let src=root.join("原始 source.pdf");
    let artifact=root.join("旧页图 page.png");
    let page_text=root.join("page.txt");
    let inventory=root.join("inventory.json");
    let request=root.join("request.json");
    let w=100u32;let h=80u32;
    pdf(&src);
    fs::write(&page_text,b"").unwrap();
    let white=vec![255;(w*h*3) as usize];
    let white_digest=files::digest(&white);
    png(&artifact,&white,ExtendedColorType::Rgb8,w,h);
    let mut render=check_ledger::bind_file(&artifact).unwrap();
    // Unlike the native PdfCraft format, historic records have no renderer
    // revision or pixel-format field, and relative artifact paths are valid.
    render["path"]=json!(artifact.file_name().unwrap().to_str().unwrap());
    render["dpi"]=json!(72);
    render["width"]=json!(w);render["height"]=json!(h);
    render["pixel_sha256"]=json!(white_digest);
    let mut source=check_ledger::bind_file(&src).unwrap();
    source["id"]=json!("main");source["role"]=json!("main");
    source["page_count"]=json!(1);
    source["pages"]=json!([{"page":1,"render":render,
        "text":check_ledger::bind_file(&page_text).unwrap(),"blocks":[]}]);

    fs::write(&request,serde_json::to_vec(&json!({"schema_version":1,
        "operation":"validate_delivery.check_source_provenance","args":[inventory]})).unwrap()).unwrap();
    let exe=std::env::var_os("Z2O_LEGACY_PDF_TEST_CLI").unwrap_or_else(||env!("CARGO_BIN_EXE_z2o").into());
    let inspect=|r:&Value,expected:Option<&str>|{
        let mut current=source.clone();
        current["pages"][0]["render"]=r.clone();
        fs::write(&inventory,serde_json::to_vec(&json!({"schema_version":1,"sources":[current]})).unwrap()).unwrap();
        let before=fs::read(&inventory).unwrap();
        let out=Command::new(&exe).arg("invoke").arg(&request).env("PATH","").env_remove("PYTHONPATH").output().unwrap();
        assert!(out.status.success(),"{}",String::from_utf8_lossy(&out.stderr));
        assert_eq!(fs::read(&inventory).unwrap(),before,"provenance inspection is read-only");
        let result=serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"].clone();
        assert_eq!(result["source_checks_complete"],false);
        assert_eq!(result["delivery_ready"],false);
        assert_eq!(result["source_fidelity"],"not_assessed");
        let findings=result["findings"].as_array().unwrap();
        assert!(findings.iter().any(|f|f["code"]=="PDF_FULL_PROVENANCE_UNMIGRATED"),"{result}");
        let ordinary:Vec<_>=findings.iter().filter(|f|f["code"]!="PDF_FULL_PROVENANCE_UNMIGRATED").collect();
        if let Some(code)=expected {
            assert_eq!(ordinary.len(),1,"{result}");
            assert_eq!(ordinary[0]["code"],code,"{result}");
            assert_eq!(ordinary[0]["object_id"],"main:page-1:render");
            assert_eq!(result["legacy_pdf_render_artifacts_checked"],0);
        } else {
            assert!(ordinary.is_empty(),"{result}");
            assert_eq!(result["legacy_pdf_render_artifacts_checked"],1);
        }
        result
    };
    let mut altered=white.clone();altered[0]=254;
    png(&artifact,&altered,ExtendedColorType::Rgb8,w,h);
    // Rehashing substituted encoded bytes must not bypass the inner digest.
    let mut rebound=render.clone();
    rebound["sha256"]=json!(files::digest(&fs::read(&artifact).unwrap()));
    assert_ne!(rebound["sha256"],render["sha256"]);
    inspect(&rebound,Some("RENDER_PIXEL_HASH_MISMATCH"));
    // With both digests rewritten this is mechanically self-consistent but
    // still not evidence that the image actually came from the PDF.
    rebound["pixel_sha256"]=json!(files::digest(&altered));
    inspect(&rebound,None);

    png(&artifact,&white,ExtendedColorType::Rgb8,w,h);
    inspect(&render,None);
    for (field,value,code) in [
        ("dpi",json!(71),"RENDER_DPI"),("dpi",json!(601),"RENDER_DPI"),
        ("dpi",json!(144.0),"RENDER_DPI"),("width",json!(0),"RENDER_GEOMETRY"),
        ("height",json!(h-1),"RENDER_GEOMETRY"),("width",json!(20_001),"RENDER_GEOMETRY"),
        ("pixel_sha256",json!(null),"RENDER_PIXEL_HASH_REQUIRED"),
    ] {
        let mut record=render.clone(); record[field]=value;
        inspect(&record,Some(code));
    }


    for (color,components) in [(ExtendedColorType::Rgba8,4),(ExtendedColorType::L8,1)] {
        png(&artifact,&vec![255;(w*h*components) as usize],color,w,h);
        rebound["sha256"]=json!(files::digest(&fs::read(&artifact).unwrap()));
        inspect(&rebound,Some("RENDER_FORMAT"));
    }
    fs::write(&artifact,b"this is not a PNG").unwrap();
    rebound["sha256"]=json!(files::digest(&fs::read(&artifact).unwrap()));
    inspect(&rebound,Some("RENDER_READ"));
    fs::OpenOptions::new().write(true).open(&artifact).unwrap().set_len(128*1024*1024+1).unwrap();
    let report=inspect(&rebound,Some("FILE_READ"));
    assert!(report["findings"].as_array().unwrap().iter().any(|f|
        f["message"].as_str().unwrap_or("").contains("encoded byte budget")));
}
