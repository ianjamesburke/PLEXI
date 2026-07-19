use crate::ui::{style, syntax::SyntaxHighlighter, theme::Colors};

pub(crate) struct ReadOnlyCodeViewer<'a> {
    id: egui::Id,
    code: &'a str,
    language: &'a str,
    line_numbers: bool,
    max_height: Option<f32>,
}

impl<'a> ReadOnlyCodeViewer<'a> {
    pub(crate) fn new(id: egui::Id, code: &'a str, language: &'a str) -> Self {
        Self {
            id,
            code,
            language,
            line_numbers: false,
            max_height: None,
        }
    }

    pub(crate) fn line_numbers(mut self, line_numbers: bool) -> Self {
        self.line_numbers = line_numbers;
        self
    }

    pub(crate) fn max_height(mut self, max_height: f32) -> Self {
        self.max_height = Some(max_height);
        self
    }

    pub(crate) fn show(self, ui: &mut egui::Ui, colors: &Colors) -> egui::Response {
        let code = if self.line_numbers {
            numbered_code(self.code)
        } else {
            self.code.to_string()
        };
        let language = self.language;
        let id = self.id;
        let max_height = self.max_height.unwrap_or(220.0);

        egui::Frame::new()
            .fill(colors.bg_active)
            .stroke(egui::Stroke::new(1.0_f32, colors.border))
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt(id)
                    .max_height(max_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let job = SyntaxHighlighter::highlight(
                            ui.ctx(),
                            ui.style(),
                            &code,
                            language,
                            colors,
                        );
                        ui.add(egui::Label::new(job).selectable(true))
                    })
                    .inner
            })
            .inner
    }
}

fn numbered_code(code: &str) -> String {
    let line_count = code.lines().count().max(1);
    let width = line_count.to_string().len();
    let mut out = String::with_capacity(code.len() + line_count * (width + 3));
    for (idx, line) in code.lines().enumerate() {
        use std::fmt::Write as _;
        let _ = writeln!(&mut out, "{:>width$}  {line}", idx + 1, width = width);
    }
    if code.ends_with('\n') {
        out
    } else {
        out.trim_end_matches('\n').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::numbered_code;

    #[test]
    fn numbered_code_aligns_line_numbers() {
        let numbered = numbered_code("one\ntwo\nthree");
        assert_eq!(numbered, "1  one\n2  two\n3  three");
    }
}
