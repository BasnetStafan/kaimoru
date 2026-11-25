//! Shopping-agent brief models.
//!
//! A brief is what a visitor asks the agent to shop for: a
//! budget, an optional category and two lists of needs. The
//! conversation layer that produces a brief from raw input
//! and the run engine that shops from it both live in the
//! services layer ([`crate::services::agent_run_service`]).

/// How a run weighs the four scoring factors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// The default balance of price, rating, needs and stock.
    #[default]
    Balanced,
    /// Price dominates: find the cheapest acceptable products.
    Cheapest,
    /// Rating dominates: find the best-reviewed products.
    BestRated,
}

/// The four factor weights of a scoring preset. Every preset's
/// weights sum to 1.0, so a total score is always in 0.0..=1.0.
#[derive(Clone, Copy, Debug)]
pub struct Weights {
    /// Weight of the pool-normalised price score.
    pub price: f64,
    /// Weight of the rating score (live average over 5).
    pub rating: f64,
    /// Weight of the needs-match score.
    pub needs: f64,
    /// Weight of the stock-depth score.
    pub stock: f64,
}

impl Preset {
    /// Returns the preset named by a brief-form value, or `None`
    /// for an unknown value.
    pub fn from_form_value(value: &str) -> Option<Self> {
        match value {
            "balanced" => Some(Self::Balanced),
            "cheapest" => Some(Self::Cheapest),
            "best_rated" => Some(Self::BestRated),
            _ => None,
        }
    }

    /// The brief-form value naming this preset.
    pub fn form_value(&self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Cheapest => "cheapest",
            Self::BestRated => "best_rated",
        }
    }

    /// This preset's factor weights.
    ///
    /// Balanced is the locked 0.35 / 0.25 / 0.30 / 0.10
    /// (price / rating / needs / stock). Cheapest puts ≈0.65 on
    /// price and splits the remaining 0.35 as 0.15 rating, 0.15
    /// needs, 0.05 stock; BestRated puts ≈0.55 on rating and
    /// splits the remaining 0.45 as 0.20 price, 0.20 needs, 0.05
    /// stock — in both, needs keeps real influence so a preset
    /// never silently ignores what the visitor asked for.
    pub fn weights(&self) -> Weights {
        match self {
            Self::Balanced => Weights {
                price: 0.35,
                rating: 0.25,
                needs: 0.30,
                stock: 0.10,
            },
            Self::Cheapest => Weights {
                price: 0.65,
                rating: 0.15,
                needs: 0.15,
                stock: 0.05,
            },
            Self::BestRated => Weights {
                price: 0.20,
                rating: 0.55,
                needs: 0.20,
                stock: 0.05,
            },
        }
    }
}

/// A validated shopping brief: the input of one agent run.
#[derive(Clone, Debug)]
pub struct Brief {
    /// Hard budget cap in integer yen, for the cart total
    /// including shipping. The agent never auto-adds past it.
    /// `None` is the chat flow's "no budget stated": the run
    /// then has no cap and nothing is reported over-budget.
    pub budget_yen: Option<i64>,
    /// Category name restricting the candidate pool, when the
    /// visitor picked one.
    pub category: Option<String>,
    /// Needs a pick must satisfy, and only through a product tag:
    /// a description keyword can support a must-have but never
    /// satisfy it alone.
    pub must_have: Vec<String>,
    /// Needs that raise a product's score when they match, by
    /// tag (full credit) or keyword (partial credit).
    pub nice_to_have: Vec<String>,
    /// The scoring preset the visitor chose.
    pub preset: Preset,
}

impl Brief {
    /// Every need in the brief, must-haves first.
    pub fn all_needs(&self) -> impl Iterator<Item = &String> {
        self.must_have.iter().chain(self.nice_to_have.iter())
    }
}

/// The brief a chat conversation has accumulated so far
/// the fields the visitor has stated,
/// merged turn by turn. Every field starts unset; a search
/// runs from whatever has accumulated. Kept in the session,
/// so it is serialisable.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ChatBrief {
    /// Stated budget cap in yen; `None` while unstated or
    /// after the visitor waived it (気にしない).
    pub budget_yen: Option<i64>,
    /// Stated category name.
    pub category: Option<String>,
    /// Stated must-have needs.
    pub must_have: Vec<String>,
    /// Stated nice-to-have needs.
    pub nice_to_have: Vec<String>,
    /// The scoring preset in force (the visitor's chip choice,
    /// or a preset the conversation set).
    pub preset: Preset,
    /// Whether the preset was chosen deliberately at some
    /// point in this conversation — a chip tap, a model
    /// update, or the deterministic rules reading priority
    /// words — as opposed to the untouched default. A
    /// deliberate preset is something to search FOR (see
    /// `agent_chat_service::has_signal`); the default is not.
    #[serde(default)]
    pub preset_explicit: bool,
}

impl ChatBrief {
    /// The engine brief for a search from the accumulated
    /// state: exactly the accumulated fields, nothing invented.
    pub fn to_brief(&self) -> Brief {
        Brief {
            budget_yen: self.budget_yen,
            category: self.category.clone(),
            must_have: self.must_have.clone(),
            nice_to_have: self.nice_to_have.clone(),
            preset: self.preset,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Preset;

    #[test]
    fn preset_weights_sum_to_one() {
        for preset in [Preset::Balanced, Preset::Cheapest, Preset::BestRated] {
            let w = preset.weights();
            let sum = w.price + w.rating + w.needs + w.stock;
            assert!((sum - 1.0).abs() < 1e-9, "{preset:?} sums to {sum}");
        }
    }

    #[test]
    fn preset_form_values_round_trip() {
        for preset in [Preset::Balanced, Preset::Cheapest, Preset::BestRated] {
            assert_eq!(Preset::from_form_value(preset.form_value()), Some(preset));
        }
        assert_eq!(Preset::from_form_value("random"), None);
    }
}
