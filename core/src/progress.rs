//! A small progress-reporting seam so `core` can report download/step progress
//! to a UI (the desktop bridges it to a Tauri event) without any UI dependency.

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ProgressEvent {
    /// A coarse phase change.
    Phase { label: String },
    /// Component/step progress: `done` of `total`, current item `label`.
    Step { done: usize, total: usize, label: String },
    /// Byte progress for the current file. `total == 0` means unknown.
    Bytes { current: u64, total: u64 },
}

pub trait ProgressSink: Send + Sync {
    fn emit(&self, ev: ProgressEvent);
}

/// No-op sink for the CLI and tests.
pub struct NullProgress;
impl ProgressSink for NullProgress {
    fn emit(&self, _ev: ProgressEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_serializes_with_kind_tag() {
        let j = serde_json::to_string(&ProgressEvent::Bytes { current: 5, total: 10 }).unwrap();
        assert_eq!(j, r#"{"kind":"bytes","current":5,"total":10}"#);
        let s = serde_json::to_string(&ProgressEvent::Step { done: 1, total: 3, label: "php".into() }).unwrap();
        assert_eq!(s, r#"{"kind":"step","done":1,"total":3,"label":"php"}"#);
    }

    #[test]
    fn null_sink_is_noop() {
        NullProgress.emit(ProgressEvent::Phase { label: "x".into() }); // must not panic
    }
}
