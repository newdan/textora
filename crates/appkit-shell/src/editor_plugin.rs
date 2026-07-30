//! EditorPlugin — 标准代码编辑器插件（Phase 1 stub 实现）。

use core::document::DocView;
use shaping::Shaper;
use ui::core::geom::Rect;
use ui::core::paint::DrawList;
use ui::plugin::{PluginFactory, ViewPlugin};
use ui::theme::Theme;

/// 标准编辑器插件。Phase 1 仅作为占位 stub；
/// 实际渲染逻辑将在后续 phase 从 workspace/app 层迁移到此处。
pub struct EditorPlugin {
    line_height: f32,
}

impl EditorPlugin {
    pub fn new() -> Self {
        Self { line_height: 16.0 }
    }
}

impl Default for EditorPlugin {
    fn default() -> Self {
        Self::new()
    }
}

pub struct EditorPluginFactory;

impl PluginFactory for EditorPluginFactory {
    fn name(&self) -> &str {
        ui::plugin::PLUGIN_EDITOR
    }

    fn can_handle(&self, _path: Option<&std::path::Path>) -> bool {
        false
    }

    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(EditorPlugin::new())
    }
}

impl ViewPlugin for EditorPlugin {
    fn name(&self) -> &str {
        "editor"
    }

    fn render(
        &mut self,
        _doc: &dyn DocView,
        _bounds: Rect,
        _theme: &Theme,
        _shaper: &mut Shaper,
        _dpi_scale: f32,
    ) -> DrawList {
        // Phase 1 stub: 返回空 DrawList，渲染仍由 app 层处理。
        DrawList::new()
    }

    fn shows_cursor(&self) -> bool {
        true
    }

    fn shows_gutter(&self) -> bool {
        true
    }

    fn handle_message(
        &mut self,
        msg: ui::plugin::PluginMessage,
        _doc: &mut dyn core::document::DocViewMut,
    ) -> bool {
        if let ui::plugin::PluginMessage::SetRenderSettings { line_height, .. } = msg {
            self.line_height = line_height;
        }
        false
    }

    fn query(
        &self,
        _query: ui::plugin::PluginQuery,
        _doc: &dyn DocView,
    ) -> ui::plugin::PluginResponse {
        ui::plugin::PluginResponse::None
    }
}

#[cfg(test)]
mod tests {
    use super::{EditorPlugin, EditorPluginFactory};
    use ui::plugin::PluginFactory;

    #[test]
    fn default_editor_plugin_matches_new_state() {
        let default_plugin = EditorPlugin::default();
        let new_plugin = EditorPlugin::new();

        assert_eq!(default_plugin.line_height, new_plugin.line_height);
    }

    #[test]
    fn editor_factory_does_not_preempt_path_specific_factories() {
        let factory = EditorPluginFactory;

        assert!(!factory.can_handle(Some(std::path::Path::new("notes.md"))));
        assert!(!factory.can_handle(None));
    }
}
