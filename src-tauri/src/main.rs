#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine;
use chrono::Utc;
use image::ImageReader;
use keyring::Entry;
use reqwest::multipart;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    sync::Mutex,
    time::Duration,
};
use tauri::{
    AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, State, WebviewUrl,
    WebviewWindowBuilder,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_updater::UpdaterExt;
use uuid::Uuid;

const NOTION_VERSION: &str = "2026-03-11";
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const DEFAULT_SUMMON_SHORTCUT: &str = "Ctrl+Alt+Space";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStatus {
    available: bool,
    current_version: String,
    version: Option<String>,
    body: Option<String>,
}

struct AppState {
    db: Mutex<Connection>,
    data_dir: PathBuf,
}

#[derive(Serialize)]
struct Record {
    id: String,
    category: Option<String>,
    content: String,
    image_path: Option<String>,
    created_at: String,
    updated_at: String,
    status: String,
    error: Option<String>,
    action: String,
    notion_block_ids: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveInput {
    content: String,
    category: Option<String>,
    image_data_url: Option<String>,
}

fn init_db(path: &Path) -> Result<Connection, String> {
    let db = Connection::open(path).map_err(|error| error.to_string())?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS records (
          id TEXT PRIMARY KEY,
          content TEXT NOT NULL,
          image_path TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          status TEXT NOT NULL DEFAULT 'pending',
          error TEXT,
          action TEXT NOT NULL DEFAULT 'create',
          attempts INTEGER NOT NULL DEFAULT 0,
          next_retry_at INTEGER NOT NULL DEFAULT 0,
          notion_block_ids TEXT
        );
        CREATE TABLE IF NOT EXISTS settings (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sticky_notes (
          id TEXT PRIMARY KEY,
          content TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );",
    )
    .map_err(|error| error.to_string())?;

    if db.prepare("SELECT category FROM records LIMIT 1").is_err() {
        db.execute("ALTER TABLE records ADD COLUMN category TEXT", [])
            .map_err(|error| error.to_string())?;
    }
    if db
        .prepare("SELECT notion_block_ids FROM records LIMIT 1")
        .is_err()
    {
        db.execute("ALTER TABLE records ADD COLUMN notion_block_ids TEXT", [])
            .map_err(|error| error.to_string())?;
    }

    Ok(db)
}

fn rows(
    db: &Connection,
    query: &str,
    bind: &[&dyn rusqlite::ToSql],
) -> Result<Vec<Record>, String> {
    let mut stmt = db.prepare(query).map_err(|error| error.to_string())?;
    let result = stmt
        .query_map(bind, |row| {
            Ok(Record {
                id: row.get(0)?,
                category: row.get(1)?,
                content: row.get(2)?,
                image_path: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                status: row.get(6)?,
                error: row.get(7)?,
                action: row.get(8)?,
                notion_block_ids: row.get(9)?,
            })
        })
        .map_err(|error| error.to_string())?;

    result
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn save_image(data_dir: &Path, data_url: &str) -> Result<String, String> {
    let encoded = data_url
        .split_once(',')
        .map(|value| value.1)
        .ok_or("图片格式无效")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| error.to_string())?;
    let image = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())?
        .thumbnail(1920, 1920);
    let dir = data_dir.join("images");
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(format!("{}.jpg", Uuid::new_v4()));
    image
        .save_with_format(&path, image::ImageFormat::Jpeg)
        .map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn save_record(input: SaveInput, state: State<AppState>) -> Result<String, String> {
    if input.content.trim().is_empty() && input.image_data_url.is_none() {
        return Err("请输入文字或添加图片".into());
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let image_path = input
        .image_data_url
        .as_deref()
        .map(|value| save_image(&state.data_dir, value))
        .transpose()?;
    let category = input
        .category
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    state
        .db
        .lock()
        .map_err(|error| error.to_string())?
        .execute(
            "INSERT INTO records(id,category,content,image_path,created_at,updated_at,status,action)
             VALUES(?1,?2,?3,?4,?5,?5,'pending','create')",
            params![id, category, input.content.trim(), image_path, now],
        )
        .map_err(|error| error.to_string())?;
    Ok(id)
}

#[tauri::command]
fn list_records(search: String, state: State<AppState>) -> Result<Vec<Record>, String> {
    let pattern = format!("%{}%", search);
    let db = state.db.lock().map_err(|error| error.to_string())?;
    rows(
        &db,
        "SELECT id,category,content,image_path,created_at,updated_at,status,error,action,notion_block_ids
         FROM records
         WHERE content LIKE ?1 OR IFNULL(category,'') LIKE ?1
         ORDER BY created_at DESC",
        &[&pattern],
    )
}

#[tauri::command]
fn update_record(id: String, content: String, state: State<AppState>) -> Result<(), String> {
    state
        .db
        .lock()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE records
             SET content=?2,updated_at=?3,status='pending',error=NULL,
                 action='correction',attempts=0,next_retry_at=0
             WHERE id=?1",
            params![id, content.trim(), Utc::now().to_rfc3339()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn delete_record(id: String, state: State<AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let pending_create = db
        .query_row(
            "SELECT action='create' AND status!='synced' FROM records WHERE id=?1",
            [&id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);

    if pending_create {
        let image_path = db
            .query_row("SELECT image_path FROM records WHERE id=?1", [&id], |row| {
                row.get::<_, Option<String>>(0)
            })
            .unwrap_or(None);
        db.execute("DELETE FROM records WHERE id=?1", [&id])
            .map_err(|error| error.to_string())?;
        if let Some(path) = image_path {
            let _ = fs::remove_file(path);
        }
        return Ok(());
    }

    db.execute(
        "UPDATE records
             SET updated_at=?2,status='pending',error=NULL,
                 action='delete',attempts=0,next_retry_at=0
             WHERE id=?1",
        params![id, Utc::now().to_rfc3339()],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn retry_record(id: String, state: State<AppState>) -> Result<(), String> {
    state
        .db
        .lock()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE records
             SET status='pending',error=NULL,attempts=0,next_retry_at=0
             WHERE id=?1",
            params![id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn get_setting(db: &Connection, key: &str) -> String {
    db.query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
        row.get(0)
    })
    .unwrap_or_default()
}

#[tauri::command]
fn get_sticky_note(state: State<AppState>) -> String {
    state
        .db
        .lock()
        .map(|db| get_setting(&db, "sticky_note"))
        .unwrap_or_default()
}

#[tauri::command]
fn get_sticky_note_by_id(id: String, state: State<AppState>) -> Result<String, String> {
    state
        .db
        .lock()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT content FROM sticky_notes WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_sticky_note_by_id(
    id: String,
    content: String,
    state: State<AppState>,
) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    state
        .db
        .lock()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE sticky_notes SET content=?1, updated_at=?2 WHERE id=?3",
            params![content, now, id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn save_sticky_note(content: String, state: State<AppState>) -> Result<(), String> {
    state
        .db
        .lock()
        .map_err(|error| error.to_string())?
        .execute(
            "INSERT INTO settings(key,value)
             VALUES('sticky_note',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [content],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_sticky_mode(state: State<AppState>) -> String {
    state
        .db
        .lock()
        .map(|db| {
            let mode = get_setting(&db, "sticky_mode");
            if mode == "edge" {
                "edge".to_string()
            } else {
                "free".to_string()
            }
        })
        .unwrap_or_else(|_| "free".to_string())
}

#[tauri::command]
fn set_sticky_mode(mode: String, app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let normalized = if mode == "edge" { "edge" } else { "free" };
    state
        .db
        .lock()
        .map_err(|error| error.to_string())?
        .execute(
            "INSERT INTO settings(key,value)
             VALUES('sticky_mode',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [normalized],
        )
        .map_err(|error| error.to_string())?;
    for (label, _) in app.webview_windows() {
        if label.starts_with("sticky-") {
            let _ = apply_sticky_edge_state(&app, &label, "nearest", normalized == "edge");
        }
    }
    let _ = app.emit("sticky-mode-changed", json!({ "mode": normalized }));
    Ok(())
}

fn sticky_plain_text(input: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    let mut entity = String::new();
    let mut in_entity = false;

    for char_value in input.chars() {
        if in_tag {
            if char_value == '>' {
                in_tag = false;
            }
            continue;
        }

        if in_entity {
            if char_value == ';' {
                output.push_str(match entity.as_str() {
                    "nbsp" => " ",
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    _ => "",
                });
                entity.clear();
                in_entity = false;
            } else {
                entity.push(char_value);
            }
            continue;
        }

        match char_value {
            '<' => in_tag = true,
            '&' => in_entity = true,
            _ => output.push(char_value),
        }
    }

    output
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[tauri::command]
fn sticky_to_record(state: State<AppState>) -> Result<String, String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let content = sticky_plain_text(&get_setting(&db, "sticky_note"));
    drop(db);
    if content.trim().is_empty() {
        return Err("便签为空，不能转为灵感".into());
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    state
        .db
        .lock()
        .map_err(|error| error.to_string())?
        .execute(
            "INSERT INTO records(id,category,content,image_path,created_at,updated_at,status,action)
             VALUES(?1,NULL,?2,NULL,?3,?3,'pending','create')",
            params![id, content.trim(), now],
        )
        .map_err(|error| error.to_string())?;
    Ok(id)
}

#[tauri::command]
fn record_to_sticky(id: String, state: State<AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|error| error.to_string())?;
    let content: String = db
        .query_row("SELECT content FROM records WHERE id=?1", [&id], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO settings(key,value)
         VALUES('sticky_note',?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [content],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_settings(app: AppHandle, state: State<AppState>) -> Value {
    let (page_id, window_color, window_opacity, more_transparent, input_transparent, enter_direct_save, text_stroke, glass_mode) = state
        .db
        .lock()
        .map(|db| {
            (
                get_setting(&db, "notion_page_id"),
                get_setting(&db, "window_color"),
                get_setting(&db, "window_opacity"),
                get_setting(&db, "more_transparent"),
                get_setting(&db, "input_transparent"),
                get_setting(&db, "enter_direct_save"),
                get_setting(&db, "text_stroke"),
                get_setting(&db, "glass_mode"),
            )
        })
        .unwrap_or_default();
    let has_token = Entry::new("inspiration-inbox", "notion-token")
        .and_then(|entry| entry.get_password())
        .is_ok();
    json!({
        "pageId": page_id,
        "hasToken": has_token,
        "autostart": app.autolaunch().is_enabled().unwrap_or(false),
        "windowColor": if window_color.is_empty() { "#f8fafb" } else { &window_color },
        "windowOpacity": if window_opacity.is_empty() { "1" } else { &window_opacity },
        "moreTransparent": more_transparent == "1",
        "inputTransparent": input_transparent == "1",
        "enterDirectSave": enter_direct_save == "1",
        "textStroke": text_stroke == "1",
        "glassMode": glass_mode == "1"
    })
}

fn normalize_notion_page_id(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(String::new());
    }

    let mut candidate = String::new();
    for character in input.chars().chain(std::iter::once('\0')) {
        if character.is_ascii_hexdigit() || character == '-' {
            candidate.push(character);
            continue;
        }

        let compact = candidate
            .chars()
            .filter(|value| value.is_ascii_hexdigit())
            .collect::<String>();
        if compact.len() >= 32 {
            return Ok(compact[compact.len() - 32..].to_ascii_lowercase());
        }
        candidate.clear();
    }

    Err("无法从输入内容中识别 Notion 页面 ID，请粘贴完整页面链接".to_string())
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    page_id: String,
    token: String,
    autostart: bool,
    window_color: String,
    window_opacity: String,
    more_transparent: bool,
    input_transparent: bool,
    enter_direct_save: bool,
    text_stroke: bool,
    glass_mode: bool,
    state: State<AppState>,
) -> Result<(), String> {
    let page_id = normalize_notion_page_id(&page_id)?;
    let opacity = window_opacity
        .parse::<f32>()
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
        .to_string();
    let color = if window_color.trim().is_empty() {
        "#f8fafb".to_string()
    } else {
        window_color.trim().to_string()
    };

    let db = state.db.lock().map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO settings(key,value)
             VALUES('notion_page_id',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [page_id],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO settings(key,value)
             VALUES('window_color',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [&color],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO settings(key,value)
             VALUES('window_opacity',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [&opacity],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO settings(key,value)
             VALUES('more_transparent',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [if more_transparent { "1" } else { "0" }],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO settings(key,value)
             VALUES('input_transparent',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [if input_transparent { "1" } else { "0" }],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO settings(key,value)
             VALUES('enter_direct_save',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [if enter_direct_save { "1" } else { "0" }],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO settings(key,value)
             VALUES('text_stroke',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [if text_stroke { "1" } else { "0" }],
    )
    .map_err(|error| error.to_string())?;
    db.execute(
        "INSERT INTO settings(key,value)
             VALUES('glass_mode',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [if glass_mode { "1" } else { "0" }],
    )
    .map_err(|error| error.to_string())?;
    drop(db);

    if !token.trim().is_empty() {
        Entry::new("inspiration-inbox", "notion-token")
            .map_err(|error| error.to_string())?
            .set_password(token.trim())
            .map_err(|error| error.to_string())?;
    }

    if autostart {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|error| error.to_string())?;

    let _ = app.emit(
        "appearance-changed",
        json!({
            "windowColor": color,
            "windowOpacity": opacity,
            "moreTransparent": more_transparent,
            "inputTransparent": input_transparent,
            "enterDirectSave": enter_direct_save,
            "textStroke": text_stroke,
            "glassMode": glass_mode
        }),
    );
    Ok(())
}

async fn ensure_category_page(
    client: &reqwest::Client,
    token: &str,
    parent_page: &str,
    category: &str,
) -> Result<String, String> {
    let category = category.trim();
    let mut cursor: Option<String> = None;

    loop {
        let mut request = client
            .get(format!(
                "https://api.notion.com/v1/blocks/{parent_page}/children"
            ))
            .bearer_auth(token)
            .header("Notion-Version", NOTION_VERSION)
            .query(&[("page_size", "100")]);
        if let Some(cursor_value) = &cursor {
            request = request.query(&[("start_cursor", cursor_value)]);
        }

        let response: Value = request
            .send()
            .await
            .map_err(|error| error.to_string())?
            .error_for_status()
            .map_err(|error| error.to_string())?
            .json()
            .await
            .map_err(|error| error.to_string())?;

        if let Some(blocks) = response["results"].as_array() {
            for block in blocks {
                let title = block["child_page"]["title"].as_str().unwrap_or_default();
                if block["type"].as_str() == Some("child_page") && title == category {
                    if let Some(id) = block["id"].as_str() {
                        return Ok(id.to_string());
                    }
                }
            }
        }

        if !response["has_more"].as_bool().unwrap_or(false) {
            break;
        }
        cursor = response["next_cursor"].as_str().map(str::to_owned);
    }

    let response: Value = client
        .post("https://api.notion.com/v1/pages")
        .bearer_auth(token)
        .header("Notion-Version", NOTION_VERSION)
        .json(&json!({
            "parent": { "page_id": parent_page },
            "properties": {
                "title": {
                    "title": [{
                        "type": "text",
                        "text": { "content": category }
                    }]
                }
            }
        }))
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())?;

    response["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or("Notion 未返回分类页面 ID".into())
}

async fn append_to_notion(token: &str, page: &str, record: &Record) -> Result<Vec<String>, String> {
    let client = reqwest::Client::new();
    let target_page = match record.category.as_deref().map(str::trim) {
        Some(category) if !category.is_empty() => {
            ensure_category_page(&client, token, page, category).await?
        }
        _ => page.to_string(),
    };
    let label = match record.action.as_str() {
        "delete" => "已删除",
        "correction" => "更正",
        _ => "灵感记录",
    };
    let mut children = vec![json!({
        "object": "block",
        "type": "heading_3",
        "heading_3": {
            "rich_text": [{
                "type": "text",
                "text": {"content": format!("{} · {}", label, record.updated_at)}
            }]
        }
    })];

    if let Some(category) = &record.category {
        if !category.trim().is_empty() {
            children.push(json!({
                "object": "block",
                "type": "paragraph",
                "paragraph": {
                    "rich_text": [{
                        "type": "text",
                        "text": {"content": format!("类别：{}", category)}
                    }]
                }
            }));
        }
    }

    children.push(json!({
        "object": "block",
        "type": "paragraph",
        "paragraph": {
            "rich_text": [{
                "type": "text",
                "text": {"content": record.content}
            }]
        }
    }));

    if let Some(path) = &record.image_path {
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let filename = Path::new(path)
            .file_name()
            .ok_or("图片文件名无效")?
            .to_string_lossy()
            .to_string();
        let upload: Value = client
            .post("https://api.notion.com/v1/file_uploads")
            .bearer_auth(token)
            .header("Notion-Version", NOTION_VERSION)
            .json(&json!({
                "mode": "single_part",
                "filename": filename,
                "content_type": "image/jpeg"
            }))
            .send()
            .await
            .map_err(|error| error.to_string())?
            .error_for_status()
            .map_err(|error| error.to_string())?
            .json()
            .await
            .map_err(|error| error.to_string())?;
        let upload_id = upload["id"].as_str().ok_or("Notion 未返回上传标识")?;
        client
            .post(format!(
                "https://api.notion.com/v1/file_uploads/{upload_id}/send"
            ))
            .bearer_auth(token)
            .header("Notion-Version", NOTION_VERSION)
            .multipart(
                multipart::Form::new().part(
                    "file",
                    multipart::Part::bytes(bytes)
                        .file_name(filename)
                        .mime_str("image/jpeg")
                        .map_err(|error| error.to_string())?,
                ),
            )
            .send()
            .await
            .map_err(|error| error.to_string())?
            .error_for_status()
            .map_err(|error| error.to_string())?;
        children.push(json!({
            "object": "block",
            "type": "image",
            "image": {
                "type": "file_upload",
                "file_upload": {"id": upload_id}
            }
        }));
    }

    let response: Value = client
        .patch(format!(
            "https://api.notion.com/v1/blocks/{target_page}/children"
        ))
        .bearer_auth(token)
        .header("Notion-Version", NOTION_VERSION)
        .json(&json!({ "children": children }))
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())?;

    let ids = response["results"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|block| block["id"].as_str().map(str::to_owned))
        .collect();
    Ok(ids)
}

async fn delete_from_notion(token: &str, block_ids: &str) -> Result<(), String> {
    let ids = block_ids
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Err("这条旧记录没有保存 Notion 块 ID，无法精确删除 Notion 中的原记录".into());
    }

    let client = reqwest::Client::new();
    for id in ids {
        let response = client
            .delete(format!("https://api.notion.com/v1/blocks/{id}"))
            .bearer_auth(token)
            .header("Notion-Version", NOTION_VERSION)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        // 404 表示 block 已经不存在（可能之前已删掉），视为成功
        let status = response.status().as_u16();
        if status == 404 {
            continue;
        }
        response
            .error_for_status()
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn delete_local_record(db: &Connection, record: &Record) -> Result<(), String> {
    db.execute("DELETE FROM records WHERE id=?1", [&record.id])
        .map_err(|error| error.to_string())?;
    if let Some(path) = &record.image_path {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

async fn sync_loop(app: AppHandle) {
    loop {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let (token, page, pending) = {
            let Ok(db) = state.db.lock() else {
                tokio::time::sleep(Duration::from_secs(8)).await;
                continue;
            };
            let token = Entry::new("inspiration-inbox", "notion-token")
                .and_then(|entry| entry.get_password())
                .unwrap_or_default();
            let page = get_setting(&db, "notion_page_id");
            let pending = rows(
                &db,
                "SELECT id,category,content,image_path,created_at,updated_at,status,error,action,notion_block_ids
                 FROM records
                 WHERE status='pending' AND next_retry_at <= unixepoch()
                 ORDER BY created_at
                 LIMIT 1",
                &[],
            )
            .unwrap_or_default();
            (token, page, pending)
        };

        if !token.is_empty() && !page.is_empty() {
            if let Some(record) = pending.first() {
                let (result, is_delete) = if record.action == "delete" {
                    (
                        delete_from_notion(
                            &token,
                            record.notion_block_ids.as_deref().unwrap_or_default(),
                        )
                        .await
                        .map(|_| Vec::new()),
                        true,
                    )
                } else {
                    (append_to_notion(&token, &page, record).await, false)
                };
                // 获取 DB 锁（失败则重试几次，避免 Notion 已删但本地没删）
                let db = loop {
                    if let Ok(db) = state.db.lock() {
                        break db;
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                };
                match result {
                    Ok(block_ids) => {
                        if is_delete {
                            let _ = delete_local_record(&db, record);
                        } else {
                            let existing = record.notion_block_ids.as_deref().unwrap_or_default();
                            let new_ids = block_ids.join(",");
                            let notion_block_ids = if existing.is_empty() {
                                new_ids
                            } else if new_ids.is_empty() {
                                existing.to_string()
                            } else {
                                format!("{existing},{new_ids}")
                            };
                            let _ = db.execute(
                                "UPDATE records
                                 SET status='synced',error=NULL,notion_block_ids=?2
                                 WHERE id=?1",
                                params![record.id, notion_block_ids],
                            );
                        }
                        let _ = app.emit("records-changed", json!({ "id": record.id }));
                    }
                    Err(error) => {
                        let _ = db.execute(
                            "UPDATE records
                             SET status='failed',error=?2,attempts=attempts+1,
                                 next_retry_at=unixepoch()+MIN(3600,30*(1 << MIN(attempts,7)))
                             WHERE id=?1",
                            params![record.id, error],
                        );
                        let _ = app.emit("records-changed", json!({ "id": record.id }));
                    }
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(8)).await;
    }
}

#[tauri::command]
fn set_expanded(
    app: AppHandle,
    expanded: bool,
    actions_expanded: Option<bool>,
    more_open: Option<bool>,
) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    window
        .set_always_on_top(true)
        .map_err(|error| error.to_string())?;
    let _ = actions_expanded;
    let height = if expanded {
        305.0
    } else if more_open.unwrap_or(false) {
        210.0
    } else {
        44.0
    };
    window
        .set_size(tauri::LogicalSize::new(320.0, height))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_details_mode(
    app: AppHandle,
    enabled: bool,
    actions_expanded: Option<bool>,
) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    window
        .set_always_on_top(true)
        .map_err(|error| error.to_string())?;
    window
        .set_size(tauri::LogicalSize::new(
            if enabled {
                880.0
            } else if actions_expanded.unwrap_or(false) {
                470.0
            } else {
                320.0
            },
            if enabled { 680.0 } else { 44.0 },
        ))
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn open_details(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("details") {
        window.show().ok();
        window.set_focus().ok();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "details", WebviewUrl::App("details.html".into()))
        .title("灵感详情")
        .inner_size(880.0, 680.0)
        .build()
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
async fn open_sticky_note(app: AppHandle) -> Result<(), String> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    if let Some(state) = app.try_state::<AppState>() {
        let db = state.db.lock().map_err(|error| error.to_string())?;
        let note_count: i64 = db
            .query_row("SELECT COUNT(*) FROM sticky_notes", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        let initial_content = if note_count == 0 {
            get_setting(&db, "sticky_note")
        } else {
            String::new()
        };
        db.execute(
            "INSERT INTO sticky_notes(id,content,created_at,updated_at) VALUES(?1,?2,?3,?3)",
            params![id, initial_content, now],
        )
        .map_err(|error| error.to_string())?;
    }
    let sticky_mode = app
        .try_state::<AppState>()
        .and_then(|state| {
            state
                .db
                .lock()
                .ok()
                .map(|db| get_setting(&db, "sticky_mode"))
        })
        .filter(|mode| mode == "edge")
        .unwrap_or_else(|| "free".to_string());
    let sticky_url = format!("sticky.html?id={id}&mode={sticky_mode}");
    let window_label = format!("sticky-{id}");
    let initial_monitor = app.primary_monitor().ok().flatten();
    let sticky_size = sticky_logical_size(initial_monitor.as_ref());
    let window = WebviewWindowBuilder::new(&app, &window_label, WebviewUrl::App(sticky_url.into()))
        .title("便签")
        .inner_size(sticky_size, sticky_size)
        .resizable(true)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(false)
        .build()
        .map_err(|error| error.to_string())?;
    if let Some(monitor) = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())
    {
        let position = monitor.position();
        window
            .set_position(PhysicalPosition::new(position.x + 96, position.y + 96))
            .ok();
    }
    window.set_focus().ok();
    if sticky_mode == "edge" {
        let _ = apply_sticky_edge_state(&app, &window_label, "nearest", true);
    }
    Ok(())
}

#[tauri::command]
fn set_sticky_pinned(app: AppHandle, window_label: String, pinned: bool) -> Result<(), String> {
    let window = app
        .get_webview_window(&window_label)
        .ok_or("便签窗口不存在")?;
    window
        .set_always_on_top(pinned)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn snap_sticky_to_nearby(app: AppHandle, window_label: String) -> Result<bool, String> {
    const SNAP_DISTANCE: i32 = 28;

    let window = app
        .get_webview_window(&window_label)
        .ok_or("便签窗口不存在")?;
    let position = window.outer_position().map_err(|error| error.to_string())?;
    let size = window.outer_size().map_err(|error| error.to_string())?;
    let left = position.x;
    let top = position.y;
    let right = left + size.width as i32;
    let bottom = top + size.height as i32;
    let mut best: Option<(i32, i32, i32)> = None;

    for (label, other) in app.webview_windows() {
        if label == window_label
            || !label.starts_with("sticky-")
            || !other.is_visible().unwrap_or(false)
        {
            continue;
        }
        let other_position = match other.outer_position() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let other_size = match other.outer_size() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let other_left = other_position.x;
        let other_top = other_position.y;
        let other_right = other_left + other_size.width as i32;
        let other_bottom = other_top + other_size.height as i32;
        let vertical_overlap = bottom.min(other_bottom) - top.max(other_top);
        let horizontal_overlap = right.min(other_right) - left.max(other_left);

        let candidates = [
            (
                (left - other_right).abs(),
                other_right,
                top,
                vertical_overlap > 24,
            ),
            (
                (right - other_left).abs(),
                other_left - size.width as i32,
                top,
                vertical_overlap > 24,
            ),
            (
                (top - other_bottom).abs(),
                left,
                other_bottom,
                horizontal_overlap > 24,
            ),
            (
                (bottom - other_top).abs(),
                left,
                other_top - size.height as i32,
                horizontal_overlap > 24,
            ),
        ];
        for (distance, x, y, overlaps) in candidates {
            if overlaps
                && distance <= SNAP_DISTANCE
                && best.is_none_or(|current| distance < current.0)
            {
                best = Some((distance, x, y));
            }
        }
    }

    if let Some((_, x, y)) = best {
        window
            .set_position(PhysicalPosition::new(x, y))
            .map_err(|error| error.to_string())?;
        return Ok(true);
    }
    Ok(false)
}

fn sticky_logical_size(monitor: Option<&Monitor>) -> f64 {
    const DEFAULT_SIZE: f64 = 377.0;
    const MIN_SIZE: f64 = 300.0;
    const MAX_SIZE: f64 = 440.0;

    let Some(monitor) = monitor else {
        return DEFAULT_SIZE;
    };
    let scale = monitor.scale_factor();
    let size = monitor.size();
    let logical_width = size.width as f64 / scale;
    let logical_height = size.height as f64 / scale;
    (logical_width * 0.22)
        .min(logical_height * 0.35)
        .clamp(MIN_SIZE, MAX_SIZE)
        .round()
}

fn apply_sticky_edge_state(
    app: &AppHandle,
    window_label: &str,
    edge: &str,
    collapsed: bool,
) -> Result<String, String> {
    const EDGE_VISIBLE: i32 = 12;

    let window = app
        .get_webview_window(window_label)
        .ok_or("便签窗口不存在")?;
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        let normal_size = sticky_logical_size(None) as u32;
        window
            .set_size(PhysicalSize::new(
                if collapsed {
                    EDGE_VISIBLE as u32
                } else {
                    normal_size
                },
                normal_size,
            ))
            .map_err(|error| error.to_string())?;
        window
            .set_position(PhysicalPosition::new(0, 96))
            .map_err(|error| error.to_string())?;
        return Ok("left".to_string());
    };

    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let logical_size = sticky_logical_size(Some(&monitor));
    let physical_size = (logical_size * monitor.scale_factor()).round() as u32;
    let left = monitor_position.x;
    let top = monitor_position.y;
    let right = left + monitor_size.width as i32;
    let bottom = top + monitor_size.height as i32;
    let width = physical_size as i32;
    let height = physical_size as i32;
    let position = window
        .outer_position()
        .unwrap_or_else(|_| PhysicalPosition::new(left + 96, top + 96));
    let center_x = position.x + width / 2;
    let center_y = position.y + height / 2;

    let normalized_edge = match edge {
        "left" | "right" => edge,
        _ => {
            let left_distance = (center_x - left).abs();
            let right_distance = (right - center_x).abs();
            if left_distance <= right_distance {
                "left"
            } else {
                "right"
            }
        }
    };

    let expanded_left = match normalized_edge {
        "left" => left,
        _ => right - width,
    };
    let expanded_top = center_y.clamp(top, bottom - height);

    let (target_width, target_height, x, y) = if collapsed {
        match normalized_edge {
            "left" => (EDGE_VISIBLE as u32, physical_size, left, expanded_top),
            "right" => (
                EDGE_VISIBLE as u32,
                physical_size,
                right - EDGE_VISIBLE,
                expanded_top,
            ),
            _ => (EDGE_VISIBLE as u32, physical_size, left, expanded_top),
        }
    } else {
        (physical_size, physical_size, expanded_left, expanded_top)
    };

    window
        .set_size(PhysicalSize::new(target_width, target_height))
        .map_err(|error| error.to_string())?;
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|error| error.to_string())?;
    window.show().ok();
    window.set_focus().ok();
    Ok(normalized_edge.to_string())
}

#[tauri::command]
fn set_sticky_edge_state(
    app: AppHandle,
    window_label: String,
    edge: String,
    collapsed: bool,
) -> Result<String, String> {
    apply_sticky_edge_state(&app, &window_label, &edge, collapsed)
}

#[tauri::command]
fn close_sticky_note(app: AppHandle, window_label: String) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(&window_label) {
        window.hide().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn minimize_sticky_note(app: AppHandle, window_label: String) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(&window_label) {
        window.minimize().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn open_screen_clip() -> Result<(), String> {
    Command::new("explorer.exe")
        .arg("ms-screenclip:")
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn drag_window(app: AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    window.start_dragging().map_err(|error| error.to_string())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
async fn check_for_update(app: AppHandle) -> Result<UpdateStatus, String> {
    let current_version = app.package_info().version.to_string();
    let update = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;

    Ok(match update {
        Some(update) => UpdateStatus {
            available: true,
            current_version,
            version: Some(update.version),
            body: update.body,
        },
        None => UpdateStatus {
            available: false,
            current_version,
            version: None,
            body: None,
        },
    })
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let update = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?
        .ok_or("当前已经是最新版本")?;

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;
    app.restart();
}

async fn update_check_loop(app: AppHandle) {
    tokio::time::sleep(Duration::from_secs(15)).await;
    loop {
        if let Ok(updater) = app.updater() {
            if let Ok(Some(update)) = updater.check().await {
                let _ = app.emit(
                    "update-available",
                    json!({
                        "version": update.version,
                        "body": update.body,
                    }),
                );
            }
        }
        tokio::time::sleep(UPDATE_CHECK_INTERVAL).await;
    }
}

fn toggle_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        window.hide().ok();
        return;
    }

    window.set_always_on_top(true).ok();
    window.show().ok();
    window.set_focus().ok();
    let _ = window.emit("summon-floating-bar", ());
}

fn register_summon_shortcut(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _, event| {
            if event.state == ShortcutState::Pressed {
                toggle_main_window(app);
            }
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_summon_shortcut(state: State<AppState>) -> String {
    state
        .db
        .lock()
        .map(|db| {
            let shortcut = get_setting(&db, "summon_shortcut");
            if shortcut.is_empty() {
                DEFAULT_SUMMON_SHORTCUT.to_string()
            } else {
                shortcut
            }
        })
        .unwrap_or_else(|_| DEFAULT_SUMMON_SHORTCUT.to_string())
}

#[tauri::command]
fn set_summon_shortcut(
    shortcut: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return Err("快捷键不能为空".to_string());
    }

    let current = get_summon_shortcut(state.clone());
    app.global_shortcut().unregister(current.as_str()).ok();
    if let Err(error) = register_summon_shortcut(&app, shortcut) {
        register_summon_shortcut(&app, &current).ok();
        return Err(format!("快捷键不可用：{error}"));
    }

    state
        .db
        .lock()
        .map_err(|error| error.to_string())?
        .execute(
            "INSERT INTO settings(key,value)
             VALUES('summon_shortcut',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [shortcut],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                window.show().ok();
            }
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            fs::create_dir_all(&dir)?;
            app.manage(AppState {
                db: Mutex::new(init_db(&dir.join("inbox.sqlite")).map_err(std::io::Error::other)?),
                data_dir: dir,
            });
            let shortcut = {
                let state = app.state::<AppState>();
                state
                    .db
                    .lock()
                    .map(|db| {
                        let value = get_setting(&db, "summon_shortcut");
                        if value.is_empty() {
                            DEFAULT_SUMMON_SHORTCUT.to_string()
                        } else {
                            value
                        }
                    })
                    .unwrap_or_else(|_| DEFAULT_SUMMON_SHORTCUT.to_string())
            };
            register_summon_shortcut(app.handle(), &shortcut).ok();
            app.autolaunch().enable().ok();
            tauri::async_runtime::spawn(sync_loop(app.handle().clone()));
            tauri::async_runtime::spawn(update_check_loop(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            save_record,
            list_records,
            update_record,
            delete_record,
            retry_record,
            get_sticky_note,
            save_sticky_note,
            get_sticky_note_by_id,
            save_sticky_note_by_id,
            get_sticky_mode,
            set_sticky_mode,
            sticky_to_record,
            record_to_sticky,
            get_settings,
            save_settings,
            set_expanded,
            set_details_mode,
            open_details,
            open_sticky_note,
            set_sticky_pinned,
            snap_sticky_to_nearby,
            set_sticky_edge_state,
            close_sticky_note,
            minimize_sticky_note,
            open_screen_clip,
            drag_window,
            quit_app,
            check_for_update,
            install_update,
            get_summon_shortcut,
            set_summon_shortcut
        ])
        .run(tauri::generate_context!())
        .expect("应用启动失败");
}

#[cfg(test)]
mod tests {
    use super::normalize_notion_page_id;

    const PAGE_ID: &str = "3816bf70b4ce8067bfaed0a7b631c7c2";

    #[test]
    fn accepts_plain_notion_page_id() {
        assert_eq!(normalize_notion_page_id(PAGE_ID).unwrap(), PAGE_ID);
    }

    #[test]
    fn extracts_id_from_notion_share_link() {
        let link = format!("https://www.notion.so/Ideas-{PAGE_ID}?source=copy_link");
        assert_eq!(normalize_notion_page_id(&link).unwrap(), PAGE_ID);
    }

    #[test]
    fn accepts_hyphenated_page_id() {
        let link = "https://www.notion.so/3816bf70-b4ce-8067-bfae-d0a7b631c7c2";
        assert_eq!(normalize_notion_page_id(link).unwrap(), PAGE_ID);
    }
}
