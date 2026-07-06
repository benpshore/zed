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
    Focusable, InteractiveElement, IntoElement, ParentElement, PathPromptOptions, Pixels, Render,
    SharedString, Styled, WeakEntity, Window, actions, div, px,
};
use serde_json::Value;
use ui::prelude::*;
use workspace::{
    OpenOptions, Workspace,
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
}

impl ReaderPanel {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        cx.update(|_window, cx| {
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

    // --- contextual tools (operate on the selected document) ----------------

    /// Extract the text layer to a sidecar and open it as a Zed tab.
    fn extract_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(asset) = self.selected else { return };
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
                    engine_json(&bin, &dir, &["doc-text", "--asset", &a, "--out", &o])
                })
                .await;
            let ok = result.is_ok();
            this.update(cx, |this, cx| {
                this.busy = false;
                this.status = match &result {
                    Ok(v) => {
                        let chars = v.get("chars").and_then(|c| c.as_i64()).unwrap_or(0);
                        format!("Extracted {chars} characters — opening…").into()
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

    /// Render page 1 of the selected document and open the PNG as a Zed tab.
    fn view_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(asset) = self.selected else { return };
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
        let Some(asset) = self.selected else { return };
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
                        let n = v
                            .get("chapters")
                            .and_then(|c| c.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0);
                        format!("Narrated {n} chapter(s) — revealed in Finder ({shown})").into()
                    }
                    Err(e) => format!("Narrate failed: {e}").into(),
                };
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
        let Some(asset) = self.selected else { return };
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
        let Some(asset) = self.selected else { return };
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

    fn begin(&mut self, msg: &str, cx: &mut Context<Self>) {
        self.busy = true;
        self.status = msg.to_string().into();
        cx.notify();
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
/// task. On non-zero exit, surface the engine's stderr message.
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
        let has_selection = self.selected.is_some();
        let ocr_available = self.ocr_available;

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

        // --- contextual tools (only when a document is selected) --------------
        let toolbar = if has_selection {
            let ocr_label = if ocr_available {
                "OCR"
            } else {
                "OCR (needs models)"
            };
            Some(
                v_flex()
                    .gap_1()
                    .mt_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(border)
                    .child(
                        Label::new("Tools")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(Button::new("t-extract", "Extract text").on_click(
                                cx.listener(|this, _, window, cx| this.extract_text(window, cx)),
                            ))
                            .child(Button::new("t-view", "View page").on_click(cx.listener(
                                |this, _, window, cx| this.view_page(window, cx),
                            )))
                            .child(
                                Button::new("t-narrate", "Narrate").on_click(
                                    cx.listener(|this, _, _, cx| this.narrate(cx)),
                                ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(
                                Button::new("t-combine", "Combine").on_click(
                                    cx.listener(|this, _, _, cx| this.combine_all(cx)),
                                ),
                            )
                            .child(
                                Button::new("t-split", "Split").on_click(
                                    cx.listener(|this, _, _, cx| this.split_selected(cx)),
                                ),
                            )
                            .child(Button::new("t-rotate", "Rotate 90°").on_click(
                                cx.listener(|this, _, _, cx| this.rotate_selected(cx)),
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(Button::new("t-ocr", ocr_label).disabled(!ocr_available))
                            .child(Button::new("t-compress", "Compress (soon)").disabled(true))
                            .child(Button::new("t-deskew", "Deskew (soon)").disabled(true)),
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
                    .when_some(toolbar, |el, tb| el.child(tb))
                    .child(footer),
            )
    }
}
