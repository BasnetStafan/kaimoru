//! Review models.
//!
//! A review is written by a member who bought the product, once
//! per member per product: a star rating in half steps (0.5 to
//! 5.0) plus one optional comment — no title, no display name.
//! These types are the feature's shared vocabulary: stored row,
//! submitted form, validated form, field errors, aggregate.

use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// One stored review row.
#[derive(Debug, Clone, FromRow)]
pub struct Review {
    /// Review ID.
    pub id: i64,
    /// The product reviewed.
    pub product_id: i64,
    /// The author (a member), from the session when posted.
    pub user_id: i64,
    /// The order that proves the purchase.
    pub order_id: i64,
    /// Stars in half steps, 0.5 to 5.0 (the CHECK constraint
    /// is the backstop). A float is safe here: a rating is not
    /// money, and every half step is exact in binary.
    pub rating: f64,
    /// The member's comment; empty when they left only a rating.
    pub comment: String,
    /// When the review was posted; never changes.
    pub created_at: DateTime<Utc>,
    /// When the review was last edited.
    pub updated_at: DateTime<Utc>,
    /// When an operator hid the review from the storefront;
    /// `None` = visible. Hiding is reversible moderation,
    /// separate from deletion.
    pub hidden_at: Option<DateTime<Utc>>,
    /// The shop's public reply, rendered under the review;
    /// empty = no reply.
    pub shop_reply: String,
    /// When the shop reply was last saved.
    pub shop_reply_at: Option<DateTime<Utc>>,
}

/// How much of the five-star bar a rating fills, as a whole
/// percent — the one computation behind every star bar, so the
/// storefront and the admin moderation list always agree.
pub fn stars_fill_percent(rating: f64) -> i64 {
    (rating / 5.0 * 100.0).round() as i64
}

impl Review {
    /// Returns how much of the five-star bar this rating fills,
    /// as a whole percent.
    pub fn stars_fill_percent(&self) -> i64 {
        stars_fill_percent(self.rating)
    }

    /// The posted date as YYYY/MM/DD, in the app's date style.
    pub fn posted_date(&self) -> String {
        self.created_at.format("%Y/%m/%d").to_string()
    }
}

/// The review form as submitted, before validation.
///
/// The rating arrives as text, so a crafted non-numeric value
/// reaches the service's validation instead of failing parsing.
pub struct ReviewInput {
    /// The rating field as typed, when present.
    pub rating: Option<String>,
    /// The comment as typed (may be blank).
    pub comment: String,
}

/// The review form after validation: safe to store.
#[derive(Debug)]
pub struct ValidReview {
    /// Stars in half steps, 0.5 to 5.0.
    pub rating: f64,
    /// Comment, trimmed, at most 1000 characters; empty is a
    /// valid rating-only review.
    pub comment: String,
}

/// Every field error at once, so the form re-renders with each
/// message under its own field, never one at a time.
#[derive(Debug, Default)]
pub struct ReviewFieldErrors {
    /// Rating error, when any.
    pub rating: Option<String>,
    /// Comment error, when any.
    pub comment: Option<String>,
}

impl ReviewFieldErrors {
    /// True when no field carries an error.
    pub fn is_empty(&self) -> bool {
        self.rating.is_none() && self.comment.is_none()
    }
}

/// A product's computed rating: the average to one decimal
/// (`None` when unreviewed) and the count.
///
/// Aggregates are computed from review rows at read time and
/// never stored, so an edit or delete can never leave a stale
/// average behind.
#[derive(Debug, Clone, Copy, Default)]
pub struct RatingAggregate {
    /// Average rating rounded to one decimal; None at 0 reviews.
    pub rating_avg: Option<f64>,
    /// Number of reviews behind the average.
    pub rating_count: i64,
}

impl RatingAggregate {
    /// Builds an aggregate from a raw SQL average, rounding to
    /// one decimal in this one place so every screen agrees.
    pub fn from_raw(raw_avg: Option<f64>, rating_count: i64) -> Self {
        let rating_avg = if rating_count > 0 {
            raw_avg.map(|avg| (avg * 10.0).round() / 10.0)
        } else {
            None
        };
        Self {
            rating_avg,
            rating_count,
        }
    }

    /// The average with exactly one decimal (for example "4.3"),
    /// or an empty string when there are no reviews.
    pub fn average_text(&self) -> String {
        match self.rating_avg {
            Some(avg) => format!("{avg:.1}"),
            None => String::new(),
        }
    }

    /// How much of the five-star summary bar is filled, as a
    /// whole percent (the average out of 5).
    pub fn stars_fill_percent(&self) -> i64 {
        match self.rating_avg {
            Some(avg) => stars_fill_percent(avg),
            None => 0,
        }
    }
}

/// One row of the star distribution on the product page.
pub struct DistributionRow {
    /// The rating value this row counts (5.0 down to 0.5).
    pub stars: f64,
    /// The row's label: "5" for a whole step, "4.5" for a half.
    pub label: String,
    /// How many reviews gave that rating.
    pub count: i64,
    /// This row's share of all reviews, as a whole percent.
    pub percent: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_rounds_to_one_decimal() {
        // The worked example: 5, 4, 3, 5, 5 averages 4.4.
        let agg = RatingAggregate::from_raw(Some(22.0 / 5.0), 5);
        assert_eq!(agg.average_text(), "4.4");
        assert_eq!(agg.rating_count, 5);
        assert_eq!(agg.stars_fill_percent(), 88);
    }

    #[test]
    fn aggregate_averages_half_steps_too() {
        // 4.5 and 2.0 average 3.25, shown as 3.3; the bar
        // follows the shown average.
        let agg = RatingAggregate::from_raw(Some(3.25), 2);
        assert_eq!(agg.average_text(), "3.3");
        assert_eq!(agg.stars_fill_percent(), 66);
    }

    #[test]
    fn aggregate_with_no_reviews_has_no_average() {
        let agg = RatingAggregate::from_raw(None, 0);
        assert_eq!(agg.rating_avg, None);
        assert_eq!(agg.average_text(), "");
        assert_eq!(agg.stars_fill_percent(), 0);
        // A raw average with a zero count is still no reviews:
        // the count decides, never the average alone.
        let agg = RatingAggregate::from_raw(Some(5.0), 0);
        assert_eq!(agg.rating_avg, None);
    }

    #[test]
    fn review_fill_percent_covers_half_steps() {
        let mut review = Review {
            id: 1,
            product_id: 1,
            user_id: 1,
            order_id: 1,
            rating: 4.5,
            comment: String::new(),
            created_at: DateTime::default(),
            updated_at: DateTime::default(),
            hidden_at: None,
            shop_reply: String::new(),
            shop_reply_at: None,
        };
        assert_eq!(review.stars_fill_percent(), 90);
        review.rating = 5.0;
        assert_eq!(review.stars_fill_percent(), 100);
        review.rating = 0.5;
        assert_eq!(review.stars_fill_percent(), 10);
    }
}
