use crate::app::App;
use crate::app_effect::AppEffect;
use ui::search_bar::SearchBarAction;

impl App {
    pub(crate) fn dispatch_search_action(&mut self, action: SearchBarAction) -> AppEffect {
        match action {
            SearchBarAction::Replace => {
                self.perform_replace();
            }
            SearchBarAction::ReplaceAll => {
                self.perform_replace_all();
            }
            _ => {
                let needs_search = self.apply_search_bar_action(&action);
                if needs_search {
                    self.perform_search_for_active_doc();
                }
            }
        }
        AppEffect::REDRAW
    }
}
