//! Bundled [SchemaStore](https://www.schemastore.org) catalog data, so
//! `json`/`yaml` language servers get sensible `$schema`-driven validation
//! and completion for well-known config files (`package.json`,
//! `tsconfig.json`, GitHub Actions workflows, ...) without the editor
//! making a network call at startup to fetch the catalog itself -- the
//! two files under `assets/schemastore/` are a one-time, offline-derived
//! slice of the real catalog (just `fileMatch`/`url`, description and
//! version-pinned URLs dropped), embedded into the binary via
//! `include_str!` and parsed once per process.
use serde_json::Value;
use std::sync::OnceLock;

const JSON_SCHEMAS_RAW: &str = include_str!("../assets/schemastore/json_schemas.json");
const YAML_SCHEMAS_RAW: &str = include_str!("../assets/schemastore/yaml_schemas.json");

fn json_schemas() -> &'static Value {
    static CELL: OnceLock<Value> = OnceLock::new();
    CELL.get_or_init(|| serde_json::from_str(JSON_SCHEMAS_RAW).unwrap_or(Value::Array(Vec::new())))
}
fn yaml_schemas() -> &'static Value {
    static CELL: OnceLock<Value> = OnceLock::new();
    CELL.get_or_init(|| {
        serde_json::from_str(YAML_SCHEMAS_RAW).unwrap_or(Value::Object(Default::default()))
    })
}

/// The bundled schema associations for `lang`, in the exact shape its own
/// language server expects as a `workspace/configuration` value --
/// `vscode-json-language-server`'s `json.schemas` is `[{fileMatch, url}]`,
/// `yaml-language-server`'s `yaml.schemas` is `{url: [fileMatch, ...]}` --
/// so this can be used as-is as that server's default `settings`, with the
/// user's own `[lsp.*].settings` merged on top (and winning on conflicts)
/// by the caller. `None` for any other language -- there's nothing to add.
pub fn default_settings(lang: &str) -> Option<Value> {
    match lang {
        "json" => Some(serde_json::json!({"json": {"schemas": json_schemas()}})),
        "yaml" => Some(serde_json::json!({"yaml": {"schemas": yaml_schemas()}})),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_catalogs_parse_and_are_not_tiny() {
        // Not asserting an exact count (the upstream catalog changes over
        // time) -- just that the embedded files are real, parsed data,
        // not an empty placeholder that silently stopped doing anything.
        let json = json_schemas().as_array().expect("json_schemas is an array");
        assert!(
            json.len() > 500,
            "expected a real bundled catalog, got {} entries",
            json.len()
        );
        let yaml = yaml_schemas()
            .as_object()
            .expect("yaml_schemas is an object");
        assert!(
            yaml.len() > 500,
            "expected a real bundled catalog, got {} entries",
            yaml.len()
        );
    }
    #[test]
    fn package_json_is_covered() {
        let json = json_schemas().as_array().unwrap();
        assert!(
            json.iter().any(|s| s["fileMatch"]
                .as_array()
                .is_some_and(|fm| fm.iter().any(|p| p == "package.json"))),
            "package.json should be one of the bundled associations"
        );
    }
    #[test]
    fn default_settings_shapes_match_each_servers_own_config_convention() {
        let j = default_settings("json").unwrap();
        assert!(j["json"]["schemas"].is_array());
        let y = default_settings("yaml").unwrap();
        assert!(y["yaml"]["schemas"].is_object());
        assert!(default_settings("rust").is_none());
    }
}
