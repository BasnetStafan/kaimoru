#!/usr/bin/env python3
"""Cleanse one Japan Post postal-code CSV into an importable file.

Reads the classic KEN_ALL layout (Shift_JIS) or the UTF-8
edition — both publish the same 15 columns — and writes a
UTF-8 CSV with only the four columns the address forms need:

    postal_code, prefecture, city, town

Cleansing steps:
  * keep columns 3/7/8/9 of the source (7-digit code, and
    the kanji prefecture / city / town); the JIS code, the
    old 5-digit code, the kana readings and the flags are
    dropped;
  * drop abolished rows (update flag "2");
  * rejoin town names Japan Post split across consecutive
    records (a name over 38 full-width characters): a
    single-town code's extra records are always fragments,
    and inside a multi-town code a fragment continues while
    its parenthesis is unclosed;
  * normalise the placeholder town texts ("以下に掲載がない
    場合", "...の次に番地がくる場合", "...一円") to an empty
    town — those codes resolve at city level;
  * drop exact duplicates.

Usage: cleanse_postal_csv.py INPUT.csv OUTPUT.csv
Statistics are printed to stdout for the caller's log.
"""

import csv
import sys

# Source column indexes (0-based) in the published layout.
COL_CODE = 2
COL_PREFECTURE = 6
COL_CITY = 7
COL_TOWN = 8
COL_MULTI_TOWN = 12  # "1" when one code covers several towns
COL_UPDATE = 13  # "2" marks an abolished row

NO_TOWN_TEXT = "以下に掲載がない場合"


def decode(raw: bytes) -> str:
    """Decodes the input as UTF-8 (the new edition) or
    Shift_JIS (the classic KEN_ALL), in that order."""
    for encoding in ("utf-8-sig", "cp932"):
        try:
            return raw.decode(encoding)
        except UnicodeDecodeError:
            continue
    raise SystemExit("input is neither UTF-8 nor Shift_JIS")


def is_placeholder_town(town: str) -> bool:
    """True for the source's no-town marker texts."""
    return (
        town == NO_TOWN_TEXT
        or town.endswith("の次に番地がくる場合")
        or town.endswith("一円")
    )


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: cleanse_postal_csv.py INPUT OUTPUT", file=sys.stderr)
        return 2
    with open(sys.argv[1], "rb") as handle:
        text = decode(handle.read())

    rows_in = 0
    malformed = 0
    abolished = 0
    merged = 0
    duplicates = 0
    cleaned = []
    pending = None  # [code, prefecture, city, town, multi_town]

    def flush() -> None:
        nonlocal pending
        if pending is not None:
            cleaned.append(pending[:4])
            pending = None

    for row in csv.reader(text.splitlines()):
        if not row:
            continue
        rows_in += 1
        if len(row) != 15:
            malformed += 1
            continue
        code = row[COL_CODE].strip()
        if len(code) != 7 or not code.isdigit():
            malformed += 1
            continue
        if row[COL_UPDATE].strip() == "2":
            abolished += 1
            continue
        prefecture = row[COL_PREFECTURE].strip()
        city = row[COL_CITY].strip()
        town = row[COL_TOWN].strip()
        multi_town = row[COL_MULTI_TOWN].strip() == "1"
        if pending is not None and pending[:4] == [code, prefecture, city, town]:
            # An exact repeat of the pending row is a duplicate
            # in the source, never a split fragment.
            duplicates += 1
            continue
        if pending is not None and pending[:3] == [code, prefecture, city]:
            unclosed = pending[3].count("（") > pending[3].count("）")
            if not pending[4] or unclosed:
                pending[3] += town
                merged += 1
                continue
        flush()
        pending = [code, prefecture, city, town, multi_town]
    flush()

    seen = set()
    final = []
    for code, prefecture, city, town in cleaned:
        if is_placeholder_town(town):
            town = ""
        key = (code, town)
        if key in seen:
            duplicates += 1
            continue
        seen.add(key)
        final.append([code, prefecture, city, town])

    with open(sys.argv[2], "w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(["postal_code", "prefecture", "city", "town"])
        writer.writerows(final)

    print(f"input rows:    {rows_in}")
    print(f"abolished:     {abolished} (dropped)")
    print(f"split rows:    {merged} (rejoined)")
    print(f"malformed:     {malformed} (skipped)")
    print(f"duplicates:    {duplicates} (dropped)")
    print(f"cleaned rows:  {len(final)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
