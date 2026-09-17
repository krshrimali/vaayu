pub mod client;
mod protocol;

pub use client::{lang_id_for_extension, Diagnostic, LspClient, LspEvent, LspProgress, Severity};
