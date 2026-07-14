//! Zen PDF — Rust Reader as a first-class Zed panel.
//!
//! A calm document workspace docked in Zed. The panel is a thin client: it
//! drives the offline engine (`reader-engined`) over a subprocess boundary and
//! renders the results, opening produced artifacts (extracted text, rendered
//! pages) as ordinary Zed tabs so they get search / editing / diff for free.
//!
//! The tool surface is contextual: select a document and its available actions
//! appear as a toolbar. Not-yet-built tools (OCR without models, compression,
//! deskew) are shown as disabled controls with an honest label rather than
//! hidden — the surface reflects reality.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use gpui::{
    Action, App, AsyncWindowContext, Context, Entity, EventEmitter, ExternalPaths, FocusHandle,
    Focusable, Global, InteractiveElement, IntoElement, ParentElement, PathPromptOptions, Pixels,
    Render, SharedString, Styled, Subscription, WeakEntity, Window, actions, div, px,
};
use serde_json::Value;
use ui::prelude::*;
use ui_input::InputField;
use workspace::{
    OpenOptions, Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

actions!(
    reader,
    [
        ToggleFocus,
        Import,
        ExtractText,
        Ocr,
        ViewPage,
        Narrate,
        Combine,
        Split,
        Rotate,
        Compress,
        Deskew,
        Normalize,
        ToggleContents,
        DeviceIphone,
        DeviceIpad,
        DeviceLetter,
        SplitBy,
        RagExport,
        AnalyzeImage,
        BackupNow,
    ]
);

/// Register the Reader panel's actions on every workspace so the app menu can
/// drive the tools (they act on the panel's selected document).
pub fn init(cx: &mut App) {
    // Default selection so the Contents panel can read the global before any
    // document is picked.
    cx.set_global(ZenSelection::default());
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<ReaderPanel>(window, cx);
        });
        workspace.register_action(|workspace, _: &ToggleContents, window, cx| {
            workspace.toggle_panel_focus::<ContentsPanel>(window, cx);
        });
        workspace.register_action(|workspace, _: &Import, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.open_files(cx));
            }
        });
        // Document files opened/dropped into the editor surface get routed to
        // the library instead of a dead "binary files" tab.
        workspace.register_action(
            |workspace, action: &zed_actions::ImportIntoReader, window, cx| {
                if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                    workspace.open_panel::<ReaderPanel>(window, cx);
                    panel.update(cx, |panel, cx| {
                        panel.import_paths(action.paths.clone(), cx)
                    });
                }
            },
        );
        workspace.register_action(|workspace, _: &ExtractText, window, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.extract_text(window, cx));
            }
        });
        workspace.register_action(|workspace, _: &Ocr, window, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.ocr_selected(window, cx));
            }
        });
        workspace.register_action(|workspace, _: &ViewPage, window, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.view_page(window, cx));
            }
        });
        workspace.register_action(|workspace, _: &Narrate, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.narrate(cx));
            }
        });
        workspace.register_action(|workspace, _: &Combine, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.combine_all(cx));
            }
        });
        workspace.register_action(|workspace, _: &Split, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.split_selected(cx));
            }
        });
        workspace.register_action(|workspace, _: &Rotate, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.rotate_selected(cx));
            }
        });
        workspace.register_action(|workspace, _: &Compress, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.compress_selected(cx));
            }
        });
        workspace.register_action(|workspace, _: &Deskew, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.deskew_selected(cx));
            }
        });
        workspace.register_action(|workspace, _: &Normalize, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.normalize_selected(cx));
            }
        });
        workspace.register_action(|workspace, _: &DeviceIphone, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.device_render("iphone17pro", cx));
            }
        });
        workspace.register_action(|workspace, _: &DeviceIpad, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.device_render("ipadmini", cx));
            }
        });
        workspace.register_action(|workspace, _: &DeviceLetter, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.device_render("letter", cx));
            }
        });
        workspace.register_action(|workspace, _: &SplitBy, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.split_by_selected(cx));
            }
        });
        workspace.register_action(|workspace, _: &RagExport, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.rag_export_selected(cx));
            }
        });
        workspace.register_action(|workspace, _: &AnalyzeImage, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.analyze_image_selected(cx));
            }
        });
        workspace.register_action(|workspace, _: &BackupNow, _, cx| {
            if let Some(panel) = workspace.panel::<ReaderPanel>(cx) {
                panel.update(cx, |panel, cx| panel.backup_now(cx));
            }
        });
    })
    .detach();
}

/// One document in the library, as reported by the engine.
#[derive(Clone, Debug)]
struct Doc {
    id: i64,
    filename: SharedString,
    pages: Option<i64>,
    status: SharedString,
}

/// One full-text search hit from the engine.
#[derive(Clone, Debug)]
struct SearchHit {
    page: i64,
    filename: SharedString,
    snippet: SharedString,
}

/// Shared current-document selection, published by the left Files panel and
/// observed by the right Contents panel.
#[derive(Clone, Default)]
struct ZenSelection {
    asset: Option<i64>,
    filename: SharedString,
    pages: Option<i64>,
    engine_bin: PathBuf,
    data_dir: PathBuf,
}

impl Global for ZenSelection {}

pub struct ReaderPanel {
    focus_handle: FocusHandle,
    workspace: WeakEntity<Workspace>,
    /// Path to the `reader-engined` backend binary.
    engine_bin: PathBuf,
    /// Library data directory (the backend appends `library.midasdoc`).
    data_dir: PathBuf,
    documents: Vec<Doc>,
    selected: Option<i64>,
    status: SharedString,
    busy: bool,
    /// Human-readable capability summary (say / render / OCR / Kokoro).
    capabilities: Option<SharedString>,
    /// Whether the OCR runtime + models are installed (enables the OCR tool).
    ocr_available: bool,
    /// Chapter audio files from the last narration (for the in-app player).
    played_chapters: Vec<PathBuf>,
    /// Whether playback is currently running.
    playing: bool,
    /// Bumped to cancel an in-flight playback loop.
    play_generation: u64,
    /// The library search box.
    search_input: Entity<InputField>,
    /// Results of the last search.
    search_hits: Vec<SearchHit>,
}

impl ReaderPanel {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        cx.update(|window, cx| {
            let search_input = cx.new(|cx| InputField::new(window, cx, "Search library…"));
            cx.new(|cx| {
                let mut panel = ReaderPanel {
                    focus_handle: cx.focus_handle(),
                    workspace,
                    engine_bin: resolve_engine_bin(),
                    data_dir: resolve_data_dir(),
                    documents: Vec::new(),
                    selected: None,
                    status: "Starting the engine…".into(),
                    busy: false,
                    capabilities: None,
                    ocr_available: false,
                    played_chapters: Vec::new(),
                    playing: false,
                    play_generation: 0,
                    search_input,
                    search_hits: Vec::new(),
                };
                panel.refresh(cx);
                panel.load_capabilities(cx);
                panel
            })
        })
    }

    /// Re-read the document list from the engine, off the UI thread.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let bin = self.engine_bin.clone();
        let dir = self.data_dir.clone();
        self.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine_json(&bin, &dir, &["list"]) })
                .await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(value) => {
                        this.documents = parse_docs(&value);
                        if this.selected.is_none() {
                            this.selected = this.documents.first().map(|d| d.id);
                        }
                        this.status = match this.documents.len() {
                            0 => "No documents yet — Import a PDF to begin.".into(),
                            1 => "1 document".into(),
                            n => format!("{n} documents").into(),
                        };
                    }
                    Err(e) => this.status = format!("Engine unavailable: {e}").into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Read engine capabilities once for the quiet status footer.
    fn load_capabilities(&mut self, cx: &mut Context<Self>) {
        let bin = self.engine_bin.clone();
        let dir = self.data_dir.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine_json(&bin, &dir, &["capabilities"]) })
                .await;
            if let Ok(value) = result {
                this.update(cx, |this, cx| {
                    this.ocr_available = value
                        .get("ocr")
                        .and_then(|o| o.get("available"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    this.capabilities = Some(summarize_capabilities(&value));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Open a native file dialog and import the chosen files via the engine.
    fn open_files(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import".into()),
        });
        self.status = "Choose documents to import…".into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let chosen = rx.await.ok().and_then(|r| r.ok()).flatten();
            if let Some(paths) = chosen {
                this.update(cx, |this, cx| this.import_paths(paths, cx)).ok();
            } else {
                this.update(cx, |this, cx| {
                    this.status = "Import cancelled.".into();
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Import each path through the engine, then refresh the list.
    ///
    /// Files are first copied into `<library>/incoming/` BY THIS PROCESS: the
    /// app holds the user's file-access grant (from the open panel or a drag),
    /// while the engine subprocess may be denied by macOS privacy protection
    /// on ~/Documents, ~/Downloads, etc. Importing the staged copy sidesteps
    /// that entirely; the staged file is removed after import (the engine
    /// content-addresses it into its own store).
    pub fn import_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let bin = self.engine_bin.clone();
        let dir = self.data_dir.clone();
        let n = paths.len();
        self.busy = true;
        self.status = format!("Importing {n} document(s)…").into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move {
                    let staging = dir.join("incoming");
                    let mut imported = 0usize;
                    let mut errors: Vec<String> = Vec::new();
                    for path in &paths {
                        let display = path
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.to_string_lossy().to_string());
                        let staged = match stage_copy(&staging, path) {
                            Ok(p) => p,
                            Err(e) => {
                                log_line(&dir, &format!("stage-copy FAILED {display}: {e}"));
                                errors.push(format!("{display}: {e}"));
                                continue;
                            }
                        };
                        let file = staged.to_string_lossy().to_string();
                        match engine_json(&bin, &dir, &["import", "--file", &file]) {
                            Ok(_) => imported += 1,
                            Err(e) => errors.push(format!("{display}: {e}")),
                        }
                        let _ = std::fs::remove_file(&staged);
                    }
                    (imported, errors)
                })
                .await;
            this.update(cx, |this, cx| {
                let (imported, errors) = outcome;
                if let Some(first_err) = errors.first() {
                    this.status = format!(
                        "Imported {imported}/{n} — {first_err} (details: panel.log in the library folder)"
                    )
                    .into();
                } else {
                    this.status = format!("Imported {imported} document(s).").into();
                }
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Render every page of the selected document and show them in a scrollable
    /// document view in the center pane — clicking a document must SHOW it.
    fn open_document_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        let (bin, dir) = (self.engine_bin.clone(), self.data_dir.clone());
        let filename: SharedString = self
            .documents
            .iter()
            .find(|d| d.id == asset)
            .map(|d| d.filename.clone())
            .unwrap_or_else(|| "Document".into());
        let page_count = self
            .documents
            .iter()
            .find(|d| d.id == asset)
            .and_then(|d| d.pages)
            .unwrap_or(1)
            .max(1);
        let ws = self.workspace.clone();
        self.begin("Rendering pages…", cx);
        cx.spawn_in(window, async move |this, cx| {
            let pages = cx
                .background_spawn(async move {
                    let cache = dir.join("render-cache").join(format!("asset{asset}"));
                    let _ = std::fs::create_dir_all(&cache);
                    let mut rendered: Vec<PathBuf> = Vec::new();
                    for p in 0..page_count {
                        let out = cache.join(format!("p{p}.png"));
                        if !out.is_file() {
                            let (a, pg, o) =
                                (asset.to_string(), p.to_string(), out.to_string_lossy().to_string());
                            if let Err(e) = engine_json(
                                &bin,
                                &dir,
                                &["render-asset", "--asset", &a, "--page", &pg, "--dpi", "150",
                                  "--out", &o],
                            ) {
                                log_line(&dir, &format!("page {p} render failed: {e}"));
                                break;
                            }
                        }
                        rendered.push(out);
                    }
                    rendered
                })
                .await;
            let n = pages.len();
            this.update(cx, |this, cx| {
                this.busy = false;
                this.status = if n == 0 {
                    "Could not render this document — see panel.log.".into()
                } else {
                    format!("Showing {n} page(s).").into()
                };
                cx.notify();
            })
            .ok();
            if n > 0 {
                ws.update_in(cx, |ws, window, cx| {
                    let view = cx.new(|cx| DocumentView {
                        filename,
                        pages,
                        focus_handle: cx.focus_handle(),
                    });
                    ws.add_item_to_active_pane(Box::new(view), None, true, window, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// The selected asset, or a LOUD status message when there is none — a
    /// tool invoked with nothing selected must never be a silent no-op.
    fn require_selected(&mut self, cx: &mut Context<Self>) -> Option<i64> {
        let sel = self.selected;
        if sel.is_none() {
            self.status = "Select a document in the list first.".into();
            log_line(&self.data_dir, "tool invoked with no document selected");
            cx.notify();
        }
        sel
    }

    // --- contextual tools (operate on the selected document) ----------------

    /// Extract the text layer to a sidecar and open it as a Zed tab.
    fn extract_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        let (bin, dir) = (self.engine_bin.clone(), self.data_dir.clone());
        let out = std::env::temp_dir().join(format!("zenpdf-asset{asset}.txt"));
        let out_engine = out.clone();
        let ws = self.workspace.clone();
        self.begin("Extracting text…", cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let a = asset.to_string();
                    let o = out_engine.to_string_lossy().to_string();
                    // Native text layer first; if the document has none (a scan
                    // or an image), fall back to on-device OCR automatically.
                    let native = engine_json(&bin, &dir, &["doc-text", "--asset", &a, "--out", &o]);
                    let has_text = native
                        .as_ref()
                        .ok()
                        .and_then(|v| v.get("chars").and_then(|c| c.as_i64()))
                        .unwrap_or(0)
                        > 0;
                    if has_text {
                        native
                    } else {
                        engine_json(&bin, &dir, &["ocr-asset", "--asset", &a, "--out", &o])
                            .map(|mut v| {
                                if let Some(obj) = v.as_object_mut() {
                                    obj.insert("ocr".into(), serde_json::Value::Bool(true));
                                }
                                v
                            })
                    }
                })
                .await;
            let ok = result.is_ok();
            this.update(cx, |this, cx| {
                this.busy = false;
                this.status = match &result {
                    Ok(v) => {
                        let chars = v.get("chars").and_then(|c| c.as_i64()).unwrap_or(0);
                        let how = if v.get("ocr").is_some() { " (OCR)" } else { "" };
                        format!("Extracted {chars} characters{how} — opening…").into()
                    }
                    Err(e) => format!("Extract failed: {e}").into(),
                };
                cx.notify();
            })
            .ok();
            if ok {
                ws.update_in(cx, |ws, window, cx| {
                    ws.open_abs_path(out.clone(), OpenOptions::default(), window, cx)
                        .detach();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Force on-device OCR of the selected document and open the text as a tab.
    fn ocr_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        let (bin, dir) = (self.engine_bin.clone(), self.data_dir.clone());
        let out = std::env::temp_dir().join(format!("zenpdf-asset{asset}-ocr.txt"));
        let out_engine = out.clone();
        let ws = self.workspace.clone();
        self.begin("Recognizing text (OCR)…", cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let a = asset.to_string();
                    let o = out_engine.to_string_lossy().to_string();
                    engine_json(&bin, &dir, &["ocr-asset", "--asset", &a, "--out", &o])
                })
                .await;
            let ok = result.is_ok();
            this.update(cx, |this, cx| {
                this.busy = false;
                this.status = match &result {
                    Ok(v) => {
                        let chars = v.get("chars").and_then(|c| c.as_i64()).unwrap_or(0);
                        format!("Recognized {chars} characters — opening…").into()
                    }
                    Err(e) => format!("OCR failed: {e}").into(),
                };
                cx.notify();
            })
            .ok();
            if ok {
                ws.update_in(cx, |ws, window, cx| {
                    ws.open_abs_path(out.clone(), OpenOptions::default(), window, cx)
                        .detach();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Render page 1 of the selected document and open the PNG as a Zed tab.
    fn view_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        let (bin, dir) = (self.engine_bin.clone(), self.data_dir.clone());
        let out = std::env::temp_dir().join(format!("zenpdf-asset{asset}-p1.png"));
        let out_engine = out.clone();
        let ws = self.workspace.clone();
        self.begin("Rendering page…", cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let a = asset.to_string();
                    let o = out_engine.to_string_lossy().to_string();
                    engine_json(
                        &bin,
                        &dir,
                        &["render-asset", "--asset", &a, "--page", "0", "--dpi", "150", "--out", &o],
                    )
                })
                .await;
            let ok = result.is_ok();
            this.update(cx, |this, cx| {
                this.busy = false;
                this.status = match &result {
                    Ok(_) => "Rendered page 1 — opening…".into(),
                    Err(e) => format!("Render failed: {e}").into(),
                };
                cx.notify();
            })
            .ok();
            if ok {
                ws.update_in(cx, |ws, window, cx| {
                    ws.open_abs_path(out.clone(), OpenOptions::default(), window, cx)
                        .detach();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Narrate the selected document to chaptered audio files, written to a
    /// findable folder and revealed in Finder so it can be played.
    fn narrate(&mut self, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        let (bin, dir) = (self.engine_bin.clone(), self.data_dir.clone());
        // A stable, reachable location under the library — not a hidden temp dir.
        let out_dir = self.data_dir.join("audio").join(format!("asset{asset}"));
        let shown = out_dir.to_string_lossy().to_string();
        self.begin("Narrating to audio…", cx);
        cx.spawn(async move |this, cx| {
            let out_dir_for_reveal = out_dir.clone();
            let result = cx
                .background_spawn(async move {
                    let a = asset.to_string();
                    let o = out_dir.to_string_lossy().to_string();
                    let r = engine_json(&bin, &dir, &["listen", "--asset", &a, "--out-dir", &o]);
                    if r.is_ok() {
                        // Reveal the audio folder in Finder so it can be played.
                        let _ = std::process::Command::new("open")
                            .arg(&out_dir_for_reveal)
                            .spawn();
                    }
                    r
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = false;
                this.status = match &result {
                    Ok(v) => {
                        let paths: Vec<PathBuf> = v
                            .get("chapters")
                            .and_then(|c| c.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|c| {
                                        c.get("path").and_then(|p| p.as_str()).map(PathBuf::from)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let n = paths.len();
                        let engine = v.get("engine").and_then(|e| e.as_str()).unwrap_or("");
                        this.played_chapters = paths;
                        format!("Narrated {n} chapter(s) with {engine} — press Play ▶").into()
                    }
                    Err(e) => format!("Narrate failed: {e}").into(),
                };
                cx.notify();
            })
            .ok();
            let _ = shown; // path is also revealed in Finder by the engine
        })
        .detach();
    }

    /// Play the narrated chapters in order via the built-in `afplay`.
    fn play(&mut self, cx: &mut Context<Self>) {
        if self.played_chapters.is_empty() {
            return;
        }
        self.playing = true;
        self.play_generation += 1;
        let generation = self.play_generation;
        let chapters = self.played_chapters.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            for chapter in chapters {
                let still_playing = this
                    .read_with(cx, |this, _| this.playing && this.play_generation == generation)
                    .unwrap_or(false);
                if !still_playing {
                    break;
                }
                let _ = cx
                    .background_spawn(async move {
                        std::process::Command::new("/usr/bin/afplay")
                            .arg(&chapter)
                            .status()
                    })
                    .await;
            }
            this.update(cx, |this, cx| {
                if this.play_generation == generation {
                    this.playing = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Stop playback (invalidate the loop + kill any running `afplay`).
    fn stop_playback(&mut self, cx: &mut Context<Self>) {
        self.playing = false;
        self.play_generation += 1;
        let _ = std::process::Command::new("/usr/bin/pkill")
            .args(["-x", "afplay"])
            .status();
        cx.notify();
    }

    /// Run a full-text search over the library (FTS5 in the engine).
    fn run_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).text(cx);
        let query = query.trim().to_string();
        if query.is_empty() {
            self.search_hits.clear();
            self.status = "Type a search query first.".into();
            cx.notify();
            return;
        }
        let (bin, dir) = (self.engine_bin.clone(), self.data_dir.clone());
        self.begin("Searching…", cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    engine_json(&bin, &dir, &["search", "--query", &query, "--limit", "30"])
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(v) => {
                        this.search_hits = v
                            .get("hits")
                            .and_then(|h| h.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|h| {
                                        Some(SearchHit {
                                            page: h.get("page")?.as_i64()?,
                                            filename: h
                                                .get("filename")
                                                .and_then(|f| f.as_str())
                                                .unwrap_or("")
                                                .to_string()
                                                .into(),
                                            snippet: h
                                                .get("snippet")
                                                .and_then(|s| s.as_str())
                                                .unwrap_or("")
                                                .to_string()
                                                .into(),
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        this.status = match this.search_hits.len() {
                            0 => "No matches.".into(),
                            1 => "1 match".into(),
                            n => format!("{n} matches").into(),
                        };
                    }
                    Err(e) => this.status = format!("Search failed: {e}").into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A no-argument engine mutation (combine / split / rotate) reported in
    /// the status line. `verb` is shown while running; `ok_label` on success.
    fn run_op(&mut self, verb: &str, args: Vec<String>, ok_label: &'static str, cx: &mut Context<Self>) {
        let (bin, dir) = (self.engine_bin.clone(), self.data_dir.clone());
        self.begin(verb, cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                    engine_json(&bin, &dir, &refs)
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = false;
                this.status = match &result {
                    Ok(_) => ok_label.into(),
                    Err(e) => format!("{ok_label} failed: {e}").into(),
                };
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    fn combine_all(&mut self, cx: &mut Context<Self>) {
        self.run_op(
            "Combining all documents…",
            vec!["combine".into(), "--name".into(), "all".into()],
            "Combined into one PDF.",
            cx,
        );
    }

    fn split_selected(&mut self, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        self.run_op(
            "Splitting into pages…",
            vec![
                "split".into(),
                "--asset".into(),
                asset.to_string(),
                "--prefix".into(),
                "split".into(),
            ],
            "Split into per-page PDFs.",
            cx,
        );
    }

    fn rotate_selected(&mut self, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        self.run_op(
            "Rotating 90°…",
            vec![
                "rotate".into(),
                "--asset".into(),
                asset.to_string(),
                "--degrees".into(),
                "90".into(),
                "--output".into(),
                "rotated".into(),
            ],
            "Rotated 90° into a new PDF.",
            cx,
        );
    }

    /// Run an engine command that writes a PDF to `out`, then open it (Preview).
    fn run_pdf_tool(&mut self, verb: &str, args: Vec<String>, cx: &mut Context<Self>) {
        let (bin, dir) = (self.engine_bin.clone(), self.data_dir.clone());
        self.begin(verb, cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                    let r = engine_json(&bin, &dir, &refs);
                    if let Ok(v) = &r {
                        if let Some(out) = v.get("out").and_then(|o| o.as_str()) {
                            let _ = std::process::Command::new("/usr/bin/open").arg(out).spawn();
                        }
                    }
                    r
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = false;
                this.status = match &result {
                    Ok(v) => {
                        let out = v.get("out").and_then(|o| o.as_str()).unwrap_or("");
                        format!("Done — opened {out}").into()
                    }
                    Err(e) => format!("Failed: {e}").into(),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn compress_selected(&mut self, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        self.run_pdf_tool(
            "Compressing…",
            vec!["compress".into(), "--asset".into(), asset.to_string()],
            cx,
        );
    }

    fn deskew_selected(&mut self, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        self.run_pdf_tool(
            "Deskewing…",
            vec!["deskew".into(), "--asset".into(), asset.to_string()],
            cx,
        );
    }

    fn normalize_selected(&mut self, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        self.run_pdf_tool(
            "Normalizing to Letter…",
            vec!["normalize".into(), "--asset".into(), asset.to_string()],
            cx,
        );
    }

    /// Run an engine command and report a summary (from `format_ok`) in the
    /// status line — for tools whose result isn't a single openable file.
    fn run_status<F>(&mut self, verb: &str, args: Vec<String>, format_ok: F, cx: &mut Context<Self>)
    where
        F: Fn(&Value) -> String + Send + 'static,
    {
        let (bin, dir) = (self.engine_bin.clone(), self.data_dir.clone());
        self.begin(verb, cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                    engine_json(&bin, &dir, &refs)
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = false;
                this.status = match &result {
                    Ok(v) => format_ok(v).into(),
                    Err(e) => format!("Failed: {e}").into(),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn device_render(&mut self, target: &str, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        self.run_pdf_tool(
            "Re-rendering for device…",
            vec![
                "device".into(),
                "--asset".into(),
                asset.to_string(),
                "--target".into(),
                target.to_string(),
            ],
            cx,
        );
    }

    fn split_by_selected(&mut self, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        self.run_status(
            "Splitting into chunks…",
            vec!["split-by".into(), "--asset".into(), asset.to_string()],
            |v| {
                let n = v.get("chunks").and_then(|c| c.as_i64()).unwrap_or(0);
                format!("Split into {n} chunk(s) — in the library's exports/")
            },
            cx,
        );
    }

    fn rag_export_selected(&mut self, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        self.run_pdf_tool(
            "Exporting RAG (JSONL)…",
            vec!["rag".into(), "--asset".into(), asset.to_string()],
            cx,
        );
    }

    fn analyze_image_selected(&mut self, cx: &mut Context<Self>) {
        let Some(asset) = self.require_selected(cx) else { return };
        self.run_status(
            "Analyzing image…",
            vec!["analyze-image".into(), "--asset".into(), asset.to_string()],
            |v| {
                let labels: Vec<String> = v
                    .get("labels")
                    .and_then(|l| l.as_array())
                    .map(|arr| {
                        arr.iter()
                            .take(4)
                            .filter_map(|l| l.get("label").and_then(|s| s.as_str()))
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                if labels.is_empty() {
                    "No labels detected.".to_string()
                } else {
                    format!("Detected: {}", labels.join(", "))
                }
            },
            cx,
        );
    }

    fn backup_now(&mut self, cx: &mut Context<Self>) {
        self.run_status(
            "Backing up library…",
            vec!["backup".into()],
            |_| "Library backed up.".to_string(),
            cx,
        );
    }

    fn begin(&mut self, msg: &str, cx: &mut Context<Self>) {
        self.busy = true;
        self.status = msg.to_string().into();
        cx.notify();
    }
}

/// Resolve the backend binary so the app works when launched normally (no env
/// var required). Order: `$READER_ENGINED`, then next to the running executable
/// (self-contained: we ship `reader-engined` beside `zed`), then a couple of
/// dev-build locations, then `reader-engined` on `PATH`.
fn resolve_engine_bin() -> PathBuf {
    if let Some(p) = std::env::var_os("READER_ENGINED") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Bundled beside the binary (release / .app Contents/MacOS).
            let beside = dir.join("reader-engined");
            if beside.is_file() {
                return beside;
            }
            // Dev: zed/target/debug/zed → ../../../target/debug/reader-engined
            let dev = dir.join("../../../target/debug/reader-engined");
            if dev.is_file() {
                return dev;
            }
        }
    }
    PathBuf::from("reader-engined")
}

/// Resolve the library directory: `$READER_DATA_DIR`, else the app-support dir.
fn resolve_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("READER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library/Application Support/RustReader");
    }
    PathBuf::from(".")
}

// ============================================================================
// Document view: the center-pane page display. A reader's main surface shows
// PAGES, not "binary files are not supported".
// ============================================================================

pub struct DocumentView {
    filename: SharedString,
    /// Rendered page PNGs, in page order (engine render-cache).
    pages: Vec<PathBuf>,
    focus_handle: FocusHandle,
}

impl EventEmitter<()> for DocumentView {}

impl Focusable for DocumentView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl workspace::Item for DocumentView {
    type Event = ();

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        self.filename.clone()
    }
}

impl Render for DocumentView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut pages = v_flex().gap_4().p_4().items_center().w_full();
        for (i, page) in self.pages.iter().enumerate() {
            pages = pages.child(
                gpui::img(page.clone())
                    .id(("page", i))
                    .max_w_full()
                    .shadow_md(),
            );
        }
        div()
            .id("document-pages")
            .track_focus(&self.focus_handle(cx))
            .key_context("DocumentView")
            .size_full()
            .overflow_y_scroll()
            .bg(cx.theme().colors().editor_background)
            .child(pages)
    }
}

/// Copy `src` into the staging dir (created on demand), keeping its filename.
/// Runs in the app process, which holds the user's file-access grant.
fn stage_copy(staging: &Path, src: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(staging)?;
    let name = src
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("path has no filename: {}", src.display()))?;
    let dst = staging.join(name);
    std::fs::copy(src, &dst)
        .map_err(|e| anyhow::anyhow!("could not read {} ({e})", src.display()))?;
    Ok(dst)
}

/// Append one diagnostic line to `<library>/panel.log`. Best-effort — logging
/// must never break a tool. This is the file to read when a button "did
/// nothing": every engine call and failure lands here.
fn log_line(data_dir: &Path, line: &str) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(data_dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join("panel.log"))
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{ts}] {line}");
    }
}

/// Run the engine and parse its stdout JSON. Blocking — call from a background
/// task. On non-zero exit, surface the engine's stderr message.
fn engine_json(bin: &Path, data_dir: &Path, args: &[&str]) -> Result<Value> {
    log_line(data_dir, &format!("engine {}", args.join(" ")));
    let output = std::process::Command::new(bin)
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .output()
        .map_err(|e| {
            let msg = format!("could not launch {}: {e}", bin.display());
            log_line(data_dir, &format!("ERROR {msg}"));
            anyhow::anyhow!("{msg}")
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log_line(
            data_dir,
            &format!("ERROR {} → {}", args.join(" "), stderr.trim()),
        );
        bail!("{}", stderr.trim());
    }
    let value = serde_json::from_slice(&output.stdout)?;
    Ok(value)
}

fn parse_docs(value: &Value) -> Vec<Doc> {
    value
        .get("documents")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|d| {
                    Some(Doc {
                        id: d.get("id")?.as_i64()?,
                        filename: d
                            .get("filename")
                            .and_then(|f| f.as_str())
                            .unwrap_or("(unnamed)")
                            .to_string()
                            .into(),
                        pages: d.get("pages").and_then(|p| p.as_i64()),
                        status: d
                            .get("status")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string()
                            .into(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn summarize_capabilities(value: &Value) -> SharedString {
    let flag = |key: &str| value.get(key).and_then(|v| v.as_bool()).unwrap_or(false);
    let nested = |key: &str| {
        value
            .get(key)
            .and_then(|o| o.get("available"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    let mark = |on: bool| if on { "on" } else { "—" };
    format!(
        "Voice {} · Render {} · OCR {} · Kokoro {}",
        mark(flag("say")),
        mark(flag("render_pdf")),
        mark(nested("ocr")),
        mark(nested("tts"))
    )
    .into()
}

impl Panel for ReaderPanel {
    fn persistent_name() -> &'static str {
        "ReaderPanel"
    }

    fn panel_key() -> &'static str {
        "ReaderPanel"
    }

    fn position(&self, _window: &Window, _cx: &App) -> DockPosition {
        DockPosition::Left
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right)
    }

    fn set_position(
        &mut self,
        _position: DockPosition,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    fn default_size(&self, _window: &Window, _cx: &App) -> Pixels {
        px(420.)
    }

    /// Zen PDF is the reason this build exists, so the panel is visible by
    /// default in a fresh workspace (users can still toggle it with cmd-k cmd-r).
    fn starts_open(&self, _window: &Window, _cx: &App) -> bool {
        true
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<ui::IconName> {
        Some(ui::IconName::File)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Zen PDF — Files")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        // Must be globally unique across panels (Zed panics otherwise). In use:
        // 0 agent · 1 project · 2 terminal · 3 git · 5 collab · 6 outline ·
        // 7 debugger. 4 is free and keeps Zen PDF ordered near the file panels.
        4
    }
}

impl Focusable for ReaderPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for ReaderPanel {}

impl Render for ReaderPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let border = colors.border;
        let panel_bg = colors.panel_background;
        let elevated = colors.elevated_surface_background;
        let selected_bg = colors.element_selected;
        let playing = self.playing;
        let has_audio = !self.played_chapters.is_empty();
        let n_chapters = self.played_chapters.len();

        // --- header: wordmark + quiet subtitle + Import -----------------------
        let header = v_flex()
            .gap_1()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        Label::new("Zen PDF")
                            .size(LabelSize::Large)
                            .weight(gpui::FontWeight::SEMIBOLD),
                    )
                    .child(
                        Button::new("import", "Import")
                            .style(ButtonStyle::Filled)
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.open_files(cx))),
                    ),
            )
            .child(
                Label::new("A calm reader for documents, OCR, and voice.")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );

        // --- document list ----------------------------------------------------
        let mut list = v_flex().gap_1().mt_2();
        if self.documents.is_empty() && !self.busy {
            list = list.child(
                div().py_8().px_3().child(
                    Label::new("Drop a PDF here, or click Import to begin.")
                        .color(Color::Muted),
                ),
            );
        } else {
            for doc in &self.documents {
                let is_selected = self.selected == Some(doc.id);
                let pages = doc
                    .pages
                    .map(|p| format!("{p} page{}", if p == 1 { "" } else { "s" }))
                    .unwrap_or_else(|| "—".into());
                let id = doc.id;
                let doc_name = doc.filename.clone();
                let doc_pages = doc.pages;
                list = list.child(
                    div()
                        .id(("doc", doc.id as usize))
                        .rounded_md()
                        .px_3()
                        .py_2()
                        .border_1()
                        .border_color(border)
                        .bg(if is_selected { selected_bg } else { elevated })
                        .hover(|s| s.bg(selected_bg))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.selected = Some(id);
                            // Publish the selection so the Contents panel updates.
                            cx.set_global(ZenSelection {
                                asset: Some(id),
                                filename: doc_name.clone(),
                                pages: doc_pages,
                                engine_bin: this.engine_bin.clone(),
                                data_dir: this.data_dir.clone(),
                            });
                            cx.notify();
                            // A reader shows the document you clicked.
                            this.open_document_view(window, cx);
                        }))
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(Label::new(doc.filename.clone()))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Label::new(SharedString::from(pages))
                                                .size(LabelSize::Small)
                                                .color(Color::Muted),
                                        )
                                        .child(
                                            Label::new(doc.status.clone())
                                                .size(LabelSize::Small)
                                                .color(Color::Muted),
                                        ),
                                ),
                        ),
                );
            }
        }


        // --- library search ----------------------------------------------------
        let mut search_section = v_flex()
            .gap_1()
            .mt_2()
            .pt_2()
            .border_t_1()
            .border_color(border)
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(div().flex_grow(1.).child(self.search_input.clone()))
                    .child(
                        Button::new("do-search", "Search")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.run_search(cx))),
                    ),
            );
        if !self.search_hits.is_empty() {
            let shown = self.search_hits.len().min(10);
            for hit in self.search_hits.iter().take(shown) {
                search_section = search_section.child(
                    v_flex()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(elevated)
                        .child(
                            Label::new(SharedString::from(format!(
                                "p.{} · {}",
                                hit.page, hit.filename
                            )))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        )
                        .child(Label::new(hit.snippet.clone()).size(LabelSize::Small)),
                );
            }
            if self.search_hits.len() > shown {
                search_section = search_section.child(
                    Label::new(SharedString::from(format!(
                        "…and {} more",
                        self.search_hits.len() - shown
                    )))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                );
            }
        }

        // --- in-app audio player (shown after a narration) --------------------
        let play_controls = if has_audio {
            let toggle = if playing {
                Button::new("pb-stop", "■ Stop")
                    .on_click(cx.listener(|this, _, _, cx| this.stop_playback(cx)))
            } else {
                Button::new("pb-play", "▶ Play")
                    .on_click(cx.listener(|this, _, _, cx| this.play(cx)))
            };
            Some(
                h_flex()
                    .gap_2()
                    .items_center()
                    .mt_2()
                    .child(toggle)
                    .child(
                        Label::new(SharedString::from(format!("{n_chapters} chapter(s)")))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
        } else {
            None
        };

        // --- status footer ----------------------------------------------------
        let mut footer = v_flex()
            .gap_0p5()
            .mt_2()
            .pt_2()
            .border_t_1()
            .border_color(border);
        footer = footer.child(
            Label::new(self.status.clone())
                .size(LabelSize::Small)
                .color(if self.busy { Color::Accent } else { Color::Muted }),
        );
        if let Some(caps) = &self.capabilities {
            footer = footer.child(
                Label::new(caps.clone())
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            );
        }

        div()
            .track_focus(&self.focus_handle)
            .key_context("ReaderPanel")
            .size_full()
            .bg(panel_bg)
            .p_4()
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _window, cx| {
                this.import_paths(paths.paths().to_vec(), cx);
            }))
            .child(
                v_flex()
                    .size_full()
                    .child(header)
                    .child(list)
                    .when(self.selected.is_some(), |el| el.child(self.render_tools_row(cx)))
                    .child(search_section)
                    .when_some(play_controls, |el, pc| el.child(pc))
                    .child(footer),
            )
    }
}

impl ReaderPanel {
    /// Visible, always-discoverable tool buttons for the selected document.
    /// Menu-only tools proved invisible in practice — a document app's core
    /// actions live next to the document list.
    fn render_tools_row(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_wrap()
            .gap_1()
            .mt_2()
            .child(Button::new("t-view", "View").on_click(cx.listener(
                |this, _, window, cx| this.open_document_view(window, cx),
            )))
            .child(Button::new("t-narrate", "▶ Narrate").on_click(cx.listener(
                |this, _, _, cx| this.narrate(cx),
            )))
            .child(Button::new("t-extract", "Extract text").on_click(cx.listener(
                |this, _, window, cx| this.extract_text(window, cx),
            )))
            .child(Button::new("t-ocr", "OCR").on_click(cx.listener(
                |this, _, window, cx| this.ocr_selected(window, cx),
            )))
            .child(Button::new("t-combine", "Combine").on_click(cx.listener(
                |this, _, _, cx| this.combine_all(cx),
            )))
            .child(Button::new("t-split", "Split").on_click(cx.listener(
                |this, _, _, cx| this.split_selected(cx),
            )))
            .child(Button::new("t-rotate", "Rotate").on_click(cx.listener(
                |this, _, _, cx| this.rotate_selected(cx),
            )))
    }
}

// ============================================================================
// Contents panel (right dock): the outline / structure of the selected doc.
// ============================================================================

pub struct ContentsPanel {
    focus_handle: FocusHandle,
    title: SharedString,
    pages: Option<i64>,
    headings: Vec<SharedString>,
    busy: bool,
    _selection: Subscription,
}

impl ContentsPanel {
    pub async fn load(
        _workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        cx.update(|_window, cx| {
            cx.new(|cx| {
                let subscription =
                    cx.observe_global::<ZenSelection>(|this: &mut ContentsPanel, cx| {
                        this.refresh(cx);
                    });
                let mut panel = ContentsPanel {
                    focus_handle: cx.focus_handle(),
                    title: "No document selected".into(),
                    pages: None,
                    headings: Vec::new(),
                    busy: false,
                    _selection: subscription,
                };
                panel.refresh(cx);
                panel
            })
        })
    }

    /// Reload the outline for whatever document is currently selected.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        // try_global: the global may not be set yet when the panel first loads.
        let selection = cx.try_global::<ZenSelection>().cloned().unwrap_or_default();
        self.title = if selection.filename.is_empty() {
            "No document selected".into()
        } else {
            selection.filename.clone()
        };
        self.pages = selection.pages;
        let Some(asset) = selection.asset else {
            self.headings.clear();
            cx.notify();
            return;
        };
        let (bin, dir) = (selection.engine_bin, selection.data_dir);
        self.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    engine_json(&bin, &dir, &["outline", "--asset", &asset.to_string()])
                })
                .await;
            this.update(cx, |this, cx| {
                this.busy = false;
                if let Ok(value) = result {
                    this.headings = value
                        .get("headings")
                        .and_then(|h| h.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|h| {
                                    h.get("title")
                                        .and_then(|t| t.as_str())
                                        .map(|s| SharedString::from(s.to_string()))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Panel for ContentsPanel {
    fn persistent_name() -> &'static str {
        "ZenContentsPanel"
    }

    fn panel_key() -> &'static str {
        "ZenContentsPanel"
    }

    fn position(&self, _window: &Window, _cx: &App) -> DockPosition {
        DockPosition::Right
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right)
    }

    fn set_position(
        &mut self,
        _position: DockPosition,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    fn default_size(&self, _window: &Window, _cx: &App) -> Pixels {
        px(300.)
    }

    fn starts_open(&self, _window: &Window, _cx: &App) -> bool {
        true
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<ui::IconName> {
        Some(ui::IconName::ListTree)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Zen PDF — Contents")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleContents)
    }

    fn activation_priority(&self) -> u32 {
        8
    }
}

impl Focusable for ContentsPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for ContentsPanel {}

impl Render for ContentsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let border = colors.border;

        let mut root = v_flex()
            .track_focus(&self.focus_handle)
            .key_context("ZenContentsPanel")
            .size_full()
            .bg(colors.panel_background)
            .p_4()
            .gap_1()
            .child(
                Label::new("Contents")
                    .size(LabelSize::Large)
                    .weight(gpui::FontWeight::SEMIBOLD),
            )
            .child(
                Label::new(self.title.clone())
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );

        if let Some(p) = self.pages {
            root = root.child(
                Label::new(SharedString::from(format!(
                    "{p} page{}",
                    if p == 1 { "" } else { "s" }
                )))
                .size(LabelSize::XSmall)
                .color(Color::Muted),
            );
        }

        let mut body = v_flex().gap_0p5().mt_2();
        if self.busy {
            body = body.child(Label::new("Reading structure…").color(Color::Muted));
        } else if self.headings.is_empty() {
            body = body.child(
                Label::new("No detected headings for this document.")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );
        } else {
            for heading in self.headings.iter().take(200) {
                body = body.child(
                    div()
                        .px_2()
                        .py_0p5()
                        .border_l_2()
                        .border_color(border)
                        .child(Label::new(heading.clone()).size(LabelSize::Small)),
                );
            }
        }

        root.child(body)
    }
}
