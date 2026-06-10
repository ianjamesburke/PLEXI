use egui::Id;

const OVERLAY_FOCUS_REGISTRY_ID: &str = "plexi_overlay_focus_registry";

/// Register an overlay field that should reclaim focus after CentralPanel.
///
/// Text-owning overlays render before panes, while app panes render during
/// CentralPanel and may call `request_focus()` later in the same frame. The
/// renderer drains this registry after CentralPanel so host overlay fields win
/// egui's last-writer-wins focus contest without every overlay hard-coding a
/// second focus path.
pub(crate) fn register_overlay_focus(ctx: &egui::Context, id: Id) {
    let registry_id = Id::new(OVERLAY_FOCUS_REGISTRY_ID);
    ctx.memory_mut(|memory| {
        let mut ids: Vec<Id> = memory.data.get_temp(registry_id).unwrap_or_default();
        if !ids.contains(&id) {
            ids.push(id);
        }
        memory.data.insert_temp(registry_id, ids);
    });
}

pub(crate) fn drain_overlay_focus(ctx: &egui::Context) -> Vec<Id> {
    let registry_id = Id::new(OVERLAY_FOCUS_REGISTRY_ID);
    ctx.memory_mut(|memory| {
        let ids = memory.data.get_temp(registry_id).unwrap_or_default();
        memory.data.remove::<Vec<Id>>(registry_id);
        ids
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_focus_registry_dedupes_and_drains_in_order() {
        let ctx = egui::Context::default();
        let first = Id::new("first_overlay_field");
        let second = Id::new("second_overlay_field");

        register_overlay_focus(&ctx, first);
        register_overlay_focus(&ctx, first);
        register_overlay_focus(&ctx, second);

        assert_eq!(drain_overlay_focus(&ctx), vec![first, second]);
        assert!(drain_overlay_focus(&ctx).is_empty());
    }
}
