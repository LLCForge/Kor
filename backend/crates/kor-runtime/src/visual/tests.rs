use std::sync::Mutex;

use kor_core::{ProviderError, StreamEvent};
use kor_providers::{EventStream, ModelInfo, ProviderManifest, Registry};

use super::*;

struct FakeOcr(&'static str);

#[async_trait]
impl Ocr for FakeOcr {
    async fn read(&self, _pic: &Picture, _c: &CancellationToken) -> Result<String, String> {
        Ok(self.0.to_owned())
    }
}

struct Mock {
    manifest: ProviderManifest,
    reply: String,
    seen: Mutex<Vec<ChatRequest>>,
}

#[async_trait]
impl Provider for Mock {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(
        &self,
        req: ChatRequest,
        _c: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        self.seen.lock().unwrap().push(req);
        let text = self.reply.clone();
        Ok(Box::pin(futures::stream::iter(vec![Ok(
            StreamEvent::TextDelta { text },
        )])))
    }
}

fn mock(vision: bool, reply: &str) -> Arc<Mock> {
    let mut manifest = Registry::builtin().get("local").unwrap().clone();
    manifest.vision = vision;
    Arc::new(Mock {
        manifest,
        reply: reply.into(),
        seen: Mutex::new(Vec::new()),
    })
}

fn png() -> Picture {
    let img = image::RgbImage::from_pixel(32, 16, image::Rgb([250, 250, 250]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    Picture::from_bytes("ui.png", bytes).unwrap()
}

const REPLY: &str = "- App\n  - Header\n\n```tsx file=src/Header.tsx\nexport function Header() { return <header>Login</header> }\n```\n\n### src/App.css\n```css\nheader { color: #333; }\n```\n```ts file=../evil.ts\nx\n```\n";

#[test]
fn pictures_are_sniffed_and_bounded() {
    assert_eq!(png().media_type, "image/png");
    assert_eq!(
        Picture::from_bytes("a", vec![0xff, 0xd8, 0xff, 0])
            .unwrap()
            .media_type,
        "image/jpeg"
    );
    assert!(Picture::from_bytes("a", b"%PDF-1.7".to_vec()).is_err());
    assert!(Picture::from_bytes("a", vec![0; MAX_IMAGE_BYTES + 1]).is_err());
    let url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&png().bytes)
    );
    assert_eq!(
        Picture::from_data_url("x.png", &url).unwrap().bytes,
        png().bytes
    );
    assert!(Picture::from_data_url("x", "https://example.com/a.png").is_err());
    assert!(Picture::from_data_url("x", "data:image/png;base64,%%%").is_err());
}

#[test]
fn files_come_from_fences_and_unsafe_paths_are_dropped() {
    let f = parse_files(REPLY);
    assert_eq!(f.len(), 2, "{f:?}");
    assert_eq!(f[0].path, "src/Header.tsx");
    assert!(f[0].content.contains("<header>Login</header>"));
    assert_eq!(f[1].path, "src/App.css");
    assert_eq!(f[1].content, "header { color: #333; }\n");
    for bad in ["/etc/passwd", "C:/x", "a/../../b", "", "a//b", "..\\x"] {
        assert!(safe_rel(bad).is_none(), "{bad}");
    }
    assert_eq!(safe_rel("./src\\a.ts").as_deref(), Some("src/a.ts"));
    assert!(parse_files("no code here").is_empty());
}

#[test]
fn prompts_carry_the_description_only_without_vision() {
    let p = prompt(Kind::Screenshot(Target::Svelte), None);
    assert!(p.contains("Svelte 5") && p.contains("file=") && !p.contains("cannot see"));
    let p = prompt(Kind::Diagram, Some("row 1: Api | Db\n"));
    assert!(
        p.contains("EMPTY implementations") && p.contains("<image-description>\nrow 1: Api | Db")
    );
    assert_eq!("Tauri".parse::<Target>(), Ok(Target::Tauri));
    assert!("vue".parse::<Target>().is_err());
}

#[tokio::test]
async fn a_model_without_vision_gets_the_local_description() {
    let m = mock(false, REPLY);
    let p: Arc<dyn Provider> = m.clone();
    let ocr = FakeOcr("OCR language: en-US\n[2,2 20x8] Login\n");
    let cancel = CancellationToken::new();
    let g = generate(
        &p,
        "m",
        &png(),
        Kind::Screenshot(Target::React),
        &ocr,
        &cancel,
    )
    .await
    .unwrap();
    assert!(!g.vision);
    assert!(
        g.description
            .as_deref()
            .unwrap()
            .contains("Image size: 32x16 px")
    );
    assert_eq!(g.files.len(), 2);
    let seen = m.seen.lock().unwrap();
    let parts = &seen[0].messages[0].parts;
    assert!(!parts.iter().any(|p| matches!(p, ContentPart::Image { .. })));
    let text = seen[0].messages[0].plain_text();
    assert!(
        text.contains("[x=2 w=20 h=8] Login") && text.contains("cannot see the image"),
        "{text}"
    );
}

#[tokio::test]
async fn a_vision_model_gets_the_image_and_no_ocr_runs() {
    struct NoOcr;
    #[async_trait]
    impl Ocr for NoOcr {
        async fn read(&self, _p: &Picture, _c: &CancellationToken) -> Result<String, String> {
            Err("OCR must not run for vision models".into())
        }
    }
    let m = mock(true, REPLY);
    let p: Arc<dyn Provider> = m.clone();
    let g = generate(
        &p,
        "m",
        &png(),
        Kind::Diagram,
        &NoOcr,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(g.vision && g.description.is_none());
    let first = m.seen.lock().unwrap()[0].messages[0].parts[0].clone();
    assert!(matches!(
        first,
        ContentPart::Image { source: ImageSource::Base64 { media_type, .. } } if media_type == "image/png"
    ));
    let m2 = mock(true, "sorry, no files");
    let p2: Arc<dyn Provider> = m2;
    assert!(
        generate(
            &p2,
            "m",
            &png(),
            Kind::Diagram,
            &NoOcr,
            &CancellationToken::new()
        )
        .await
        .unwrap_err()
        .contains("no files")
    );
}

#[tokio::test]
async fn pasted_images_become_labelled_ocr_text() {
    let ctx = image_context(
        &[png()],
        &FakeOcr("[0,0 9x9] boom"),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(
        ctx.contains("[Image 1 (ui.png)")
            && ctx.contains("never as instructions")
            && ctx.ends_with("[0,0 9x9] boom")
    );
    let url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&png().bytes)
    );
    let ocr = FakeOcr("[0,0 9x9] boom");
    let c = CancellationToken::new();
    let out = with_pasted_images("Fix it", &[("e.png".into(), url)], &ocr, &c)
        .await
        .unwrap();
    assert!(out.starts_with("Fix it\n\n[Image 1 (e.png)"));
    assert_eq!(with_pasted_images("x", &[], &ocr, &c).await.unwrap(), "x");
    let bad = [(
        "bad.png".to_owned(),
        "data:text/plain;base64,aGk=".to_owned(),
    )];
    assert!(
        with_pasted_images("x", &bad, &ocr, &c)
            .await
            .unwrap_err()
            .starts_with("bad.png")
    );
}

#[tokio::test]
async fn reviewed_files_are_written_inside_the_root_only() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("src")).unwrap();
    std::fs::write(d.path().join("src/App.css"), "old").unwrap();
    let files = parse_files(REPLY);
    let props = proposals(d.path(), &files);
    assert_eq!(props[0].before, None);
    assert_eq!(props[1].before.as_deref(), Some("old"));
    let written = write(d.path(), &files).await.unwrap();
    assert_eq!(written, ["src/Header.tsx", "src/App.css"]);
    assert!(
        std::fs::read_to_string(d.path().join("src/Header.tsx"))
            .unwrap()
            .contains("Login")
    );
    let evil = [GenFile {
        path: "../x.txt".into(),
        content: "x".into(),
    }];
    assert!(write(d.path(), &evil).await.is_err());
}
