# RFM analytics — specification

V2 item 3 (docs/specs/EC_V2_SCOPE.md). Admin-only, read-only
analytics over order history: a recency / frequency / monetary
score per member, the classic named RFM segments, a customer
leaderboard, and RFM band conditions in the existing customer
segment system (customer_segments), so campaigns can target by
band instead of by raw yen / count / date thresholds.

Status: APPROVED 2026-11-27 — the owner took every
recommendation in section 8 (D1 quintiles, D2 all
non-cancelled orders, D3 monetary net of refunds, D4 page +
segment conditions in one build, D5 the fixed named-segment
table). Built the same day.

## 1. Population

- Members only: orders with a user_id. Guest orders carry an
  email but no member identity and are never scored (identity
  by account, not by email matching).
- A member with no counted order is "unscored" and shown as
  such; they are not band 1.

## 2. Counted orders

- Counted: status received, preparing, shipped, delivered —
  every placed order that was not cancelled.
- Excluded: cancelled orders, entirely (all three measures).
- Monetary is NET of refunds: order total_yen minus the
  order's payments' amount_refunded_yen. A fully refunded
  order still counts for R and F (the purchase happened) but
  contributes 0 to M.
- (Decision D2 covers the counted-status set.)

## 3. Raw measures

Per member, over counted orders:

- R (recency): days from the newest order's JST calendar date
  to today (JST). Smaller is better.
- F (frequency): count of counted orders.
- M (monetary): sum of net order totals, integer yen.

## 4. Scores (bands 1-5)

- Quintile bands over the scored population, per measure:
  the top fifth scores 5, the bottom fifth scores 1. R is
  inverted (fewest days = 5).
- Ties share a band: equal raw values never split across
  bands (band edges fall between distinct values).
- Small populations: with fewer than 5 scored members the
  bands collapse to the distinct values present; the page
  states the scored population size so an operator can see
  when bands are coarse.
- (Decision D1: quintiles vs fixed thresholds.)

## 5. Named segments

Fixed mapping over (R, F, M) bands, shown on the RFM page
with member counts; a member belongs to exactly one:

- Champions: R>=4, F>=4, M>=4
- Loyal: F>=4 (and not Champions)
- Big spenders: M>=4 (and not above)
- Promising: R>=4, F 2-3
- New: R=5, F=1
- At risk: R<=2, F>=3
- Hibernating: R<=2 (and not At risk)
- Other: everything else

## 6. Surfaces

- New admin page (analytics area): scored-population size,
  the named-segment counts, a leaderboard (top members by M,
  switchable to F and to most recent R), and the full member
  table with raw R/F/M, bands, and named segment; sortable
  by each band. Same admin permission group as the existing
  customers / segments pages.
- Customers list: R/F/M band columns added.
- Segment editor: three optional conditions — minimum R
  band, minimum F band, minimum M band (1-5 each) — AND-ed
  with the existing conditions exactly like them. This is
  the scope item "RFM bands become selectable".
- Nothing member-facing: scores are operator-only, never
  shown on MyPage. (Decision D4.)

## 7. Computation and storage

- Recompute on view: one aggregate query over orders joined
  to payments, banding done in the service layer. No scores
  table, no background job, no staleness window. The scope
  allows a scores table only if this proves too slow; at any
  plausible size for this store the aggregate is a single
  indexed scan.
- The segment-condition evaluation needs bands at query
  time: the segment matcher computes the same banding as a
  CTE over the same aggregate, so a segment match and the
  RFM page can never disagree.
- Migration (45) only for the segment editor: three nullable
  band columns on customer_segments (min_r_score,
  min_f_score, min_m_score, each CHECK 1..5 when present).

## 8. Owner decisions

- D1 Scoring: quintile bands (recommended; the scope's own
  word, self-adjusting as the store grows) vs fixed
  thresholds (stable over time, but arbitrary yen/day lines
  that age badly).
- D2 Counted orders: all non-cancelled (recommended) vs
  delivered-only (stricter "completed purchase", but recent
  behaviour disappears until delivery and R lags reality).
- D3 M net of refunds (recommended) vs gross totals
  (simpler; overstates members who were refunded).
- D4 Scope of this build: page + customers columns + segment
  band conditions (recommended; the segment feed is the
  scope's stated purpose) vs page only, segment conditions
  as a follow-up.
- D5 Named segments: the fixed table in section 5
  (recommended) vs bands only, no named layer.

## 9. Testing

- Unit: banding (quintile edges, ties sharing a band, R
  inversion, small populations), named-segment mapping
  (exactly one segment per member), net-M aggregation.
- Suite: admin RFM page renders with fixture members at
  known bands; segment with band conditions matches exactly
  the members the page shows in those bands; unscored
  members never match a band condition; permission guard
  holds for staff without the group.
- Visual: the RFM page and the segment editor checked in a
  real browser at desktop and narrow widths.
