use std::fmt;

use rsui_core::{ActionKind, Node, Role, Text};

use crate::outline::write_line;

/// How many similar nodes to mention when a query finds nothing.
const MAX_SIMILAR: usize = 8;

/// Describes the nodes to look for. Every condition that's set must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    role: Option<Text>,
    name: Option<Text>,
    name_contains: Option<String>,
    key: Option<Text>,
    action: Option<Text>,
}

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = Some(role.name().into());
        self
    }

    /// Matches a role by its name in snapshots, such as `"text_input"`.
    pub fn role_name(mut self, role: impl Into<Text>) -> Self {
        self.role = Some(role.into());
        self
    }

    /// Matches this exact name.
    pub fn name(mut self, name: impl Into<Text>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Matches names containing this text, ignoring case.
    pub fn name_contains(mut self, text: &str) -> Self {
        self.name_contains = Some(text.to_lowercase());
        self
    }

    /// Matches a key by how it's written in an outline.
    pub fn key(mut self, key: impl fmt::Display) -> Self {
        self.key = Some(key.to_string().into());
        self
    }

    pub fn action(mut self, action: ActionKind) -> Self {
        self.action = Some(action.name().into());
        self
    }

    /// Matches an action by its name in snapshots, such as `"set_value"`.
    pub fn action_name(mut self, action: impl Into<Text>) -> Self {
        self.action = Some(action.into());
        self
    }

    fn matches(&self, node: &Node) -> bool {
        self.role
            .as_deref()
            .is_none_or(|role| node.role.name() == role)
            && self.name.as_deref().is_none_or(|name| node.name == name)
            && self
                .name_contains
                .as_deref()
                .is_none_or(|text| node.name.to_lowercase().contains(text))
            && self.key.as_deref().is_none_or(|key| {
                node.key
                    .as_ref()
                    .is_some_and(|node_key| node_key.to_string() == key)
            })
            && self
                .action
                .as_deref()
                .is_none_or(|name| node.actions.iter().any(|action| action.name() == name))
    }

    /// Every match in the subtree at `node`, which is at `path` in its tree.
    /// Everything under this node that matches, in tree order. `path` is where the node is.
    pub fn find_all(&self, node: &Node, path: &[u32]) -> Vec<Found> {
        let mut found = Vec::new();
        let mut path = path.to_vec();
        self.collect(node, &mut path, &mut found);
        found
    }

    fn collect(&self, node: &Node, path: &mut Vec<u32>, found: &mut Vec<Found>) {
        if self.matches(node) {
            found.push(Found {
                path: path.clone(),
                node: node.clone(),
            });
        }
        for (index, child) in node.children.iter().enumerate() {
            path.push(index as u32);
            self.collect(child, path, found);
            path.pop();
        }
    }

    /// The one node under this one that matches, or why there isn't one. `path` is where this node is.
    pub fn find_one(&self, root: &Node, path: &[u32]) -> Result<Found, FindError> {
        let mut found = self.find_all(root, path);
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => Err(FindError::NotFound {
                query: self.to_string(),
                similar: self.similar(root, path),
            }),
            count => Err(FindError::Ambiguous {
                query: self.to_string(),
                count,
                matches: found.iter().take(MAX_SIMILAR).map(Found::line).collect(),
            }),
        }
    }

    /// Nodes the query may have meant: the same role, or failing that, anything that
    /// can be acted on.
    fn similar(&self, root: &Node, path: &[u32]) -> Vec<String> {
        let same_role = Query {
            role: self.role.clone(),
            ..Query::default()
        };
        let mut nodes = if self.role.is_some() {
            same_role.find_all(root, path)
        } else {
            Vec::new()
        };
        if nodes.is_empty() {
            nodes = Query::default()
                .find_all(root, path)
                .into_iter()
                .filter(|found| !found.node.actions.is_empty())
                .collect();
        }
        nodes.iter().take(MAX_SIMILAR).map(Found::line).collect()
    }
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(role) = &self.role {
            parts.push(format!("role {role}"));
        }
        if let Some(name) = &self.name {
            parts.push(format!("name {name:?}"));
        }
        if let Some(text) = &self.name_contains {
            parts.push(format!("name containing {text:?}"));
        }
        if let Some(key) = &self.key {
            parts.push(format!("key {key}"));
        }
        if let Some(action) = &self.action {
            parts.push(format!("action {action}"));
        }
        if parts.is_empty() {
            f.write_str("any node")
        } else {
            f.write_str(&parts.join(", "))
        }
    }
}

/// A node found by a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Child indices from the root to the node.
    pub path: Vec<u32>,
    pub node: Node,
}

impl Found {
    /// The node's line in an outline.
    pub fn line(&self) -> String {
        let mut out = String::new();
        write_line(&self.node, &self.path, &mut out);
        out
    }
}

/// Why a query didn't identify one node. Each error lists nodes the query may have
/// meant, so whoever wrote it can correct it without another look at the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindError {
    NotFound {
        query: String,
        similar: Vec<String>,
    },
    Ambiguous {
        query: String,
        count: usize,
        matches: Vec<String>,
    },
}

impl fmt::Display for FindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lines = |f: &mut fmt::Formatter<'_>, lines: &[String]| {
            for line in lines {
                write!(f, "\n  {line}")?;
            }
            Ok(())
        };
        match self {
            Self::NotFound { query, similar } => {
                write!(f, "no node matches {query}")?;
                if !similar.is_empty() {
                    f.write_str("; similar nodes:")?;
                    lines(f, similar)?;
                }
                Ok(())
            }
            Self::Ambiguous {
                query,
                count,
                matches,
            } => {
                write!(
                    f,
                    "{count} nodes match {query}, so add conditions; first matches:"
                )?;
                lines(f, matches)
            }
        }
    }
}

impl std::error::Error for FindError {}
