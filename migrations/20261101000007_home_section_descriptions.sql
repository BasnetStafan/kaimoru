-- Home section descriptions: one admin-editable text per
-- section row, next to its heading override. Only the
-- featured section's templates render it today (the top
-- page banner and the /special page share the one value);
-- an empty value shows the built-in default sentence,
-- exactly like the heading override beside it.
ALTER TABLE home_sections ADD COLUMN description text NOT NULL DEFAULT '';
