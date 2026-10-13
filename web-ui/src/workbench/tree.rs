//! Bounded JSON tree. Agent strings stay text; long and deep values collapse.

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use serde_json::{Map, Value};

use crate::codec;

pub const TREE_DEPTH: usize = 4;
pub const TREE_FANOUT: usize = 12;
pub const TREE_STRING: usize = 180;
const TREE_BUILD_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Text(String),
    Group {
        name: String,
        children: Vec<Node>,
        collapsed: bool,
    },
}

pub fn from_value(value: &Value) -> Node {
    node_at(value, 0)
}

pub fn redact(node: &Node, secret: Option<&str>) -> Node {
    match node {
        Node::Text(text) => Node::Text(codec::redact(text, secret)),
        Node::Group {
            name,
            children,
            collapsed,
        } => Node::Group {
            name: codec::redact(name, secret),
            children: children.iter().map(|child| redact(child, secret)).collect(),
            collapsed: *collapsed,
        },
    }
}

#[cfg(test)]
pub(crate) fn contains_text(node: &Node, needle: &str) -> bool {
    match node {
        Node::Text(text) => text.contains(needle),
        Node::Group { name, children, .. } => {
            name.contains(needle) || children.iter().any(|child| contains_text(child, needle))
        }
    }
}

fn node_at(value: &Value, depth: usize) -> Node {
    if depth >= TREE_BUILD_DEPTH {
        return text_node(&compact(value));
    }
    match value {
        Value::Array(items) => group("list", array_children(items, depth), depth),
        Value::Object(object) => group("object", object_children(object, depth), depth),
        Value::String(text) => text_node(text),
        other => Node::Text(scalar(other)),
    }
}

fn array_children(items: &[Value], depth: usize) -> Vec<Node> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| named(&index.to_string(), node_at(item, depth + 1), depth + 1))
        .collect()
}

fn object_children(object: &Map<String, Value>, depth: usize) -> Vec<Node> {
    object
        .iter()
        .map(|(key, value)| named(key, node_at(value, depth + 1), depth + 1))
        .collect()
}

fn named(name: &str, child: Node, depth: usize) -> Node {
    group(name, vec![child], depth)
}

fn group(name: impl Into<String>, children: Vec<Node>, depth: usize) -> Node {
    let collapsed = depth >= TREE_DEPTH;
    Node::Group {
        name: name.into(),
        children: bound(children),
        collapsed,
    }
}

fn bound(mut children: Vec<Node>) -> Vec<Node> {
    if children.len() <= TREE_FANOUT {
        return children;
    }
    let rest = children.split_off(TREE_FANOUT);
    let hidden = rest.len();
    children.push(Node::Group {
        name: format!("{hidden} more"),
        children: rest,
        collapsed: true,
    });
    children
}

fn text_node(text: &str) -> Node {
    if text.chars().count() <= TREE_STRING {
        return Node::Text(text.to_owned());
    }
    let preview: String = text.chars().take(TREE_STRING).collect();
    Node::Group {
        name: format!("{preview}…"),
        children: vec![Node::Text(text.to_owned())],
        collapsed: true,
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        other => compact(other),
    }
}

fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}
