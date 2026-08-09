use appkit_shell::{ProductHost, ShellEffect};

use crate::product::{NotoraProduct, NotoraProductEvent};

/// 一次 product inbox drain 的有序、强类型结果。
pub(super) struct ProductCompletions {
    pub(super) shell_effect: ShellEffect,
    pub(super) events: Vec<NotoraProductEvent>,
}

pub(super) struct ProductEventCoordinator;

impl ProductEventCoordinator {
    pub(super) fn drain(product: &mut NotoraProduct) -> ProductCompletions {
        let shell_effect = ProductHost::drain_product_events(product);
        let events = product.take_workspace_events();
        ProductCompletions { shell_effect, events }
    }
}
