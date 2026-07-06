//! Zen PDF — Rust Reader as a first-class Zed panel.
//!
//! A calm document workspace docked in Zed. The panel is a thin client: it
//! drives the offline engine (`reader-engined`) over a subprocess boundary and
//! renders the results. That boundary is deliberate — it keeps the engine's
//! SQLite out of Zed's link graph (avoiding the libsqlite3-sys conflict) and
//! lets the same backend serve the CLI / API / Docker surfaces later.
//!
//! Milestone: import documents and list them from the real engine. Page
//! viewing, Edit-bucket mutations, and git-diff hang off Zed's own machinery
//! (editor / multi_buffer / git_ui) from here.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use gpui::{
    Action, App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, PathPromptOptions, Pixels, Render, SharedString,
    Styled, WeakEntity, Window, actions, div, px,
};
use serde_json::Value;
use ui::prelude::*;
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

actions!(reader, [ToggleFocus]);

/// Register the Reader panel's actions on every workspace.
pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<ReaderPanel>(window, cx);
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

pub struct ReaderPanel {
    focus_handle: FocusHandle,
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
}

impl ReaderPanel {
    pub async fn load(
        _workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        cx.update(|_window, cx| {
            cx.new(|cx| {
                let mut panel = ReaderPanel {
                    focus_handle: cx.focus_handle(),
                    engine_bin: resolve_engine_bin(),
                    data_dir: resolve_data_dir(),
                    documents: Vec::new(),
                    selected: None,
                    status: "Starting the engine…".into(),
                    busy: false,
                    capabilities: None,
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
    fn import_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
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
                    let mut imported = 0usize;
                    let mut last_err: Option<String> = None;
                    for path in &paths {
                        let file = path.to_string_lossy().to_string();
                        match engine_json(&bin, &dir, &["import", "--file", &file]) {
                            Ok(_) => imported += 1,
                            Err(e) => last_err = Some(format!("{e}")),
                        }
                    }
                    (imported, last_err)
                })
                .await;
            this.update(cx, |this, cx| {
                let (imported, last_err) = outcome;
                if let Some(err) = last_err {
                    this.status = format!("Imported {imported}/{n} · {err}").into();
                } else {
                    this.status = format!("Imported {imported} document(s).").into();
                }
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }
}

/// Resolve the backend binary: `$READER_ENGINED`, else `reader-engined` on PATH.
fn resolve_engine_bin() -> PathBuf {
    std::env::var_os("READER_ENGINED")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("reader-engined"))
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

/// Run the engine and parse its stdout JSON. Blocking — call from a background
/// task. On non-zero exit, surface the engine's stderr JSON error.
fn engine_json(bin: &Path, data_dir: &Path, args: &[&str]) -> Result<Value> {
    let output = std::process::Command::new(bin)
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("could not launch {}: {e}", bin.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
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
    let say = value.get("say").and_then(|v| v.as_bool()).unwrap_or(false);
    let render = value
        .get("render_pdf")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let ocr = value
        .get("ocr")
        .and_then(|o| o.get("available"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let kokoro = value
        .get("tts")
        .and_then(|t| t.get("available"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mark = |on: bool| if on { "on" } else { "—" };
    format!(
        "Voice {} · Render {} · OCR {} · Kokoro {}",
        mark(say),
        mark(render),
        mark(ocr),
        mark(kokoro)
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
        px(440.)
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
        Some("Zen PDF")
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
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = Some(id);
                            cx.notify();
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

        // --- status footer ----------------------------------------------------
        let mut footer = v_flex().gap_0p5().mt_2().pt_2().border_t_1().border_color(border);
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
            .child(v_flex().size_full().child(header).child(list).child(footer))
    }
}
