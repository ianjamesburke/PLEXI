use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum View {
    Canvas {
        primitives: Vec<CanvasPrimitive>,
    },
    Document {
        nodes: Vec<DocumentNode>,
        style: DocumentStyle,
    },
    Media {
        nodes: Vec<MediaNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CanvasPrimitive {
    Rect {
        rect: Rect,
        fill: Color,
        radius: f32,
    },
    Text {
        pos: Point,
        text: String,
        style: TextStyle,
    },
    Line {
        from: Point,
        to: Point,
        stroke: Stroke,
    },
    Image {
        rect: Rect,
        source: ImageSource,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocumentNode {
    Heading {
        level: u8,
        text: InlineText,
    },
    Paragraph {
        content: InlineText,
    },
    List {
        ordered: bool,
        items: Vec<InlineText>,
    },
    CodeBlock {
        language: Option<String>,
        text: String,
    },
    Preformatted {
        text: String,
    },
    Divider,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentStyle {
    pub wrap_text: bool,
}

impl Default for DocumentStyle {
    fn default() -> Self {
        Self { wrap_text: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InlineText {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaNode {
    pub kind: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub width: f32,
    pub color: Color,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub size: f32,
    pub color: Color,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageSource {
    File { path: String },
    Url { url: String },
}

#[cfg(test)]
mod tests {
    use crate::protocol::view::{DocumentNode, DocumentStyle, InlineText, View};

    #[test]
    fn document_view_serializes_with_tagged_type() {
        let view = View::Document {
            nodes: vec![DocumentNode::Heading {
                level: 1,
                text: InlineText {
                    text: "Title".to_string(),
                },
            }],
            style: DocumentStyle::default(),
        };
        let json = serde_json::to_value(&view).expect("serialize");
        assert_eq!(json.get("type").and_then(|v| v.as_str()), Some("document"));
    }
}
