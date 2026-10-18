pub(crate) use crate::ui::app_chrome::{footer_keys_height, AppChrome, TextLabelStyle};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeConfig;
    use crate::host::wasm_app::bindings::plexi::platform::types::FooterKeyEntry;
    use crate::ui::app_chrome::chip_row_height;
    use crate::ui::hints::{HintBar, HintGroup};
    use crate::ui::style;
    use crate::ui::theme::Colors;

    #[test]
    fn semantic_app_chrome_colors_follow_host_theme_tokens() {
        let cfg = ThemeConfig {
            bg_toolbar: Some("#010203".into()),
            border: Some("#070809".into()),
            ..ThemeConfig::default()
        };
        let colors = Colors::from_config(&cfg);
        let chrome = AppChrome::new(&colors);

        assert_eq!(chrome.toolbar_fill(), colors.bg_toolbar);
        assert_eq!(chrome.border(), colors.border);
    }

    /// #2240: footer keys pack onto one row when they fit and wrap onto
    /// additional rows in a narrow band, and the measured height grows with the
    /// wrapped row count so `Column`/bottom-pin reserves enough space.
    #[test]
    fn footer_keys_wrap_in_narrow_band() {
        let entries: Vec<FooterKeyEntry> = (0..6)
            .map(|i| FooterKeyEntry {
                keys: vec![format!("^{i}")],
                description: format!("action {i}"),
            })
            .collect();
        let groups: Vec<HintGroup<'_>> = entries
            .iter()
            .map(|e| HintGroup::from_strings(&e.keys, &e.description))
            .collect();

        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            // Wide band: everything fits on a single row.
            let wide = HintBar::new(&groups).rows(ui, 10_000.0);
            assert_eq!(wide.len(), 1, "all entries fit on one row when wide");
            assert_eq!(wide[0], 0..entries.len());

            // Narrow band: entries wrap onto multiple rows, none dropped.
            let narrow = HintBar::new(&groups).rows(ui, 90.0);
            assert!(narrow.len() > 1, "entries must wrap in a narrow band");
            let covered: usize = narrow.iter().map(|r| r.end - r.start).sum();
            assert_eq!(covered, entries.len(), "every entry appears exactly once");
            for w in narrow.windows(2) {
                assert_eq!(w[0].end, w[1].start, "rows are contiguous");
            }

            // Height reflects the wrapped row count: taller than a single row.
            let one_row = style::SPACE_SM + (chip_row_height(ui) + 4.0) + style::SPACE_SM;
            assert!(
                footer_keys_height(ui, &entries, false) >= one_row,
                "wrapped footer height must be at least one row tall"
            );
        });
    }
}
