//! RFM analytics: banding, named segments, and the admin
//! page's read model.
//!
//! Scores are computed on view from the repository's raw
//! measures — no scores table, no background job (RFM spec
//! section 7). Banding is quintile-style over the scored
//! population: band = ceil(5 * k / n) with k the number of
//! members at-or-below the member's value (at-or-above for
//! recency days, where fewer is better), so a band is a
//! statement about relative position and equal values
//! always share a band. The segment matcher computes the
//! same rule in SQL (see rfm_repository::bands_cte).

use sqlx::PgPool;

use crate::error::AppError;
use crate::pii::decrypt_field;
use crate::repositories::admin::rfm_repository::{self, MemberMeasures};

/// One scored member: raw measures, bands, named segment.
#[derive(Debug, Clone)]
pub struct ScoredMember {
    /// The member.
    pub user_id: i64,
    /// Family name, decrypted.
    pub family_name: String,
    /// Given name, decrypted.
    pub given_name: String,
    /// Account email, decrypted.
    pub email: String,
    /// Days since the newest counted order.
    pub r_days: i32,
    /// Counted order count.
    pub f_count: i64,
    /// Net monetary total, integer yen.
    pub m_yen: i64,
    /// The newest counted order's JST date.
    pub last_order_date: chrono::NaiveDate,
    /// Recency band, 1 (stalest) to 5 (freshest).
    pub r_band: i32,
    /// Frequency band, 1 to 5.
    pub f_band: i32,
    /// Monetary band, 1 to 5.
    pub m_band: i32,
    /// The named segment (RFM spec section 5).
    pub segment: NamedSegment,
}

/// The named RFM segments, in the spec's first-match order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedSegment {
    /// R>=4, F>=4, M>=4.
    Champions,
    /// F>=4, not Champions.
    Loyal,
    /// M>=4, not above.
    BigSpenders,
    /// R>=4, F 2-3.
    Promising,
    /// R=5, F=1.
    New,
    /// R<=2, F>=3.
    AtRisk,
    /// R<=2, not At risk.
    Hibernating,
    /// Everything else.
    Other,
}

impl NamedSegment {
    /// Every segment, in first-match order — the summary
    /// table's row order.
    pub const ALL: [NamedSegment; 8] = [
        NamedSegment::Champions,
        NamedSegment::Loyal,
        NamedSegment::BigSpenders,
        NamedSegment::Promising,
        NamedSegment::New,
        NamedSegment::AtRisk,
        NamedSegment::Hibernating,
        NamedSegment::Other,
    ];

    /// The operator-facing Japanese label.
    pub fn label(self) -> &'static str {
        match self {
            NamedSegment::Champions => "最優良",
            NamedSegment::Loyal => "常連",
            NamedSegment::BigSpenders => "高額購入",
            NamedSegment::Promising => "有望",
            NamedSegment::New => "新規",
            NamedSegment::AtRisk => "離反注意",
            NamedSegment::Hibernating => "休眠",
            NamedSegment::Other => "その他",
        }
    }
}

/// The named segment for one band triple — the spec
/// section 5 table, first match wins.
pub fn named_segment(r: i32, f: i32, m: i32) -> NamedSegment {
    if r >= 4 && f >= 4 && m >= 4 {
        NamedSegment::Champions
    } else if f >= 4 {
        NamedSegment::Loyal
    } else if m >= 4 {
        NamedSegment::BigSpenders
    } else if r >= 4 && (2..=3).contains(&f) {
        NamedSegment::Promising
    } else if r == 5 && f == 1 {
        NamedSegment::New
    } else if r <= 2 && f >= 3 {
        NamedSegment::AtRisk
    } else if r <= 2 {
        NamedSegment::Hibernating
    } else {
        NamedSegment::Other
    }
}

/// One band for a higher-is-better measure: k counts the
/// population at-or-below `value`.
fn band_higher(values: &[i64], value: i64) -> i32 {
    let k = values.iter().filter(|&&v| v <= value).count() as i64;
    band(k, values.len() as i64)
}

/// One band for recency days (lower is better): k counts
/// the population at-or-above `days`.
fn band_recency(values: &[i64], days: i64) -> i32 {
    let k = values.iter().filter(|&&v| v >= days).count() as i64;
    band(k, values.len() as i64)
}

/// ceil(5k / n) in integer arithmetic; the caller
/// guarantees n >= 1 and 1 <= k <= n, so the result is
/// always in 1..=5.
fn band(k: i64, n: i64) -> i32 {
    ((5 * k + n - 1) / n) as i32
}

/// Bands raw measures into scored members (pure; the
/// unit-tested core).
fn score(measures: Vec<MemberMeasures>) -> Vec<ScoredMember> {
    let r_values: Vec<i64> = measures.iter().map(|m| m.r_days as i64).collect();
    let f_values: Vec<i64> = measures.iter().map(|m| m.f_count).collect();
    let m_values: Vec<i64> = measures.iter().map(|m| m.m_yen).collect();
    measures
        .into_iter()
        .map(|m| {
            let r_band = band_recency(&r_values, m.r_days as i64);
            let f_band = band_higher(&f_values, m.f_count);
            let m_band = band_higher(&m_values, m.m_yen);
            ScoredMember {
                user_id: m.user_id,
                family_name: decrypt_field(&m.family_name),
                given_name: decrypt_field(&m.given_name),
                email: decrypt_field(&m.email),
                r_days: m.r_days,
                f_count: m.f_count,
                m_yen: m.m_yen,
                last_order_date: m.last_order_date,
                r_band,
                f_band,
                m_band,
                segment: named_segment(r_band, f_band, m_band),
            }
        })
        .collect()
}

/// The RFM page's read model: every scored member, banded
/// and segmented.
pub async fn scored_members(pool: &PgPool) -> Result<Vec<ScoredMember>, AppError> {
    Ok(score(rfm_repository::member_measures(pool).await?))
}

/// Member counts per named segment, in first-match order,
/// zero counts included (the summary shows the whole map).
pub fn segment_counts(members: &[ScoredMember]) -> Vec<(NamedSegment, i64)> {
    NamedSegment::ALL
        .iter()
        .map(|s| {
            (
                *s,
                members.iter().filter(|m| m.segment == *s).count() as i64,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measures(rows: &[(i64, i32, i64, i64)]) -> Vec<MemberMeasures> {
        rows.iter()
            .map(|&(user_id, r_days, f_count, m_yen)| MemberMeasures {
                user_id,
                family_name: String::new(),
                given_name: String::new(),
                email: String::new(),
                r_days,
                f_count,
                m_yen,
                last_order_date: chrono::NaiveDate::from_ymd_opt(2026, 11, 27).unwrap(),
            })
            .collect()
    }

    #[test]
    fn distinct_population_spreads_across_bands() {
        // Ten members, all values distinct: each quintile
        // boundary is crossed exactly every two members.
        let rows: Vec<(i64, i32, i64, i64)> = (1..=10)
            .map(|i| (i, (i * 10) as i32, i, i * 1000))
            .collect();
        let scored = score(measures(&rows));
        let by_id = |id: i64| scored.iter().find(|s| s.user_id == id).unwrap();
        // Most money / most orders -> band 5; least -> 1.
        assert_eq!(by_id(10).m_band, 5);
        assert_eq!(by_id(10).f_band, 5);
        assert_eq!(by_id(1).m_band, 1);
        assert_eq!(by_id(9).m_band, 5);
        assert_eq!(by_id(8).m_band, 4);
        // Recency inverts: fewest days (id 1) -> 5.
        assert_eq!(by_id(1).r_band, 5);
        assert_eq!(by_id(10).r_band, 1);
    }

    #[test]
    fn ties_always_share_a_band() {
        let rows = vec![
            (1, 5, 1, 100),
            (2, 5, 1, 100),
            (3, 5, 1, 100),
            (4, 5, 1, 100),
            (5, 90, 9, 900),
        ];
        let scored = score(measures(&rows));
        let bands: Vec<(i32, i32, i32)> = scored
            .iter()
            .filter(|s| s.user_id < 5)
            .map(|s| (s.r_band, s.f_band, s.m_band))
            .collect();
        assert!(bands.iter().all(|b| *b == bands[0]));
    }

    #[test]
    fn single_member_is_band_five_everywhere() {
        let scored = score(measures(&[(7, 30, 2, 5000)]));
        assert_eq!(scored[0].r_band, 5);
        assert_eq!(scored[0].f_band, 5);
        assert_eq!(scored[0].m_band, 5);
    }

    #[test]
    fn named_segments_first_match() {
        assert_eq!(named_segment(5, 5, 5), NamedSegment::Champions);
        assert_eq!(named_segment(2, 5, 5), NamedSegment::Loyal);
        assert_eq!(named_segment(3, 2, 5), NamedSegment::BigSpenders);
        assert_eq!(named_segment(4, 3, 2), NamedSegment::Promising);
        assert_eq!(named_segment(5, 1, 1), NamedSegment::New);
        assert_eq!(named_segment(1, 3, 3), NamedSegment::AtRisk);
        assert_eq!(named_segment(2, 1, 2), NamedSegment::Hibernating);
        assert_eq!(named_segment(3, 3, 3), NamedSegment::Other);
    }

    #[test]
    fn segment_counts_cover_every_member_once() {
        let rows: Vec<(i64, i32, i64, i64)> =
            (1..=10).map(|i| (i, (i * 7) as i32, i, i * 500)).collect();
        let scored = score(measures(&rows));
        let total: i64 = segment_counts(&scored).iter().map(|(_, c)| c).sum();
        assert_eq!(total, 10);
    }
}
