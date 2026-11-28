//! The MCP principal: who a credential authenticates as.
//!
//! Two credential kinds reach the MCP tools — an API key
//! (Phases 2–3) and an OAuth 2.1 access token issued by the
//! external authorization server (Phase 4) — and everything
//! a caller creates (a cart, an order draft) belongs to the
//! principal, whichever kind it is. Ownership, idempotency
//! and rate limits all compare the one [`McpPrincipal::principal_key`]
//! string, so the two kinds can never read or spend each
//! other's state; the `api_key_id` / `oauth_subject` columns
//! on the ownership tables are provenance only.

/// Who an MCP credential authenticated as: the API key's
/// row, or the OAuth token's subject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpPrincipal {
    /// An API key, by its row ID.
    ApiKey(i64),
    /// An OAuth subject (the token's `sub` claim).
    OAuth(String),
}

impl McpPrincipal {
    /// The one ownership value every check compares:
    /// `key:<id>` for a key, `oauth:<subject>` for a token.
    /// The prefix keeps the two namespaces disjoint — a
    /// subject that happens to look numeric can never
    /// collide with a key's row ID.
    pub fn principal_key(&self) -> String {
        match self {
            Self::ApiKey(id) => format!("key:{id}"),
            Self::OAuth(subject) => format!("oauth:{subject}"),
        }
    }

    /// The creating key's ID, for the provenance column —
    /// `None` when an OAuth token created the row.
    pub fn api_key_id(&self) -> Option<i64> {
        match self {
            Self::ApiKey(id) => Some(*id),
            Self::OAuth(_) => None,
        }
    }

    /// The creating token's subject, for the provenance
    /// column — `None` when an API key created the row.
    pub fn oauth_subject(&self) -> Option<&str> {
        match self {
            Self::ApiKey(_) => None,
            Self::OAuth(subject) => Some(subject),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_keys_carry_their_namespace() {
        assert_eq!(McpPrincipal::ApiKey(7).principal_key(), "key:7");
        assert_eq!(
            McpPrincipal::OAuth("auth0|abc".to_string()).principal_key(),
            "oauth:auth0|abc"
        );
        // A numeric subject still cannot collide with a key.
        assert_ne!(
            McpPrincipal::OAuth("7".to_string()).principal_key(),
            McpPrincipal::ApiKey(7).principal_key()
        );
    }

    #[test]
    fn provenance_columns_follow_the_kind() {
        let key = McpPrincipal::ApiKey(3);
        assert_eq!(key.api_key_id(), Some(3));
        assert_eq!(key.oauth_subject(), None);
        let oauth = McpPrincipal::OAuth("sub-1".to_string());
        assert_eq!(oauth.api_key_id(), None);
        assert_eq!(oauth.oauth_subject(), Some("sub-1"));
    }
}
