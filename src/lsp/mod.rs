pub mod client;
mod protocol;

pub use client::{
    code_source_label, lang_id_for_extension, Diagnostic, LspClient, LspEvent, LspProgress,
    Severity,
};
