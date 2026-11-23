use crate::app::App;
use crate::app_effect::AppEffect;
use crate::app_search::SearchReplaceScope;
use ui::search_bar::SearchBarAction;

impl App {
    pub(crate) fn dispatch_search_action(&mut self, action: SearchBarAction) -> AppEffect {
        let effect = match action {
            SearchBarAction::Replace => {
                self.replace_search_matches(SearchReplaceScope::ActiveMatch)
            }
            SearchBarAction::ReplaceAll => {
                self.replace_search_matches(SearchReplaceScope::AllMatches)
            }
            _ => {
                let needs_search = self.apply_search_bar_action(&action);
                if needs_search {
                    self.perform_search_for_active_doc();
                }
                AppEffect::NONE
            }
        };
        effect.merge(AppEffect::REDRAW)
    }
}
