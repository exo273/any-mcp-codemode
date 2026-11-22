//! Tool catalog: name sanitizing, JSON Schema → TypeScript declarations, search.
//! Behavior mirrors Cloudflare `utils.ts` + `json-schema-types.ts`, plus `outputSchema`.

use serde_json::{Map, Value};
use std::fmt::Write as _;

/// One upstream tool as seen by the sandbox.
#[derive(Debug, Clone)]
pub struct Tool {
    /// Upstream server key from the config.
    pub server: String,
    /// Original MCP tool name (used on the wire).
    pub name: String,
    /// Sanitized JS identifier (used in the sandbox).
    pub ident: String,
    /// Tool description.
    pub description: String,
    /// JSON Schema of the input.
    pub input: Value,
    /// JSON Schema of the output, if the upstream declares one.
    pub output: Option<Value>,
}

const RESERVED: &[&str] = &[
    "abstract",
    "arguments",
    "await",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "double",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "function",
    "goto",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "int",
    "interface",
    "let",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "volatile",
    "while",
    "with",
    "yield",
];

/// Turn any tool/server name into a valid JS identifier (Cloudflare `sanitizeToolName`).
#[must_use]
pub fn sanitize(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if matches!(c, '-' | '.' | ' ' | '\t') {
                '_'
            } else {
                c
            }
        })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    if s.is_empty() {
        return "_".into();
    }
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    if RESERVED.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

fn pascal(ident: &str) -> String {
    ident
        .split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            c.next().map_or_else(String::new, |f| {
                f.to_ascii_uppercase().to_string() + c.as_str()
            })
        })
        .collect::<String>()
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .to_string()
}

fn quote_prop(name: &str) -> String {
    let ident = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if ident {
        name.to_string()
    } else {
        Value::String(name.into()).to_string()
    }
}

fn doc(text: &str) -> String {
    text.trim()
        .replace("\r\n", " ")
        .replace('\n', " ")
        .replace("*/", "*\\/")
}

/// Convert a JSON Schema into a TypeScript type expression.
#[must_use]
pub fn ts_type(schema: &Value, root: &Value, depth: usize) -> String {
    if depth > 20 {
        return "unknown".into();
    }
    let Some(o) = schema.as_object() else {
        return if schema == &Value::Bool(false) {
            "never".into()
        } else {
            "unknown".into()
        };
    };
    let nullable = o.get("nullable") == Some(&Value::Bool(true));
    let t = ts_inner(o, root, depth);
    if nullable && t != "unknown" {
        format!("{t} | null")
    } else {
        t
    }
}

fn ts_inner(o: &Map<String, Value>, root: &Value, depth: usize) -> String {
    let next = |s: &Value| ts_type(s, root, depth + 1);
    if let Some(r) = o.get("$ref").and_then(Value::as_str) {
        let target = if r == "#" {
            Some(root)
        } else {
            r.strip_prefix('#').and_then(|p| root.pointer(p))
        };
        return target.map_or_else(|| "unknown".into(), next);
    }
    for (key, sep) in [("anyOf", " | "), ("oneOf", " | "), ("allOf", " & ")] {
        if let Some(list) = o.get(key).and_then(Value::as_array) {
            return list.iter().map(next).collect::<Vec<_>>().join(sep);
        }
    }
    if let Some(list) = o.get("enum").and_then(Value::as_array) {
        return if list.is_empty() {
            "never".into()
        } else {
            list.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(" | ")
        };
    }
    if let Some(c) = o.get("const") {
        return c.to_string();
    }
    match &o.get("type") {
        Some(Value::Array(types)) => types
            .iter()
            .map(|t| ts_inner(&single_type(o, t), root, depth))
            .collect::<Vec<_>>()
            .join(" | "),
        Some(Value::String(t)) => match t.as_str() {
            "string" => "string".into(),
            "number" | "integer" => "number".into(),
            "boolean" => "boolean".into(),
            "null" => "null".into(),
            "array" => match o.get("items") {
                Some(Value::Array(tuple)) => format!(
                    "[{}]",
                    tuple.iter().map(next).collect::<Vec<_>>().join(", ")
                ),
                Some(items) => format!("{}[]", wrap(&next(items))),
                None => "unknown[]".into(),
            },
            _ => ts_object(o, root, depth),
        },
        _ if o.contains_key("properties") => ts_object(o, root, depth),
        _ => "unknown".into(),
    }
}

fn single_type(o: &Map<String, Value>, t: &Value) -> Map<String, Value> {
    let mut m = o.clone();
    m.insert("type".into(), t.clone());
    m
}

fn wrap(t: &str) -> String {
    if t.contains(' ') {
        format!("({t})")
    } else {
        t.to_string()
    }
}

fn ts_object(o: &Map<String, Value>, root: &Value, depth: usize) -> String {
    let required: Vec<&str> = o
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut out = String::from("{ ");
    if let Some(props) = o.get("properties").and_then(Value::as_object) {
        for (k, v) in props {
            if let Some(d) = v.get("description").and_then(Value::as_str) {
                let _ = write!(out, "/** {} */ ", doc(d));
            }
            let opt = if required.contains(&k.as_str()) {
                ""
            } else {
                "?"
            };
            let _ = write!(
                out,
                "{}{opt}: {}; ",
                quote_prop(k),
                ts_type(v, root, depth + 1)
            );
        }
    }
    match o.get("additionalProperties") {
        Some(Value::Object(_)) => {
            let _ = write!(
                out,
                "[key: string]: {}; ",
                ts_type(&o["additionalProperties"], root, depth + 1)
            );
        }
        Some(Value::Bool(true)) => out.push_str("[key: string]: unknown; "),
        _ => {}
    }
    if out == "{ " {
        "Record<string, unknown>".into()
    } else {
        out + "}"
    }
}

/// Generate `declare const codemode` TypeScript for the given tools.
/// `flat` = also expose tools at `codemode.<tool>` (single-server mode).
#[must_use]
pub fn declarations(tools: &[Tool], flat: bool) -> String {
    let mut types = String::new();
    let mut by_server: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    for t in tools {
        let base = pascal(&format!("{}_{}", sanitize(&t.server), t.ident));
        let _ = writeln!(
            types,
            "type {base}Input = {};",
            ts_type(&t.input, &t.input, 0)
        );
        let out = t
            .output
            .as_ref()
            .map_or_else(|| "unknown".into(), |o| ts_type(o, o, 0));
        let _ = writeln!(types, "type {base}Output = {out};");
        let _ = write!(
            by_server.entry(&t.server).or_default(),
            "    /** {} */\n    {}: (input: {base}Input) => Promise<{base}Output>;\n",
            doc(if t.description.is_empty() {
                &t.name
            } else {
                &t.description
            }),
            t.ident
        );
    }
    let mut decl = String::from("declare const codemode: {\n");
    for (server, methods) in &by_server {
        if flat {
            decl.push_str(methods);
        }
        let _ = write!(decl, "  {}: {{\n{methods}  }};\n", sanitize(server));
    }
    decl.push_str("  /** Search tools by keywords. */\n  search: (query: string) => Promise<{ server: string; tool: string; description: string }[]>;\n");
    decl.push_str("  /** TypeScript declaration of one tool: \"server.tool\". */\n  describe: (target: string) => Promise<string>;\n};");
    format!("{types}\n{decl}")
}

/// Rank tools by keyword hits in name + description (case-insensitive). Max 50 results.
#[must_use]
pub fn search<'a>(tools: &'a [Tool], query: &str) -> Vec<&'a Tool> {
    let words: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(String::from)
        .collect();
    let mut hits: Vec<(usize, &Tool)> = tools
        .iter()
        .filter_map(|t| {
            let hay =
                format!("{} {} {} {}", t.server, t.name, t.ident, t.description).to_lowercase();
            let score = words.iter().filter(|w| hay.contains(w.as_str())).count();
            (score > 0 || words.is_empty()).then_some((score, t))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.ident.cmp(&b.1.ident)));
    hits.into_iter().take(50).map(|(_, t)| t).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitize_matches_cloudflare_rules() {
        assert_eq!(sanitize("get-issue.v2 now"), "get_issue_v2_now");
        assert_eq!(sanitize("3d"), "_3d");
        assert_eq!(sanitize("delete"), "delete_");
        assert_eq!(sanitize("ñ!"), "_");
        assert_eq!(sanitize(""), "_");
    }

    #[test]
    fn ts_types_cover_common_schemas() {
        let s = json!({
            "type": "object", "required": ["owner"],
            "properties": {
                "owner": {"type": "string", "description": "repo owner"},
                "n": {"type": ["integer", "null"]},
                "state": {"enum": ["open", "closed"]},
                "tags": {"type": "array", "items": {"$ref": "#/$defs/tag"}},
                "x-y": {"anyOf": [{"type": "string"}, {"type": "boolean"}]}
            },
            "$defs": {"tag": {"type": "string"}}
        });
        assert_eq!(
            ts_type(&s, &s, 0),
            r#"{ /** repo owner */ owner: string; n?: number | null; state?: "open" | "closed"; tags?: string[]; "x-y"?: string | boolean; }"#
        );
    }

    #[test]
    fn declarations_include_output_schema_and_flat_mode() {
        let t = Tool {
            server: "gh".into(),
            name: "get-issue".into(),
            ident: "get_issue".into(),
            description: "Get */ issue".into(),
            input: json!({"type":"object","properties":{}}),
            output: Some(
                json!({"type":"object","properties":{"id":{"type":"number"}},"required":["id"]}),
            ),
        };
        let d = declarations(&[t], true);
        assert!(
            d.contains("type GhGetIssueOutput = { id: number; };"),
            "{d}"
        );
        assert!(d.contains("Get *\\/ issue"));
        assert_eq!(
            d.matches("get_issue: (input: GhGetIssueInput)").count(),
            2,
            "flat + namespaced"
        );
    }

    #[test]
    fn search_ranks_by_hits() {
        let mk = |n: &str, d: &str| Tool {
            server: "s".into(),
            name: n.into(),
            ident: sanitize(n),
            description: d.into(),
            input: json!({}),
            output: None,
        };
        let tools = [
            mk("list_issues", "list repo issues"),
            mk("create_issue", "create an issue"),
            mk("ping", "health"),
        ];
        let r: Vec<_> = search(&tools, "list issue")
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(r, ["list_issues", "create_issue"]);
    }
}
