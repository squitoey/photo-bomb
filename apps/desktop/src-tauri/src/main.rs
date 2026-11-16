// Desktop shell: translates UI calls into photobomb-core calls. No logic of its own.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::borrow::Cow;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use photobomb_core::{
    group, BatchResult, GroupPage, HistoryEntry, Library, Op, Photo, PhotoDetails, PhotoPage, Plan,
    ScanProgress, ScanSummary, SimilarOpts, Source, Status,
};
use serde::Serialize;
use tauri::http::{Request, Response, StatusCode};
use tauri::{AppHandle, Emitter, State};

struct AppState {
    lib: Arc<Library>,
    cancel: Arc<AtomicBool>,
    scanning: Arc<AtomicBool>,
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

#[derive(Serialize)]
struct AppStatus {
    #[serde(flatten)]
    status: Status,
    scanning: bool,
    ignored: i64,
    library_path: String,
}

#[derive(Serialize, Clone)]
struct ScanFinished {
    summary: Option<ScanSummary>,
    error: Option<String>,
}

#[derive(Serialize)]
struct Mark {
    id: i64,
    size: i64,
    path: String,
}

#[tauri::command(async)]
fn status(state: State<AppState>) -> CmdResult<AppStatus> {
    Ok(AppStatus {
        status: state.lib.status().map_err(err)?,
        scanning: state.scanning.load(Ordering::Relaxed),
        ignored: state.lib.ignored_count().map_err(err)?,
        library_path: state.lib.data_dir.to_string_lossy().to_string(),
    })
}

#[tauri::command(async)]
fn sources(state: State<AppState>) -> CmdResult<Vec<Source>> {
    state.lib.sources().map_err(err)
}

#[tauri::command(async)]
fn add_source(state: State<AppState>, path: String) -> CmdResult<Source> {
    state.lib.add_source(Path::new(&path)).map_err(err)
}

#[tauri::command(async)]
fn remove_source(state: State<AppState>, id: i64) -> CmdResult<()> {
    if state.scanning.load(Ordering::Relaxed) {
        return Err("Wait for the scan to finish first.".into());
    }
    state.lib.remove_source(id).map_err(err)
}

#[tauri::command]
fn start_scan(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    if state.scanning.swap(true, Ordering::SeqCst) {
        return Ok(()); // already running
    }
    state.cancel.store(false, Ordering::SeqCst);
    let lib = state.lib.clone();
    let cancel = state.cancel.clone();
    let scanning = state.scanning.clone();
    std::thread::spawn(move || {
        let last_emit = Mutex::new(Instant::now() - Duration::from_secs(1));
        let progress_app = app.clone();
        let progress = move |p: ScanProgress| {
            // At most ~10 updates a second, but always let stage completions through.
            let mut last = last_emit.lock().unwrap_or_else(|e| e.into_inner());
            if last.elapsed() >= Duration::from_millis(100) || (p.total > 0 && p.done == p.total) {
                *last = Instant::now();
                let _ = progress_app.emit("scan-progress", p);
            }
        };
        let result = photobomb_core::scan::scan(&lib, &progress, &cancel);
        scanning.store(false, Ordering::SeqCst);
        let payload = match result {
            Ok(summary) => ScanFinished {
                summary: Some(summary),
                error: None,
            },
            Err(e) => ScanFinished {
                summary: None,
                error: Some(err(e)),
            },
        };
        let _ = app.emit("scan-finished", payload);
    });
    Ok(())
}

#[tauri::command]
fn cancel_scan(state: State<AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command(async)]
fn exact_groups(state: State<AppState>, page: usize, page_size: usize) -> CmdResult<GroupPage> {
    let groups = state.lib.exact_groups().map_err(err)?;
    Ok(group::page(groups, page, page_size))
}

#[tauri::command(async)]
fn similar_groups(
    state: State<AppState>,
    opts: SimilarOpts,
    page: usize,
    page_size: usize,
) -> CmdResult<GroupPage> {
    let groups = state.lib.similar_groups(&opts).map_err(err)?;
    Ok(group::page(groups, page, page_size))
}

/// Every photo the auto-select rules would remove, across all groups of this kind.
#[tauri::command(async)]
fn auto_select(
    state: State<AppState>,
    kind: String,
    opts: Option<SimilarOpts>,
) -> CmdResult<Vec<Mark>> {
    let groups = if kind == "exact" {
        state.lib.exact_groups().map_err(err)?
    } else {
        state
            .lib
            .similar_groups(&opts.unwrap_or_default())
            .map_err(err)?
    };
    Ok(group::auto_select(&groups)
        .into_iter()
        .map(|p| Mark {
            id: p.id,
            size: p.size,
            path: p.path.clone(),
        })
        .collect())
}

#[tauri::command(async)]
fn set_removable(state: State<AppState>, id: i64, removable: bool) -> CmdResult<()> {
    state.lib.set_removable(id, removable).map_err(err)
}

/// Takes one photo out of its group; the rest of the group is unaffected.
#[tauri::command(async)]
fn ignore_photo(state: State<AppState>, id: i64, others: Vec<i64>) -> CmdResult<()> {
    state.lib.ignore_photo(id, &others).map_err(err)
}

#[tauri::command(async)]
fn unignore_photo(state: State<AppState>, id: i64) -> CmdResult<()> {
    state.lib.unignore_photo(id).map_err(err)
}

/// Photos in no exact or similar group at this similarity level.
#[tauri::command(async)]
fn other_photos(
    state: State<AppState>,
    opts: SimilarOpts,
    page: usize,
    page_size: usize,
) -> CmdResult<PhotoPage> {
    let photos = state.lib.other_photos(&opts).map_err(err)?;
    Ok(group::photo_page(photos, page, page_size))
}

#[tauri::command(async)]
fn photo_details(state: State<AppState>, id: i64) -> CmdResult<PhotoDetails> {
    state.lib.photo_details(id).map_err(err)
}

#[tauri::command(async)]
fn ignore_group(state: State<AppState>, ids: Vec<i64>) -> CmdResult<()> {
    state.lib.ignore_group(&ids).map_err(err)
}

#[tauri::command(async)]
fn clear_ignored(state: State<AppState>) -> CmdResult<()> {
    state.lib.clear_ignored().map_err(err)
}

#[tauri::command(async)]
fn plan(
    state: State<AppState>,
    ids: Vec<i64>,
    op: Op,
    similar: Option<SimilarOpts>,
    allow_empty: bool,
) -> CmdResult<Plan> {
    state
        .lib
        .plan(&ids, &op, similar.as_ref(), allow_empty)
        .map_err(err)
}

#[tauri::command(async)]
fn execute(
    state: State<AppState>,
    ids: Vec<i64>,
    op: Op,
    similar: Option<SimilarOpts>,
    allow_empty: bool,
) -> CmdResult<BatchResult> {
    if state.scanning.load(Ordering::Relaxed) {
        return Err("Wait for the scan to finish first.".into());
    }
    state
        .lib
        .execute(&ids, &op, similar.as_ref(), allow_empty)
        .map_err(err)
}

#[tauri::command(async)]
fn history(state: State<AppState>) -> CmdResult<Vec<HistoryEntry>> {
    state.lib.history().map_err(err)
}

#[tauri::command(async)]
fn undo(state: State<AppState>, batch_id: i64) -> CmdResult<BatchResult> {
    state.lib.undo(batch_id).map_err(err)
}

#[tauri::command(async)]
fn unreadable(state: State<AppState>) -> CmdResult<Vec<Photo>> {
    state.lib.unreadable().map_err(err)
}

/// Shows the file in Finder. Only paths that are in the index are accepted.
#[tauri::command(async)]
fn reveal(state: State<AppState>, id: i64) -> CmdResult<()> {
    let photo = state.lib.photo(id).map_err(err)?.ok_or("unknown photo")?;
    std::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(&photo.path)
        .spawn()
        .map_err(err)?;
    Ok(())
}

/// Dev-only: true when the app was started with PHOTOBOMB_DEBUG_DIR set.
#[tauri::command]
fn debug_enabled() -> bool {
    std::env::var_os("PHOTOBOMB_DEBUG_DIR").is_some()
}

/// Dev-only: writes a self-test artefact into PHOTOBOMB_DEBUG_DIR.
#[tauri::command(async)]
fn debug_write(name: String, content: String) -> CmdResult<()> {
    let dir = std::env::var_os("PHOTOBOMB_DEBUG_DIR").ok_or("debug mode is off")?;
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        || name.starts_with('.')
    {
        return Err("bad name".into());
    }
    std::fs::create_dir_all(&dir).map_err(err)?;
    std::fs::write(Path::new(&dir).join(name), content).map_err(err)
}

/// Dev-only: lets the self-test pause without relying on web view timers.
#[tauri::command(async)]
fn debug_sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms.min(5000)));
}

fn mime_for(path: &Path) -> &'static str {
    match photobomb_core::decode::ext_of(path).as_str() {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        _ => "image/jpeg",
    }
}

/// photo://localhost/thumb/{id} and photo://localhost/full/{id}.
/// The webview never sees or supplies file paths, only ids from the index.
fn serve_photo(lib: &Library, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let fail = |code: StatusCode| {
        Response::builder()
            .status(code)
            .body(Cow::Borrowed(&[][..]))
            .unwrap()
    };
    let mut parts = request.uri().path().trim_start_matches('/').split('/');
    let (Some(kind), Some(id)) = (
        parts.next(),
        parts.next().and_then(|s| s.parse::<i64>().ok()),
    ) else {
        return fail(StatusCode::BAD_REQUEST);
    };
    let path = match kind {
        "thumb" => lib.thumb_path(id),
        "full" => match lib.preview_path(id) {
            Ok(p) => p,
            Err(_) => return fail(StatusCode::NOT_FOUND),
        },
        _ => return fail(StatusCode::BAD_REQUEST),
    };
    match std::fs::read(&path) {
        Ok(bytes) => Response::builder()
            .header("Content-Type", mime_for(&path))
            .header("Cache-Control", "no-store")
            .body(Cow::Owned(bytes))
            .unwrap(),
        Err(_) => fail(StatusCode::NOT_FOUND),
    }
}

fn main() {
    let lib = Arc::new(Library::open_default().expect("could not open the photo library index"));
    let protocol_lib = lib.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            lib,
            cancel: Arc::new(AtomicBool::new(false)),
            scanning: Arc::new(AtomicBool::new(false)),
        })
        .register_asynchronous_uri_scheme_protocol("photo", move |_ctx, request, responder| {
            let lib = protocol_lib.clone();
            std::thread::spawn(move || responder.respond(serve_photo(&lib, &request)));
        })
        .invoke_handler(tauri::generate_handler![
            status,
            sources,
            add_source,
            remove_source,
            start_scan,
            cancel_scan,
            exact_groups,
            similar_groups,
            auto_select,
            set_removable,
            ignore_photo,
            unignore_photo,
            other_photos,
            photo_details,
            ignore_group,
            clear_ignored,
            plan,
            execute,
            history,
            undo,
            unreadable,
            reveal,
            debug_enabled,
            debug_write,
            debug_sleep
        ])
        .run(tauri::generate_context!())
        .expect("error while running Photo Bomb");
}
