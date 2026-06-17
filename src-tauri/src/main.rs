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
    sync::Mutex,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::ManagerExt;
use uuid::Uuid;

const NOTION_VERSION: &str = "2026-03-11";

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
        .unwrap()
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
    rows(
        &state.db.lock().unwrap(),
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
        .unwrap()
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
    let db = state.db.lock().unwrap();
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
        .unwrap()
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
fn get_settings(app: AppHandle, state: State<AppState>) -> Value {
    let page_id = get_setting(&state.db.lock().unwrap(), "notion_page_id");
    let has_token = Entry::new("inspiration-inbox", "notion-token")
        .and_then(|entry| entry.get_password())
        .is_ok();
    json!({
        "pageId": page_id,
        "hasToken": has_token,
        "autostart": app.autolaunch().is_enabled().unwrap_or(false)
    })
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    page_id: String,
    token: String,
    autostart: bool,
    state: State<AppState>,
) -> Result<(), String> {
    state
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO settings(key,value)
             VALUES('notion_page_id',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [page_id],
        )
        .map_err(|error| error.to_string())?;

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
    .map_err(|error| error.to_string())
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
            .unwrap()
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
            .multipart(multipart::Form::new().part(
                "file",
                multipart::Part::bytes(bytes)
                    .file_name(filename)
                    .mime_str("image/jpeg")
                    .unwrap(),
            ))
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
        .unwrap_or(&Vec::new())
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
        client
            .delete(format!("https://api.notion.com/v1/blocks/{id}"))
            .bearer_auth(token)
            .header("Notion-Version", NOTION_VERSION)
            .send()
            .await
            .map_err(|error| error.to_string())?
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
            let db = state.db.lock().unwrap();
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
                let result = if record.action == "delete" {
                    delete_from_notion(
                        &token,
                        record.notion_block_ids.as_deref().unwrap_or_default(),
                    )
                    .await
                    .map(|_| Vec::new())
                } else {
                    append_to_notion(&token, &page, record).await
                };
                let db = state.db.lock().unwrap();
                match result {
                    Ok(block_ids) => {
                        if record.action == "delete" {
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
fn set_expanded(app: AppHandle, expanded: bool) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    window
        .set_always_on_top(true)
        .map_err(|error| error.to_string())?;
    window
        .set_size(tauri::LogicalSize::new(
            360.0,
            if expanded { 305.0 } else { 44.0 },
        ))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_details_mode(app: AppHandle, enabled: bool) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    window
        .set_always_on_top(true)
        .map_err(|error| error.to_string())?;
    window
        .set_size(tauri::LogicalSize::new(
            if enabled { 880.0 } else { 360.0 },
            if enabled { 680.0 } else { 44.0 },
        ))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn open_details(app: AppHandle) -> Result<(), String> {
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
fn snap_main_window(app: AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    let position = window.outer_position().map_err(|error| error.to_string())?;
    let size = window.outer_size().map_err(|error| error.to_string())?;
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or("没有可用显示器")?;
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();

    let left = monitor_position.x;
    let top = monitor_position.y;
    let right = left + monitor_size.width as i32 - size.width as i32;
    let bottom = top + monitor_size.height as i32 - size.height as i32;
    let center_x = position.x + size.width as i32 / 2;
    let center_y = position.y + size.height as i32 / 2;

    let distances = [
        ("left", (position.x - left).abs()),
        ("right", (right - position.x).abs()),
        ("top", (position.y - top).abs()),
        ("bottom", (bottom - position.y).abs()),
    ];
    let nearest = distances
        .iter()
        .min_by_key(|(_, distance)| *distance)
        .map(|(edge, _)| *edge)
        .unwrap_or("left");

    let margin = 8;
    let (x, y) = match nearest {
        "left" => (
            left + margin,
            center_y.clamp(top + margin, bottom - margin),
        ),
        "right" => (
            right - margin,
            center_y.clamp(top + margin, bottom - margin),
        ),
        "top" => (
            center_x.clamp(left + margin, right - margin),
            top + margin,
        ),
        _ => (
            center_x.clamp(left + margin, right - margin),
            bottom - margin,
        ),
    };

    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|error| error.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                window.show().ok();
            }
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            fs::create_dir_all(&dir)?;
            app.manage(AppState {
                db: Mutex::new(init_db(&dir.join("inbox.sqlite")).map_err(std::io::Error::other)?),
                data_dir: dir,
            });
            app.autolaunch().enable().ok();
            tauri::async_runtime::spawn(sync_loop(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            save_record,
            list_records,
            update_record,
            delete_record,
            retry_record,
            get_settings,
            save_settings,
            set_expanded,
            set_details_mode,
            open_details,
            snap_main_window
        ])
        .run(tauri::generate_context!())
        .expect("应用启动失败");
}
