// Postal code address autofill.
//
// This module owns one behaviour: on any form that carries a
// postal_code field beside prefecture / city / address_line
// fields (checkout delivery, the address book, the profile
// form, sign-up), entering a full 7-digit postal code looks
// the address up in the store's own postal code table and
// fills the three fields, so the shopper only types the
// street number onwards.
//
// The data is Japan Post's published CSV, maintained by the
// store itself: db/import_postal_codes.sh loads it
// into the postal_codes table (refreshed monthly), and the
// lookup calls the store's own /api/postal-codes endpoint —
// no outside service is involved, so the Content-Security-
// Policy needs no outside host for this module. Until the
// first import the table is empty and every code answers
// "not found"; when a code is unknown the shopper is told
// so, and when the lookup itself fails the fields simply
// stay manual, exactly as they were before this module
// existed.
//
// A code can cover several towns. One town fills the street
// field directly (only while it is still empty); several
// towns offer a picker under the postal field instead of a
// guess. The DOM is built with createElement and textContent
// only — never innerHTML.
//
// The listeners live on the document, not on the fields:
// some forms (the agent settlement's address form) are
// swapped into the page after load by htmx, and a field
// bound at load time would never exist for them. Delegation
// covers every postal field whenever it appears, with no
// re-binding pass and no risk of binding one field twice;
// the per-field state (last code looked up, picker, note)
// is kept per input element in a WeakMap, so a re-rendered
// form simply starts with fresh state.

(function () {
    "use strict";

    var API_BASE = "/api/postal-codes/";

    var states = new WeakMap();

    function digits(value) {
        return (value || "").replace(/[^0-9]/g, "");
    }

    function setPrefecture(select, name) {
        for (var i = 0; i < select.options.length; i++) {
            if (select.options[i].value === name || select.options[i].text === name) {
                select.value = select.options[i].value;
                return;
            }
        }
    }

    function stateFor(postal) {
        var state = states.get(postal);
        if (!state) {
            state = { lastCode: "", pickedTown: "", picker: null, note: null };
            states.set(postal, state);
        }
        return state;
    }

    function fieldsFor(postal) {
        var scope = postal.closest("form") || document;
        return {
            prefecture: scope.querySelector('[name="prefecture"]'),
            city: scope.querySelector('[name="city"]'),
            street: scope.querySelector('[name="address_line"]')
        };
    }

    function clearPicker(state) {
        if (state.picker) {
            state.picker.remove();
            state.picker = null;
        }
    }

    function clearNote(state) {
        if (state.note) {
            state.note.remove();
            state.note = null;
        }
    }

    function showNote(postal, state, text) {
        clearNote(state);
        state.note = document.createElement("p");
        state.note.className = "field-error";
        state.note.textContent = text;
        postal.after(state.note);
    }

    function showPicker(postal, state, towns) {
        clearPicker(state);
        var picker = document.createElement("select");
        state.picker = picker;
        picker.className = "postal-town-picker";
        picker.setAttribute("aria-label", "町名の候補");
        var first = document.createElement("option");
        first.value = "";
        first.textContent = "町名を選択してください";
        picker.appendChild(first);
        towns.forEach(function (town) {
            var option = document.createElement("option");
            option.value = town;
            option.textContent = town;
            picker.appendChild(option);
        });
        picker.addEventListener("change", function () {
            if (!picker.value) {
                return;
            }
            // Overwrite only the field this picker filled
            // (or an untouched one): a street the visitor
            // typed by hand is never clobbered.
            var street = fieldsFor(postal).street;
            if (street && (!street.value || street.value === state.pickedTown)) {
                street.value = picker.value;
                state.pickedTown = picker.value;
            }
        });
        postal.after(picker);
    }

    function lookup(postal) {
        var fields = fieldsFor(postal);
        if (!fields.prefecture || !fields.city || !fields.street) {
            return;
        }
        var state = stateFor(postal);

        // Only codes actually looked up are remembered: a
        // form that opens with a code already in the field
        // (a saved address, a resumed checkout) must still
        // look that code up when the visitor re-enters it —
        // remembering the loaded value instead would silently
        // swallow exactly that lookup.
        var code = digits(postal.value);
        if (code.length !== 7 || code === state.lastCode) {
            return;
        }
        state.lastCode = code;
        fetch(API_BASE + code)
            .then(function (response) {
                if (response.status === 404) {
                    return { notFound: true };
                }
                if (!response.ok) {
                    throw new Error("postal lookup failed");
                }
                return response.json();
            })
            .then(function (data) {
                if (data.notFound) {
                    clearPicker(state);
                    showNote(postal, state, "該当する住所が見つかりませんでした");
                    return;
                }
                var results = data.addresses || [];
                if (!results.length) {
                    clearPicker(state);
                    showNote(postal, state, "該当する住所が見つかりませんでした");
                    return;
                }
                clearNote(state);
                setPrefecture(fields.prefecture, results[0].prefecture);
                fields.city.value = results[0].city;
                var towns = [];
                results.forEach(function (result) {
                    if (result.town && towns.indexOf(result.town) === -1) {
                        towns.push(result.town);
                    }
                });
                if (towns.length === 1) {
                    clearPicker(state);
                    if (!fields.street.value) {
                        fields.street.value = towns[0];
                        state.pickedTown = towns[0];
                    }
                } else if (towns.length > 1) {
                    showPicker(postal, state, towns);
                } else {
                    clearPicker(state);
                }
            })
            .catch(function () {
                // The lookup itself failed: stay silent,
                // manual entry is unaffected.
            });
    }

    function onEvent(event) {
        var target = event.target;
        if (target && target.matches && target.matches('input[name="postal_code"]')) {
            lookup(target);
        }
    }

    document.addEventListener("input", onEvent);
    document.addEventListener("change", onEvent);
})();
