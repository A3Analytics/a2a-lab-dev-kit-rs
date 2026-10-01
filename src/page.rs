//! Cursor pagination shared by list and query operations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::A2aLabError;

/// Largest page a caller may request.
pub const MAX_PAGE_LIMIT: u32 = 1_000;

fn default_limit() -> u32 {
    100
}

/// A page request using an opaque cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
}

impl PageRequest {
    /// Creates a request after checking the cursor and limit.
    pub fn new(cursor: Option<String>, limit: u32) -> Result<Self, A2aLabError> {
        let request = Self { cursor, limit };
        request.check()?;
        Ok(request)
    }

    /// Rejects an empty cursor, a non-numeric cursor, or a limit outside `1..=MAX_PAGE_LIMIT`.
    pub fn check(&self) -> Result<(), A2aLabError> {
        if self.limit == 0 || self.limit > MAX_PAGE_LIMIT {
            return Err(A2aLabError::invalid(
                "limit",
                format!("must be 1..={MAX_PAGE_LIMIT}"),
            ));
        }
        if let Some(cursor) = &self.cursor
            && (cursor.is_empty() || !cursor.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err(A2aLabError::invalid("cursor", "is not a valid page cursor"));
        }
        Ok(())
    }

    /// Returns the opaque cursor.
    #[must_use]
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Returns the requested page size.
    #[must_use]
    pub const fn limit(&self) -> u32 {
        self.limit
    }

    pub(crate) fn offset(&self) -> Result<usize, A2aLabError> {
        self.check()?;
        let Some(cursor) = &self.cursor else {
            return Ok(0);
        };
        cursor
            .parse()
            .map_err(|_| A2aLabError::invalid("cursor", "is not a valid page cursor"))
    }
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            cursor: None,
            limit: default_limit(),
        }
    }
}

/// One page of results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Page<T> {
    items: Vec<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

impl<T> Page<T> {
    /// Creates a page. `next_cursor` is absent on the last page.
    #[must_use]
    pub fn new(items: Vec<T>, next_cursor: Option<String>) -> Self {
        Self { items, next_cursor }
    }

    /// Returns the items in this page.
    #[must_use]
    pub fn items(&self) -> &[T] {
        &self.items
    }

    /// Returns the cursor for the following page.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }
}

pub(crate) fn slice_page<T: Clone>(
    items: &[T],
    request: &PageRequest,
) -> Result<Page<T>, A2aLabError> {
    let start = request.offset()?;
    if start > items.len() {
        return Err(A2aLabError::invalid("cursor", "is past the end"));
    }
    let end = start.saturating_add(usize::try_from(request.limit()).unwrap_or(usize::MAX));
    let end = end.min(items.len());
    let next_cursor = (end < items.len()).then(|| end.to_string());
    Ok(Page::new(items[start..end].to_vec(), next_cursor))
}
