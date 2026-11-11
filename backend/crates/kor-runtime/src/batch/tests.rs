use std::path::{Path, PathBuf};

use async_trait::async_trait;

use super::*;

fn files(d: &Path, names: &[(&str, &[u8])]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|(n, b)| {
            let p = d.join(n);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b).unwrap();
            p
        })
        .collect()
}

fn name(s: &str) -> String {
    s.to_owned()
}

#[test]
fn model_answers_are_parsed_fenced_or_bare() {
    let a = "Plan:\n```json\n[{\"op\":\"rename\",\"from\":\"a.jpg\",\"to\":\"2024-a.jpg\"}]\n```";
    assert_eq!(
        parse_ops(a).unwrap(),
        vec![Op::Rename {
            from: name("a.jpg"),
            to: name("2024-a.jpg")
        }]
    );
    assert!(parse_ops("yapamam").is_err());
    assert!(
        parse_ops("[{\"op\":\"delete\",\"file\":\"a\"}]").is_err(),
        "there is no delete"
    );
    let c = parse_ops("[{\"op\":\"csv_clean\",\"file\":\"x.csv\"}]").unwrap();
    assert!(matches!(
        &c[0],
        Op::CsvClean {
            trim: true,
            drop_empty: true,
            dedupe: false,
            ..
        }
    ));
}

#[test]
fn validation_numbers_clashes_and_keeps_targets_in_the_selection() {
    let d = tempfile::tempdir().unwrap();
    let sel = files(
        d.path(),
        &[("a.jpg", b"1"), ("b.jpg", b"2"), ("hedef.jpg", b"var")],
    );
    let other = d.path().join("baska.txt");
    std::fs::write(&other, "x").unwrap();
    let plan = validate(
        &sel[..2],
        vec![
            Op::Rename {
                from: name("a.jpg"),
                to: name("hedef.jpg"),
            },
            Op::Rename {
                from: name("b.jpg"),
                to: name("hedef.jpg"),
            },
            Op::Rename {
                from: name("baska.txt"),
                to: name("y.txt"),
            },
            Op::Rename {
                from: name("a.jpg"),
                to: name("../kacis.jpg"),
            },
            Op::Rename {
                from: sel[1].display().to_string(),
                to: name("alt/c.jpg"),
            },
            Op::ConvertImage {
                from: name("a.jpg"),
                to: name("a.exe"),
                width: None,
                height: None,
            },
        ],
    );
    let targets: Vec<PathBuf> = plan.steps.iter().map(|s| s.target.clone()).collect();
    assert_eq!(
        targets,
        vec![
            d.path().join("hedef (2).jpg"),
            d.path().join("hedef (3).jpg")
        ]
    );
    // `b.jpg` is already renamed by step 2; outside file, `..`, bad type are refused.
    assert_eq!(plan.warnings.len(), 4, "{:?}", plan.warnings);
    assert_eq!(plan.rows[0].op, "rename");
    assert!(plan.rows[0].from.ends_with("a.jpg") && plan.rows[0].to.ends_with("hedef (2).jpg"));
}

#[test]
fn apply_runs_every_operation_and_undo_reverts_the_batch() {
    let d = tempfile::tempdir().unwrap();
    let undo = d.path().join("undo");
    let dir = d.path().join("w");
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(40, 20)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    // PNG with a text chunk inserted after IHDR (8 sig + 25 IHDR).
    let mut tagged = png.get_ref()[..33].to_vec();
    let text = b"tEXtComment\0gizli";
    tagged.extend_from_slice(&u32::try_from(text.len() - 4).unwrap().to_be_bytes());
    tagged.extend_from_slice(text);
    tagged.extend_from_slice(&[0, 0, 0, 0]);
    tagged.extend_from_slice(&png.get_ref()[33..]);
    let latin = encoding_rs::WINDOWS_1254.encode("çığ şöyle").0.into_owned();
    let sel = files(
        &dir,
        &[
            ("foto 1.png", &tagged),
            ("not.txt", b"merhaba dunya, dunya"),
            ("eski.txt", &latin),
            ("veri.csv", b"ad; yas\n ali ;3\n\n ali ;3\nveli;\"4;5\"\n"),
            ("tasi.txt", b"tasinacak"),
        ],
    );
    let ops = vec![
        Op::ConvertImage {
            from: name("foto 1.png"),
            to: name("foto-1.jpg"),
            width: Some(20),
            height: None,
        },
        Op::StripMetadata {
            from: name("foto 1.png"),
            to: None,
        },
        Op::Replace {
            file: name("not.txt"),
            find: name("dunya"),
            replace: name("dünya"),
            regex: false,
        },
        Op::Reencode {
            file: name("eski.txt"),
            encoding: name("windows-1254"),
        },
        Op::CsvClean {
            file: name("veri.csv"),
            to: None,
            trim: true,
            drop_empty: true,
            dedupe: true,
            delimiter: None,
        },
        Op::Rename {
            from: name("tasi.txt"),
            to: name("arsiv/tasindi.txt"),
        },
    ];
    let plan = validate(&sel, ops);
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    let out = apply(&plan, Some(&undo));
    assert!(out.failed.is_empty(), "{:?}", out.failed);
    assert_eq!(out.done.len(), 6);
    let jpg = image::open(dir.join("foto-1.jpg")).unwrap();
    assert_eq!((jpg.width(), jpg.height()), (20, 10));
    let stripped = std::fs::read(dir.join("foto 1.png")).unwrap();
    assert!(
        !stripped.windows(4).any(|w| w == b"tEXt") && image::load_from_memory(&stripped).is_ok()
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("not.txt")).unwrap(),
        "merhaba dünya, dünya"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("eski.txt")).unwrap(),
        "çığ şöyle"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("veri.csv")).unwrap(),
        "ad,yas\nali,3\nveli,4;5\n"
    );
    assert!(dir.join("arsiv/tasindi.txt").is_file() && !dir.join("tasi.txt").exists());

    // `kor undo` (the same journal) restores everything and removes new files.
    let id = out.undo.unwrap();
    let j = kor_tools::undo::list(&undo)
        .into_iter()
        .find(|j| j.session == id)
        .unwrap();
    let r = kor_tools::undo::undo(&j.dir).unwrap();
    assert!(r.failed.is_empty(), "{r:?}");
    assert_eq!(std::fs::read(dir.join("foto 1.png")).unwrap(), tagged);
    assert_eq!(std::fs::read(dir.join("eski.txt")).unwrap(), latin);
    assert!(dir.join("tasi.txt").is_file() && !dir.join("arsiv/tasindi.txt").exists());
    assert!(!dir.join("foto-1.jpg").exists());
}

#[test]
fn jpeg_metadata_is_stripped_losslessly() {
    let mut jpg = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(8, 8)
        .write_to(&mut jpg, image::ImageFormat::Jpeg)
        .unwrap();
    let raw = jpg.into_inner();
    let mut tagged = raw[..2].to_vec();
    let exif = b"Exif\0\0konum";
    tagged.extend_from_slice(&[0xFF, 0xE1]);
    tagged.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
    tagged.extend_from_slice(exif);
    tagged.extend_from_slice(&raw[2..]);
    assert_eq!(strip_jpeg(&tagged).unwrap(), raw);
    assert!(strip_jpeg(b"nope").is_err());
}

#[test]
fn pdfs_merge_in_order() {
    use lopdf::{Document, Object, Stream, dictionary};
    let d = tempfile::tempdir().unwrap();
    let make = |p: &Path, n: usize| {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let kids: Vec<Object> = (0..n)
            .map(|_| {
                let c = doc.add_object(Stream::new(dictionary! {}, b"".to_vec()));
                doc.add_object(
                    dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => c },
                )
                .into()
            })
            .collect();
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => kids, "Count" => i64::try_from(n).unwrap(),
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", cat);
        doc.save(p).unwrap();
    };
    make(&d.path().join("a.pdf"), 2);
    make(&d.path().join("b.pdf"), 3);
    let sel = vec![d.path().join("a.pdf"), d.path().join("b.pdf")];
    let plan = validate(
        &sel,
        vec![Op::PdfMerge {
            files: vec![name("a.pdf"), name("b.pdf")],
            to: name("hepsi.pdf"),
        }],
    );
    let out = apply(&plan, None);
    assert!(out.failed.is_empty(), "{:?}", out.failed);
    assert_eq!(
        Document::load(d.path().join("hepsi.pdf"))
            .unwrap()
            .get_pages()
            .len(),
        5
    );
    let bad = validate(
        &sel,
        vec![Op::PdfMerge {
            files: vec![name("a.pdf")],
            to: name("x.pdf"),
        }],
    );
    assert_eq!(bad.warnings.len(), 1);
}

#[test]
fn media_steps_never_pass_raw_arguments() {
    let d = tempfile::tempdir().unwrap();
    let sel = files(d.path(), &[("v.mp4", b"x")]);
    let plan = validate(
        &sel,
        vec![
            Op::Media {
                from: name("v.mp4"),
                to: name("v.webm"),
                width: Some(640),
                height: None,
                audio_bitrate_k: None,
                video_bitrate_k: None,
                start_s: Some(1.5),
                duration_s: None,
            },
            Op::Media {
                from: name("v.mp4"),
                to: name("v"),
                width: None,
                height: None,
                audio_bitrate_k: None,
                video_bitrate_k: None,
                start_s: None,
                duration_s: None,
            },
            Op::Media {
                from: name("v.mp4"),
                to: name("w.mp3"),
                width: None,
                height: None,
                audio_bitrate_k: None,
                video_bitrate_k: None,
                start_s: Some(-1.0),
                duration_s: None,
            },
        ],
    );
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.rows[0].detail, "640x?");
}

#[test]
fn globs_expand_relative_to_the_working_folder() {
    let d = tempfile::tempdir().unwrap();
    files(
        d.path(),
        &[
            ("a.jpg", b""),
            ("b.JPG", b""),
            ("c.png", b""),
            ("alt/d.jpg", b""),
        ],
    );
    let got = expand(&[name("*.jpg")], d.path()).unwrap();
    assert_eq!(got, vec![d.path().join("a.jpg")]);
    let got = expand(&[name("**/*.jpg"), name("c.png")], d.path()).unwrap();
    assert_eq!(got.len(), 3);
    assert!(expand(&[name("yok.txt")], d.path()).is_err());
}

struct Scripted(String);

#[async_trait]
impl Planner for Scripted {
    async fn ask(&self, prompt: String) -> Result<String, String> {
        assert!(prompt.contains("a.txt") && prompt.contains("There is no delete operation"));
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn plan_asks_once_and_validates() {
    let d = tempfile::tempdir().unwrap();
    let sel = files(d.path(), &[("a.txt", b"x")]);
    let p = plan(
        &Scripted(r#"[{"op":"rename","from":"a.txt","to":"b.txt"}]"#.into()),
        &sel,
        "adını b yap",
    )
    .await
    .unwrap();
    assert_eq!(p.steps[0].target, d.path().join("b.txt"));
    assert!(plan(&Scripted("[]".into()), &sel, "  ").await.is_err());
    assert!(plan(&Scripted("[]".into()), &[], "x").await.is_err());
    // Preview only: nothing changed yet.
    assert!(d.path().join("a.txt").is_file());
}

#[test]
fn csv_quotes_round_trip() {
    let out = csv_clean("a,\"b,c\",\"d\"\"e\"\r\n", ',', true, true, false);
    assert_eq!(out, "a,\"b,c\",\"d\"\"e\"\n");
    assert_eq!(sniff_delim("a;b;c\n"), ';');
}
