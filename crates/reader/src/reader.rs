//! Rust Reader as a first-class Zed panel — the fork-and-extend entry point.
//!
//! This is the minimal integration milestone: a dockable panel registered in
//! the Zed workspace, proving the path. Document viewing, the bundle tree,
//! Edit-bucket mutations, and git-diff hang off Zed's own machinery from here
//! (editor / multi_buffer / git_ui / project_panel) rather than a from-scratch
//! gpui shell.

use anyhow::Result;
use gpui::{
    Action, App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, Styled, WeakEntity, Window,
    actions, div, px,
};
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

pub struct ReaderPanel {
    focus_handle: FocusHandle,
}

impl ReaderPanel {
    pub async fn load(
        _workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        cx.update(|_window, cx| {
            cx.new(|cx| ReaderPanel {
                focus_handle: cx.focus_handle(),
            })
        })
    }
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
        px(420.)
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<ui::IconName> {
        Some(ui::IconName::File)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Rust Reader")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        5
    }
}

impl Focusable for ReaderPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for ReaderPanel {}

impl Render for ReaderPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .p_4()
            .child("Rust Reader — panel placeholder (fork-and-extend Zed)")
    }
}
