use serde::{Deserialize, Serialize};

use crate::protocol::effect::AppEffect;
use crate::protocol::schema::FrameId;
use crate::protocol::view::View;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum AppOutput {
    View(View),
    Effect(AppEffect),
    Log(AppLog),
    FrameDone { frame_id: FrameId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLog {
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[cfg(test)]
mod tests {
    use crate::protocol::output::{AppOutput, LogLevel};
    use crate::protocol::view::{DocumentNode, DocumentStyle, InlineText, View};

    #[test]
    fn output_round_trip_document_view() {
        let out = AppOutput::View(View::Document {
            nodes: vec![DocumentNode::Paragraph {
                content: InlineText {
                    text: "hello".to_string(),
                },
            }],
            style: DocumentStyle::default(),
        });

        let json = serde_json::to_string(&out).expect("serialize");
        let decoded: AppOutput = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(decoded, AppOutput::View(View::Document { .. })));
        assert!(!matches!(decoded, AppOutput::Log(log) if log.level == LogLevel::Error));
    }
}
