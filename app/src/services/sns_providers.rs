//! SNS provider definitions.
//! The static shape of each provider: slugs, display names,
//! endpoint paths, issuers, scopes. Credentials resolve
//! through the configuration crate's environment accessors;
//! nothing secret lives here.

use crate::config::CONFIG;

/// One SNS provider the store can integrate with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SnsProvider {
    /// LINE Login.
    Line,
    /// Google OAuth / OpenID Connect.
    Google,
    /// Yahoo! JAPAN ID (YConnect).
    Yahoo,
}

/// The resolved endpoints of one provider.
pub struct ProviderEndpoints {
    /// Authorization endpoint (browser redirect target).
    pub authorize: String,
    /// Token endpoint (server-side code exchange).
    pub token: String,
    /// JWKS endpoint (ID-token signature keys).
    pub jwks: String,
}

impl SnsProvider {
    /// Every provider, in display order.
    pub const ALL: [Self; 3] = [Self::Line, Self::Google, Self::Yahoo];

    /// Parses a provider from its URL slug; anything else is
    /// not a provider route.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "line" => Some(Self::Line),
            "google" => Some(Self::Google),
            "yahoo" => Some(Self::Yahoo),
            _ => None,
        }
    }

    /// The URL slug of this provider.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Google => "google",
            Self::Yahoo => "yahoo",
        }
    }

    /// The Japanese display name used by buttons and rows.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Line => "LINE",
            Self::Google => "Google",
            Self::Yahoo => "Yahoo! JAPAN",
        }
    }

    /// The expected `iss` claim of this provider's ID tokens.
    #[must_use]
    pub fn issuer(self) -> &'static str {
        match self {
            Self::Line => "https://access.line.me",
            Self::Google => "https://accounts.google.com",
            Self::Yahoo => "https://auth.login.yahoo.co.jp",
        }
    }

    /// The OpenID scopes requested at authorization time.
    /// Yahoo grants the email scope only after its UserInfo
    /// review; until then tokens simply carry no email and the
    /// flow falls back to asking for one.
    #[must_use]
    pub fn scope(self) -> &'static str {
        match self {
            Self::Line | Self::Yahoo => "openid profile email",
            Self::Google => "openid email profile",
        }
    }

    /// This provider's (client id, client secret) pair from the
    /// environment, when both resolve.
    #[must_use]
    pub fn credentials(self) -> Option<(String, String)> {
        let (id, secret) = match self {
            Self::Line => (CONFIG.line_channel_id(), CONFIG.line_channel_secret()),
            Self::Google => (CONFIG.google_client_id(), CONFIG.google_client_secret()),
            Self::Yahoo => (CONFIG.yahoo_client_id(), CONFIG.yahoo_client_secret()),
        };
        match (id, secret) {
            (Some(id), Some(secret)) => Some((id, secret)),
            _ => None,
        }
    }

    /// The test base-URL override for this provider, when set.
    fn base_override(self) -> Option<String> {
        match self {
            Self::Line => CONFIG.sns_line_base_url(),
            Self::Google => CONFIG.sns_google_base_url(),
            Self::Yahoo => CONFIG.sns_yahoo_base_url(),
        }
    }

    fn authorize_path(self) -> &'static str {
        match self {
            Self::Line => "/oauth2/v2.1/authorize",
            Self::Google => "/o/oauth2/v2/auth",
            Self::Yahoo => "/yconnect/v2/authorization",
        }
    }

    fn token_path(self) -> &'static str {
        match self {
            Self::Line => "/oauth2/v2.1/token",
            Self::Google => "/token",
            Self::Yahoo => "/yconnect/v2/token",
        }
    }

    fn jwks_path(self) -> &'static str {
        match self {
            Self::Line => "/oauth2/v2.1/certs",
            Self::Google => "/oauth2/v3/certs",
            Self::Yahoo => "/yconnect/v2/public-keys",
        }
    }

    /// Resolves this provider's endpoints. With a base
    /// override (test stubs) every URL derives from the one
    /// base, because the stub serves all paths on one host;
    /// otherwise the production hosts apply.
    #[must_use]
    pub fn endpoints(self) -> ProviderEndpoints {
        if let Some(base) = self.base_override() {
            let base = base.trim_end_matches('/');
            return ProviderEndpoints {
                authorize: format!("{base}{}", self.authorize_path()),
                token: format!("{base}{}", self.token_path()),
                jwks: format!("{base}{}", self.jwks_path()),
            };
        }
        match self {
            Self::Line => ProviderEndpoints {
                authorize: "https://access.line.me/oauth2/v2.1/authorize".to_string(),
                token: "https://api.line.me/oauth2/v2.1/token".to_string(),
                jwks: "https://api.line.me/oauth2/v2.1/certs".to_string(),
            },
            Self::Google => ProviderEndpoints {
                authorize: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                token: "https://oauth2.googleapis.com/token".to_string(),
                jwks: "https://www.googleapis.com/oauth2/v3/certs".to_string(),
            },
            Self::Yahoo => ProviderEndpoints {
                authorize: "https://auth.login.yahoo.co.jp/yconnect/v2/authorization".to_string(),
                token: "https://auth.login.yahoo.co.jp/yconnect/v2/token".to_string(),
                jwks: "https://auth.login.yahoo.co.jp/yconnect/v2/public-keys".to_string(),
            },
        }
    }
}
