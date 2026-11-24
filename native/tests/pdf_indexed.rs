use lopdf::{dictionary,Dictionary,Document,Object,ObjectId,Stream,StringFormat};
use serde_json::{json,Value};
use std::{fs,path::Path};
use z2o_native::{files,invoke,pdf_inventory,pipeline,source_prepare};

fn stream(doc:&mut Document,dict:Dictionary,bytes:Vec<u8>)->ObjectId{
    doc.add_object(Stream::new(dict,bytes))
}
fn indexed(base:&str,hival:i64,lookup:Object)->Object{
    Object::Array(vec![Object::Name(b"Indexed".to_vec()),Object::Name(base.as_bytes().to_vec()),hival.into(),lookup])
}
fn image(width:i32,height:i32,bits:i32,space:Object)->Dictionary{
    dictionary!{"Type"=>"XObject","Subtype"=>"Image","Width"=>width,"Height"=>height,"BitsPerComponent"=>bits,"ColorSpace"=>space}
}
fn write_pdf(mut doc:Document,path:&Path,objects:&[(&str,ObjectId)],ops:&str){
    let pages=doc.new_object_id();
    let mut resources=Dictionary::new();
    for (key,id) in objects{resources.set(*key,Object::Reference(*id));}
    let contents=stream(&mut doc,Dictionary::new(),ops.as_bytes().to_vec());
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Contents"=>contents,"Resources"=>dictionary!{"XObject"=>resources}});
    doc.objects.insert(pages,Object::Dictionary(dictionary!{"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1,
        "MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()]}));
    let catalog=doc.add_object(dictionary!{"Type"=>"Catalog","Pages"=>pages});doc.trailer.set("Root",catalog);doc.save(path).unwrap();
}
fn export(pdf:&Path,xref:u32,output:&Path)->anyhow::Result<Value>{
    Ok(invoke(json!({"schema_version":1,"operation":"pdf_image_objects.export_image_object",
        "args":[pdf],"kwargs":{"page_number":1,"image_xref":xref,"destination":output}}))?["result"].clone())
}

#[test]
fn lossless_indexed_flate_exports_exact_packed_rgb_and_provenance(){
    let temp=tempfile::tempdir().unwrap();let root=temp.path();let pdf=root.join("packed-indexed.pdf");
    let mut doc=Document::with_version("1.5");
    let rgb=vec![10,20,30,40,50,60,70,80,90];
    let compressed_palette=craft_pdf::filters::encode_flate(&rgb);
    let palette=stream(&mut doc,dictionary!{"Filter"=>"FlateDecode"},compressed_palette.clone());
    let color_space=doc.add_object(indexed("DeviceRGB",2,Object::Reference(palette)));
    let mut d=image(3,2,2,Object::Reference(color_space));d.set("Filter","FlateDecode");
    // Rows represent [0, 1, 2] and [3, 2, 1]; nonzero padding is ignored.
    let raw=[0b00011011,0b11100111];
    let obj=stream(&mut doc,d,craft_pdf::filters::encode_flate(&raw));
    write_pdf(doc,&pdf,&[("Image",obj)],"q 30 0 0 20 10 10 cm /Image Do Q");
    let original=fs::read(&pdf).unwrap();

    let png=root.join("rgb.png");let record=export(&pdf,obj.0,&png).unwrap();
    let actual=image::open(&png).unwrap().into_rgb8();
    // Index 3 clips to 2, while index 2 still selects entry 2; mapping the
    // entire sample domain to hival would incorrectly make index 2 select 1.
    let expected=[10,20,30,40,50,60,70,80,90,70,80,90,70,80,90,40,50,60];
    assert_eq!(actual.dimensions(),(3,2));assert_eq!(actual.as_raw(),&expected);
    let object=&record["source_object"];
    assert_eq!(record["source_fidelity"],"not_assessed");
    assert_eq!(record["visual_review"]["status"],"pending");
    assert_eq!(object["pdf_colorspace"],"/Indexed");
    assert_eq!(object["bits_per_component"],2);
    assert_eq!(object["decoded_samples_sha256"],files::digest(&expected));
    assert_eq!(object["indexed_palette"],json!({"base":"/DeviceRGB","hival":2,"decode":[0.,3.],
        "lookup_format":"stream","lookup_sha256":files::digest(&rgb),"encoded_lookup_stream_sha256":files::digest(&compressed_palette)}));

    let inventory=source_prepare::prepare_sources(&json!({"main":pdf}),&root.join("inventory"),&json!({"main":"main"}),
        None,72,None,|_,_,_|{}).unwrap();
    let data=pipeline::read_json(&inventory).unwrap();
    let source=&data["sources"][0];
    assert_eq!(source["image_objects"][0]["extraction"]["source_object"],*object);
    assert_eq!(pdf_inventory::replay(source,&pdf,|_,_,_|{}).unwrap()["same_backend_replay_complete"],true);
    let mut changed=source.clone();
    changed["image_objects"][0]["extraction"]["source_object"]["indexed_palette"]["lookup_sha256"]=json!("0".repeat(64));
    assert!(pdf_inventory::replay(&changed,&pdf,|_,_,_|{}).is_err());
    assert_eq!(fs::read(&pdf).unwrap(),original);
}

#[test]
fn indexed_gray_decode_rounds_and_applies_independent_soft_mask(){
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();let pdf=root.join("gray-indexed.pdf");
    let mut doc=Document::with_version("1.5");
    let mask=stream(&mut doc,image(4,1,8,Object::Name(b"DeviceGray".to_vec())),vec![255,192,127,0]);
    let mut d=image(4,1,2,indexed("DeviceGray",2,Object::String(vec![10,100,200],StringFormat::Hexadecimal)));
    // Affine Decode yields 2.5, 1.5, 0.5 and -0.5 from samples 0-3.
    // Round to nearest then clip to 0-2: indices 2, 2, 1, 0.
    d.set("Decode",vec![Object::Real(2.5),Object::Real(-0.5)]);d.set("SMask",Object::Reference(mask));
    let obj=stream(&mut doc,d,vec![0b00011011]);
    write_pdf(doc,&pdf,&[("Gray",obj)],"q 40 0 0 10 10 10 cm /Gray Do Q");
    let png=root.join("gray.png");let report=export(&pdf,obj.0,&png).unwrap();
    assert_eq!(report["method"],"smask_composited_png");
    assert_eq!(report["source_object"]["indexed_palette"]["lookup_format"],"string");
    assert_eq!(report["source_object"]["smask"]["image_xref"],mask.0);
    assert_eq!(image::open(&png).unwrap().into_luma_alpha8().as_raw(),&[200,255,200,192,100,127,10,0]);
}

#[test]
fn malformed_indexed_palettes_and_unsupported_display_never_publish_an_asset(){
    let tmp=tempfile::tempdir().unwrap();let root=tmp.path();let pdf=root.join("bad-indexed.pdf");
    let mut doc=Document::with_version("1.5");
    let table=||Object::String(vec![10,20,30,40,50,60],StringFormat::Hexadecimal);
    let short=indexed("DeviceRGB",2,table());
    let cmyk=indexed("DeviceCMYK",1,table());
    let oversized=indexed("DeviceRGB",256,table());
    let mut d=image(1,1,8,indexed("DeviceRGB",1,table()));d.set("Filter","DCTDecode");
    let jpeg=stream(&mut doc,d,vec![0]);
    let mut d=image(1,1,8,indexed("DeviceRGB",1,table()));d.set("Decode",vec![0.into(),1.into(),2.into()]);
    let decode=stream(&mut doc,d,vec![0]);
    let bad_mask=stream(&mut doc,image(1,1,8,indexed("DeviceGray",1,Object::String(vec![100,200],StringFormat::Hexadecimal))),vec![0]);
    let mut d=image(1,1,8,indexed("DeviceRGB",1,table()));d.set("SMask",Object::Reference(bad_mask));
    let mask=stream(&mut doc,d,vec![0]);
    let objects=[
        ("Short",stream(&mut doc,image(1,1,8,short),vec![0])),
        ("Cmyk",stream(&mut doc,image(1,1,8,cmyk),vec![0])),
        ("Oversized",stream(&mut doc,image(1,1,8,oversized),vec![0])),
        ("Jpeg",jpeg),("Decode",decode),("Mask",mask),
    ];
    write_pdf(doc,&pdf,&objects,"q 1 0 0 1 1 1 cm /Short Do /Cmyk Do /Oversized Do /Jpeg Do /Decode Do /Mask Do Q");
    for ((name,id),reason) in objects.into_iter().zip(["lookup length","Indexed base","Indexed hival","with DCT","Indexed Decode","non-gray SMask"]){
        let out=root.join(format!("{name}.png"));let error=export(&pdf,id.0,&out).unwrap_err();
        assert!(format!("{error:#}").contains(reason),"{name}: {error:#}");assert!(!out.exists());
    }
}

fn predicted_params(predictor: i32, width: i32, colors: i32, bits: i32) -> Dictionary {
    dictionary! {
        "Predictor" => predictor, "Columns" => width,
        "Colors" => colors, "BitsPerComponent" => bits
    }
}

#[test]
fn flate_png_predictor_expands_packed_indexed_rows_and_packed_soft_mask() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let pdf = root.join("png-predictor-indexed-with-mask.pdf");
    let mut doc = Document::with_version("1.5");
    let mut mask = image(5, 5, 4, Object::Name(b"DeviceGray".to_vec()));
    mask.set("Filter", "FlateDecode");
    mask.set("DecodeParms", predicted_params(12, 5, 1, 4));
    // Five Up-filtered rows. Decoded nibbles, excluding padding:
    // 01234, 56789, ABCDE, F4321, 079BF.
    let mask_rows = [
        2, 0x01, 0x23, 0x40,
        2, 0x55, 0x55, 0x50,
        2, 0x55, 0x55, 0x50,
        2, 0x49, 0x65, 0x30,
        2, 0x13, 0x69, 0xE0,
    ];
    let mask = stream(&mut doc, mask, craft_pdf::filters::encode_flate(&mask_rows));

    let rgb = [8, 16, 24, 40, 64, 88, 128, 152, 176, 208, 232, 255];
    let mut d = image(5, 5, 2, indexed("DeviceRGB", 3,
        Object::String(rgb.to_vec(), StringFormat::Hexadecimal)));
    d.set("Filter", vec![Object::Name(b"ASCIIHexDecode".to_vec()), Object::Name(b"FlateDecode".to_vec())]);
    d.set("DecodeParms", vec![Object::Null, Object::Dictionary(predicted_params(15, 5, 1, 2))]);
    d.set("SMask", Object::Reference(mask));
    // Sub, Up, Paeth, Average, None. After undoing byte-level prediction,
    // packed rows are [1b 40], [e4 80], [6c 00], [ff 40], [01 c0].
    let rows = [
        1, 0x1b, 0x25,
        2, 0xc9, 0x40,
        4, 0x88, 0x94,
        3, 0xc9, 0xc1,
        0, 0x01, 0xc0,
    ];
    let wrapped = craft_pdf::filters::encode_flate(&rows).iter()
        .map(|b| format!("{b:02X}")).collect::<String>() + ">";
    let id = stream(&mut doc, d, wrapped.into_bytes());
    write_pdf(doc, &pdf, &[("Packed", id)], "q 50 0 0 50 10 10 cm /Packed Do Q");

    let out = root.join("combined.png");
    let report = export(&pdf, id.0, &out).unwrap();
    let actual = image::open(out).unwrap().into_rgba8();
    assert_eq!(actual.dimensions(), (5, 5));
    let indices = [0,1,2,3,1, 3,2,1,0,2, 1,2,3,0,0, 3,3,3,3,1, 0,0,0,1,3];
    let alpha = [0,17,34,51,68, 85,102,119,136,153, 170,187,204,221,238,
                 255,68,51,34,17, 0,119,153,187,255];
    let expected = indices.iter().zip(alpha).flat_map(|(&i, a)| {
        [rgb[i*3], rgb[i*3+1], rgb[i*3+2], a]
    }).collect::<Vec<_>>();
    assert_eq!(actual.as_raw(), &expected);
    assert_eq!(report["source_object"]["smask"]["decoded_alpha_sha256"], files::digest(&alpha));
    assert_eq!(report["source_object"]["smask"]["status"], "nonopaque");
    assert_eq!(report["source_object"]["filters"], json!(["ASCIIHexDecode", "FlateDecode"]));
    assert_eq!(report["method"], "smask_composited_png");
    assert_eq!(report["visual_review"]["status"], "pending");
    assert_eq!(report["source_fidelity"], "not_assessed");
}

#[test]
fn lzw_png_predictor_decodes_packed_rgb_with_a_two_byte_left_neighbour() {
    let temp = tempfile::tempdir().unwrap();
    let pdf = temp.path().join("packed-rgb-lzw.pdf");
    let mut doc = Document::with_version("1.5");
    let mut d = image(3, 2, 4, Object::Name(b"DeviceRGB".to_vec()));
    d.set("Filter", "LZWDecode");
    let mut p = predicted_params(15, 3, 3, 4);
    p.set("EarlyChange", 0);
    d.set("DecodeParms", p);
    // Independent sequence of 9-bit PDF LZW codes: clear, literal bytes,
    // end. The PNG Sub row uses a two-byte lookback (3 x 4 bits), then Up.
    let codes = [256u16, 1, 0x12, 0x34, 0x44, 0x44, 0x3a,
                 2, 0x86, 0x42, 0xfe, 0xba, 0x80, 257];
    let mut packed = vec![0u8; (codes.len() * 9).div_ceil(8)];
    for (i, code) in codes.into_iter().enumerate() {
        for shift in 0..9 {
            let k = i * 9 + shift;
            packed[k / 8] |= ((code >> (8 - shift)) as u8 & 1) << (7 - k % 8);
        }
    }
    let id = stream(&mut doc, d, packed);
    write_pdf(doc, &pdf, &[("RGB", id)], "q 30 0 0 20 10 10 cm /RGB Do Q");
    let out = temp.path().join("rgb.png");
    let report = export(&pdf, id.0, &out).unwrap();
    let actual = image::open(out).unwrap().into_rgb8();
    assert_eq!(actual.dimensions(), (3, 2));
    let expected = [17,34,51,68,85,102,119,136,153, 153,136,119,102,85,68,51,34,17];
    assert_eq!(actual.as_raw(), &expected);
    assert_eq!(report["source_object"]["decoded_samples_sha256"], files::digest(&expected));
    assert_eq!(report["method"], "lossless_sample_png");
}

#[test]
fn malformed_predictor_layouts_and_rows_do_not_replace_existing_output() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let pdf = root.join("bad-row-metadata.pdf");
    let mut doc = Document::with_version("1.5");
    let declared = || predicted_params(15, 5, 1, 2);
    let parent_dict = || image(5, 2, 2, Object::Name(b"DeviceGray".to_vec()));
    let valid = [0, 0x1b, 0x40, 0, 0xe4, 0x80];
    let mut cases = Vec::new();
    for (name, p, bytes, reason) in [
        // Different packed widths can fit the same downstream
        // byte count in other cases. Here Width 5 and Columns 6 both use two.
        ("Columns", predicted_params(15, 6, 1, 2), valid.as_slice(), "predictor layout"),
        ("Colors", predicted_params(15, 5, 3, 2), valid.as_slice(), "predictor layout"),
        ("Bits", predicted_params(15, 5, 1, 4), valid.as_slice(), "predictor layout"),
        ("MissingTag", declared(), &[0x1b, 0x40, 0xe4, 0x80], "row byte count"),
        ("Short", declared(), &valid[..5], "row byte count"),
        // A stream-level predictor drops a dangling tag without producing
        // another sample; checking only the final sample count misses it.
        ("DanglingTag", declared(), &[0, 0x1b, 0x40, 0, 0xe4, 0x80, 0], "image byte budget"),
        ("InvalidTag", declared(), &[0, 0x1b, 0x40, 5, 0xe4, 0x80], "invalid image predictor row"),
        ("PackedTiff", predicted_params(2, 5, 1, 2), &valid[..4], "packed or Indexed TIFF"),
    ] {
        let mut d = parent_dict();
        d.set("Filter", "FlateDecode");
        d.set("DecodeParms", p);
        cases.push((name, stream(&mut doc, d, craft_pdf::filters::encode_flate(bytes)), reason));
    }
    let mut d = parent_dict();
    d.set("Filter", vec![Object::Name(b"FlateDecode".to_vec()), Object::Name(b"ASCIIHexDecode".to_vec())]);
    d.set("DecodeParms", vec![Object::Dictionary(declared()), Object::Null]);
    cases.push(("NotFinal", stream(&mut doc, d, vec![0]), "final Flate or LZW"));
    let mut d = parent_dict();
    d.set("Filter", "ASCIIHexDecode");
    d.set("DecodeParms", declared());
    cases.push(("WrongFilter", stream(&mut doc, d, vec![0]), "final Flate or LZW"));

    let objects = cases.iter().map(|(n, id, _)| (*n, *id)).collect::<Vec<_>>();
    let ops = objects.iter().map(|(name, _)| format!("/{name} Do ")).collect::<String>();
    write_pdf(doc, &pdf, &objects, &format!("q 50 0 0 20 10 10 cm {ops}Q"));
    let out = root.join("must-not-change.png");
    fs::write(&out, b"pre-existing output").unwrap();
    for (name, id, reason) in cases {
        let err = format!("{:#}", export(&pdf, id.0, &out).unwrap_err());
        assert!(err.contains("PAGE_RENDER_REQUIRED") && err.contains(reason), "{name}: {err}");
        assert_eq!(fs::read(&out).unwrap(), b"pre-existing output");
    }
}
