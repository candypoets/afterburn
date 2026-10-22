//! Canvas comments for the `send` method.
//!
//! Send prompts an agent with the comments a client collected on the canvas. Unlike `open` and
//! `init` it never publishes or opens a document. This module validates the request payload and
//! renders it as the plain text the agent receives.

use std::fmt::Write as _;

use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// The most comments one `send` request may carry.
pub const MAX_COMMENTS: usize = 50;

/// The most characters of comment content (text plus node and page labels) one request may carry.
pub const MAX_COMMENT_CHARS: usize = 20_000;

/// The most characters one free-form prompt may carry.
pub const MAX_PROMPT_CHARS: usize = 20_000;

/// The one-line instruction appended when comments arrive without a prompt.
pub const EMPTY_PROMPT_INSTRUCTION: &str = "Address these comments in the project document.";

/// One comment on a canvas element: the text itself plus optional selectors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    /// The comment body. Required and non-empty.
    pub text: String,
    /// Node label the comment is attached to, when known.
    pub node: Option<String>,
    /// Page label the comment is attached to, when known.
    pub page: Option<String>,
}

/// Parse the `comments` request field.
///
/// The field is a list of objects with a required string `text` and optional string `node` and
/// `page` labels. Missing or `null` comments mean an empty list; non-string fields, blank text,
/// more than [`MAX_COMMENTS`] entries, and oversized content are rejected. All of this happens
/// before the caller resolves a workspace or touches an agent.
pub fn parse_comments(raw: Option<&Value>) -> Result<Vec<Comment>> {
    let raw = match raw {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(raw) => raw,
    };
    let items = raw
        .as_array()
        .ok_or_else(|| Error::new(format!("\"comments\" must be a list, not {raw}")))?;
    if items.len() > MAX_COMMENTS {
        return Err(Error::new(format!(
            "too many comments ({}; at most {MAX_COMMENTS})",
            items.len()
        )));
    }

    let mut comments = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let object = item.as_object().ok_or_else(|| {
            Error::new(format!("comments[{index}] must be an object, not {item}"))
        })?;
        let text = match object.get("text") {
            Some(Value::String(text)) => text.trim(),
            Some(other) => {
                return Err(Error::new(format!(
                    "comments[{index}].text must be a string, not {other}"
                )))
            }
            None => {
                return Err(Error::new(format!(
                    "comments[{index}] has no \"text\" string"
                )))
            }
        };
        if text.is_empty() {
            return Err(Error::new(format!("comments[{index}].text is empty")));
        }
        comments.push(Comment {
            text: text.to_string(),
            node: optional_label(object, index, "node")?,
            page: optional_label(object, index, "page")?,
        });
    }
    validate_comments(&comments)?;
    Ok(comments)
}

/// Reject oversized comment sets that were built without [`parse_comments`].
pub fn validate_comments(comments: &[Comment]) -> Result<()> {
    if comments.len() > MAX_COMMENTS {
        return Err(Error::new(format!(
            "too many comments ({}; at most {MAX_COMMENTS})",
            comments.len()
        )));
    }

    let mut total = 0usize;
    for (index, comment) in comments.iter().enumerate() {
        if comment.text.trim().is_empty() {
            return Err(Error::new(format!("comments[{index}].text is empty")));
        }
        total += comment.text.chars().count();
        total += comment
            .node
            .as_deref()
            .map_or(0, |node| node.chars().count());
        total += comment
            .page
            .as_deref()
            .map_or(0, |page| page.chars().count());
    }
    if total > MAX_COMMENT_CHARS {
        return Err(Error::new(format!(
            "comments are too long ({total} characters; limit {MAX_COMMENT_CHARS})"
        )));
    }
    Ok(())
}

/// Reject a free-form prompt larger than [`MAX_PROMPT_CHARS`].
pub fn validate_prompt(prompt: &str) -> Result<()> {
    let length = prompt.chars().count();
    if length > MAX_PROMPT_CHARS {
        return Err(Error::new(format!(
            "\"prompt\" is too long ({length} characters; limit {MAX_PROMPT_CHARS})"
        )));
    }
    Ok(())
}

/// Render a `send` request as the text submitted to the agent.
///
/// With comments the shape is:
///
/// ```text
/// Mindmap comments for <label> (<path>):
///
/// 1. [<page>] <node>: <text>
/// 2. ...
///
/// <prompt>
/// ```
///
/// Blank sections are omitted: a missing path drops the parenthetical, a missing page or node
/// drops its selector, and an empty prompt is replaced with [`EMPTY_PROMPT_INSTRUCTION`]. With no
/// comments at all the request is just the prompt. Newlines, tabs, and backslashes inside comment
/// fields are escaped so one comment always occupies one numbered line.
pub fn format_prompt(
    label: &str,
    path: Option<&str>,
    comments: &[Comment],
    prompt: &str,
) -> Result<String> {
    validate_comments(comments)?;
    validate_prompt(prompt)?;
    let prompt = prompt.trim();

    if comments.is_empty() && prompt.is_empty() {
        return Err(Error::new(
            "send needs a prompt or at least one non-empty comment",
        ));
    }
    if comments.is_empty() {
        return Ok(prompt.to_string());
    }

    let mut out = String::new();
    out.push_str("Mindmap comments for ");
    out.push_str(&inline(label));
    if let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) {
        out.push_str(" (");
        out.push_str(&inline(path));
        out.push(')');
    }
    out.push_str(":\n\n");
    for (index, comment) in comments.iter().enumerate() {
        let _ = writeln!(out, "{}. {}", index + 1, entry(comment));
    }

    out.push('\n');
    if prompt.is_empty() {
        out.push_str(EMPTY_PROMPT_INSTRUCTION);
    } else {
        out.push_str(prompt);
    }
    Ok(out)
}

fn optional_label(
    object: &Map<String, Value>,
    index: usize,
    field: &str,
) -> Result<Option<String>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            Ok(if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            })
        }
        Some(other) => Err(Error::new(format!(
            "comments[{index}].{field} must be a string, not {other}"
        ))),
    }
}

/// One numbered comment without its number.
fn entry(comment: &Comment) -> String {
    let mut out = String::new();
    if let Some(page) = label(comment.page.as_deref()) {
        out.push('[');
        out.push_str(&page);
        out.push_str("] ");
    }
    if let Some(node) = label(comment.node.as_deref()) {
        out.push_str(&node);
        out.push_str(": ");
    }
    out.push_str(&inline(&comment.text));
    out
}

fn label(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(inline)
}

/// Keep a comment field on one line: escape the characters that would break the layout.
fn inline(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.trim().chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character.is_control() => out.push(' '),
            character => out.push(character),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn comment(text: &str, node: Option<&str>, page: Option<&str>) -> Comment {
        Comment {
            text: text.into(),
            node: node.map(str::to_owned),
            page: page.map(str::to_owned),
        }
    }

    fn error_text(value: Value) -> String {
        parse_comments(Some(&value))
            .err()
            .map(|error| error.message().to_string())
            .unwrap_or_default()
    }

    #[test]
    fn parses_text_with_optional_node_and_page() {
        let raw = json!([
            {
                "text": " This should say where overrides reset. ",
                "node": "Components",
                "page": "Mindmap"
            },
            {"text": "Second", "node": null, "page": ""},
        ]);
        let comments = parse_comments(Some(&raw)).unwrap_or_default();
        assert_eq!(
            comments,
            vec![
                comment(
                    "This should say where overrides reset.",
                    Some("Components"),
                    Some("Mindmap")
                ),
                comment("Second", None, None),
            ]
        );
    }

    #[test]
    fn missing_or_empty_comment_lists_are_allowed() {
        assert!(parse_comments(None).unwrap_or_default().is_empty());
        assert!(parse_comments(Some(&Value::Null))
            .unwrap_or_default()
            .is_empty());
        assert!(parse_comments(Some(&json!([])))
            .unwrap_or_default()
            .is_empty());
    }

    #[test]
    fn rejects_blank_text() {
        let error = error_text(json!([{"text": "   ", "node": "Components"}]));
        assert!(error.contains("comments[0].text is empty"), "{error}");
    }

    #[test]
    fn rejects_non_string_fields() {
        for raw in [
            json!([{"text": 3}]),
            json!([{"text": "x", "node": 4}]),
            json!([{"text": "x", "page": false}]),
            json!([{"text": "x"}, "not an object"]),
        ] {
            assert!(parse_comments(Some(&raw)).is_err(), "accepted {raw}");
        }
        assert!(parse_comments(Some(&json!({"text": "x"}))).is_err());
        let error = error_text(json!([{"text": "x", "node": 4}]));
        assert!(error.contains("node must be a string"), "{error}");
    }

    #[test]
    fn rejects_too_many_comments() {
        let raw = Value::Array(
            (0..=MAX_COMMENTS)
                .map(|index| json!({"text": format!("comment {index}")}))
                .collect(),
        );
        let error = error_text(raw);
        assert!(error.contains("too many comments"), "{error}");
        assert!(
            error.contains(&format!("at most {MAX_COMMENTS}")),
            "{error}"
        );
    }

    #[test]
    fn rejects_oversized_comment_content() {
        let comments = vec![comment(&"x".repeat(MAX_COMMENT_CHARS + 1), None, None)];
        let error = validate_comments(&comments)
            .err()
            .map(|error| error.message().to_string())
            .unwrap_or_default();
        assert!(error.contains("comments are too long"), "{error}");
    }

    #[test]
    fn formats_comments_with_a_prompt() {
        let comments = vec![
            comment(
                "This should say where overrides reset.",
                Some("Components"),
                Some("Mindmap"),
            ),
            comment("This has no node.", None, Some("Mindmap")),
            comment("This has no page.", Some("Components"), None),
            comment("Bare comment.", None, None),
        ];
        let formatted = format_prompt(
            "Mindmap",
            Some("/root/code/mindmap"),
            &comments,
            "Why does this rebuild?",
        )
        .unwrap_or_default();
        assert_eq!(
            formatted,
            "Mindmap comments for Mindmap (/root/code/mindmap):\n\
             \n\
             1. [Mindmap] Components: This should say where overrides reset.\n\
             2. [Mindmap] This has no node.\n\
             3. Components: This has no page.\n\
             4. Bare comment.\n\
             \n\
             Why does this rebuild?"
        );
    }

    #[test]
    fn empty_prompt_adds_one_instruction_line() {
        let comments = vec![comment(
            "This should say where overrides reset.",
            Some("Components"),
            Some("Mindmap"),
        )];
        let formatted = format_prompt("Mindmap", Some("/root/code/mindmap"), &comments, "   ")
            .unwrap_or_default();
        assert_eq!(
            formatted,
            "Mindmap comments for Mindmap (/root/code/mindmap):\n\
             \n\
             1. [Mindmap] Components: This should say where overrides reset.\n\
             \n\
             Address these comments in the project document."
        );
    }

    #[test]
    fn omits_a_missing_path_and_works_without_comments() {
        let comments = vec![comment("Bare comment.", None, None)];
        assert_eq!(
            format_prompt("Mindmap", None, &comments, "").unwrap_or_default(),
            "Mindmap comments for Mindmap:\n\n1. Bare comment.\n\nAddress these comments in the project document."
        );
        assert_eq!(
            format_prompt("Mindmap", None, &[], " Just answer this. ").unwrap_or_default(),
            "Just answer this."
        );
        assert!(format_prompt("Mindmap", None, &[], "  ").is_err());
    }

    #[test]
    fn escapes_newlines_tabs_and_backslashes_in_comment_fields() {
        let comments = vec![comment(
            "line one\nline two\twith\\slash",
            Some("node\nname"),
            Some("page\tone"),
        )];
        assert_eq!(
            format_prompt("Mindmap", None, &comments, "").unwrap_or_default(),
            "Mindmap comments for Mindmap:\n\
             \n\
             1. [page\\tone] node\\nname: line one\\nline two\\twith\\\\slash\n\
             \n\
             Address these comments in the project document."
        );
    }

    #[test]
    fn rejects_an_oversized_prompt() {
        assert!(validate_prompt(&"x".repeat(MAX_PROMPT_CHARS)).is_ok());
        assert!(validate_prompt(&"x".repeat(MAX_PROMPT_CHARS + 1)).is_err());
    }
}
