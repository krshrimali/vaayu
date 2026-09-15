pub mod client;
mod protocol;

pub use client::{lang_id_for_extension, CompletionResultItem, Diagnostic, LspClient, LspEvent, Severity};
