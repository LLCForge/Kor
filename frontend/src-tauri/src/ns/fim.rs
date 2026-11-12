//! `fim.complete(file, line, col?, write?)` (§4.2): Mistral Codestral'ın imleç konumuna koyacağı
//! kod → `{ text, written }`. İş mantığı `kor_runtime::fim`; dosya açık projenin içinde olmalıdır.

use kor_i18n::Localizer;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::api::{Res, root, s};
use crate::state::AppState;

pub const NS: &str = "fim";

pub async fn call(
    _ns: &str,
    name: &str,
    args: &[Value],
    _app: &AppHandle,
    state: &AppState,
) -> Option<Res> {
    if name != "complete" {
        return None;
    }
    Some(complete(args, state).await)
}

async fn complete(args: &[Value], state: &AppState) -> Res {
    let file = s(args, 0, "fim.complete")?;
    let line = args
        .get(1)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or("`fim.complete`: 2. bağımsız değişken satır numarası olmalı")?;
    let col = args
        .get(2)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok());
    let write = args.get(3).and_then(Value::as_bool).unwrap_or(false);
    let root = root(state)?;
    let l = crate::bridge::load_config(&root).map_or_else(
        |_| Localizer::new(kor_i18n::Lang::Tr),
        |c| Localizer::new(c.config.ui.language.resolve()),
    );
    let cursor = kor_runtime::fim::locate(&l, &root, file, line, col)?;
    let text = kor_runtime::fim::complete(&l, &cursor).await?;
    if write {
        kor_runtime::fim::insert(&cursor, &text).map_err(|e| e.to_string())?;
    }
    Ok(json!({ "text": text, "written": write }))
}
