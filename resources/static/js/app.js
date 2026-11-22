// Agentic Store frontend script.
// Plain vanilla JavaScript only, by project rule. Dynamic page updates are
// handled by the self hosted htmx file loaded before this script.

// A product image whose file is missing shows the neutral
// placeholder instead of the browser's broken-image icon. The
// capture-phase listener also covers images swapped in later
// (quick-add, galleries); the flag keeps it loop-safe.
function useNoImageFallback(img) {
    if (img.dataset.noImageFallback) {
        return;
    }
    img.dataset.noImageFallback = "1";
    img.src = "/static/images/no-image.svg";
}
document.addEventListener(
    "error",
    function (event) {
        var img = event.target;
        var src = img.getAttribute("src") || "";
        if (
            img instanceof HTMLImageElement &&
            (src === "" || src.indexOf("/static/") === 0)
        ) {
            useNoImageFallback(img);
        }
    },
    true
);

// Agent settlement, payment step: the method buttons switch
// in place, like the human checkout's payment options — the
// server renders both grand totals on the total element and
// this only swaps which one shows, toggles the cash-on-
// delivery fee row, and records the pick in the proceed
// form's method field. The buttons stay real submit buttons
// of the method form, so without this script the step still
// works through the server round trip. The card is swapped
// in by htmx at any time, so the listener is delegated.
document.addEventListener("click", function (event) {
    var button = event.target.closest(
        "[data-settlement-methods] button[name='payment_method']"
    );
    if (!button) {
        return;
    }
    var card = button.closest(".agent-approval-card");
    if (!card) {
        return;
    }
    event.preventDefault();
    var slug = button.value;
    card
        .querySelectorAll("[data-settlement-methods] button")
        .forEach(function (other) {
            var selected = other === button;
            other.classList.toggle("is-selected", selected);
            other.setAttribute("aria-pressed", selected ? "true" : "false");
        });
    var isCod = slug === "cod";
    var grandTotal = card.querySelector("[data-grand-total]");
    if (grandTotal) {
        var totalValue = grandTotal.querySelector("dd");
        if (totalValue) {
            totalValue.textContent = isCod
                ? grandTotal.dataset.totalCod
                : grandTotal.dataset.totalOnline;
        }
        // The fee row is only in the card while COD stands
        // chosen (the server renders it that way); switching
        // in place creates it from the rendered fee, or
        // removes it again.
        var codFeeRow = card.querySelector("[data-cod-fee-row]");
        if (isCod && !codFeeRow && grandTotal.dataset.codFee) {
            codFeeRow = document.createElement("div");
            codFeeRow.setAttribute("data-cod-fee-row", "");
            var feeDt = document.createElement("dt");
            feeDt.textContent = "代引き手数料";
            var feeDd = document.createElement("dd");
            feeDd.textContent = grandTotal.dataset.codFee;
            codFeeRow.appendChild(feeDt);
            codFeeRow.appendChild(feeDd);
            grandTotal.parentNode.insertBefore(codFeeRow, grandTotal);
        } else if (!isCod && codFeeRow) {
            codFeeRow.remove();
        }
    }
    var methodInput = card.querySelector("[data-settlement-method-input]");
    if (methodInput) {
        methodInput.value = slug;
    }
    var proceed = card.querySelector("[data-settlement-proceed]");
    if (proceed) {
        proceed.disabled = false;
    }
});

document.addEventListener("DOMContentLoaded", function () {
    // Images that already failed before this script ran are
    // caught by the sweep: complete with no natural size.
    document.querySelectorAll("img").forEach(function (img) {
        if (
            img.complete &&
            img.naturalWidth === 0 &&
            (img.getAttribute("src") || "").indexOf("/static/") === 0
        ) {
            useNoImageFallback(img);
        }
    });

    // Address-book picker on the checkout delivery step:
    // choosing a saved address folds the typed form away (the
    // server resolves the pick from the id and ignores the
    // typed values); choosing "new" brings it back. Without
    // JS both blocks stay visible and the radio still decides.
    var addressPicker = document.querySelector("[data-address-picker]");
    if (addressPicker) {
        var typedAddress = document.getElementById("typed-address");
        var syncAddressPicker = function () {
            var chosen = addressPicker.querySelector(
                "input[name='delivery_choice']:checked"
            );
            var useTyped = !chosen || chosen.value === "new";
            if (typedAddress) {
                typedAddress.style.display = useTyped ? "" : "none";
                typedAddress
                    .querySelectorAll("input, select, textarea")
                    .forEach(function (el) {
                        el.disabled = !useTyped;
                    });
            }
        };
        addressPicker
            .querySelectorAll("input[name='delivery_choice']")
            .forEach(function (radio) {
                radio.addEventListener("change", syncAddressPicker);
            });
        syncAddressPicker();
    }

    // Logout asks first, with the browser's own confirmation dialog.
    // Cancelling keeps the member logged in: the form never submits.
    // Selecting by the form's action covers every logout form (the
    // header and the member page) without extra classes to maintain.
    document.querySelectorAll('form[action="/logout"]').forEach(function (form) {
        form.addEventListener("submit", function (event) {
            if (!window.confirm("ログアウトしますか？")) {
                event.preventDefault();
            }
        });
    });

    // Payment method choice on the payment step: the summary
    // follows the radio. The server renders both grand totals
    // as data attributes (the only difference between methods
    // is the 330 yen cash-on-delivery fee, and the totals
    // themselves are always computed server-side) — this script
    // only swaps which one is shown and hides the fee row for
    // online methods.
    var paymentOptions = document.querySelector("[data-payment-options]");
    if (paymentOptions) {
        var grandTotal = document.querySelector("[data-grand-total]");
        var codFeeRow = document.querySelector("[data-cod-fee-row]");
        paymentOptions.addEventListener("change", function (event) {
            var isCod = event.target.value === "cod";
            if (grandTotal) {
                grandTotal.textContent = isCod
                    ? grandTotal.dataset.totalCod
                    : grandTotal.dataset.totalOnline;
            }
            if (codFeeRow) {
                codFeeRow.style.display = isCod ? "" : "none";
            }
        });
    }

    // Review deletion asks first, with the same native
    // confirmation as logout (Reviews Spec p.4): cancelling
    // keeps the review, and deletion is never a bare link.
    document.querySelectorAll("form.review-delete-form").forEach(function (form) {
        form.addEventListener("submit", function (event) {
            if (!window.confirm("レビューを削除しますか？")) {
                event.preventDefault();
            }
        });
    });

    // Recent searches (Product Search Spec): the last 5 search terms,
    // kept only in this browser's localStorage. Nothing is sent to
    // the server as history, and guests and members share the same
    // behaviour because the data never leaves the browser.
    var RECENT_KEY = "agentic_recent_searches";
    var RECENT_MAX = 5;

    function loadRecents() {
        try {
            var parsed = JSON.parse(localStorage.getItem(RECENT_KEY) || "[]");
            if (!Array.isArray(parsed)) {
                return [];
            }
            return parsed.filter(function (term) {
                return typeof term === "string" && term !== "";
            });
        } catch (e) {
            return [];
        }
    }

    function saveRecents(list) {
        try {
            localStorage.setItem(RECENT_KEY, JSON.stringify(list));
        } catch (e) {
            // Storage unavailable (private mode, quota): recent
            // searches simply do not persist. Nothing else breaks.
        }
    }

    // Record the term whenever a search results page is viewed:
    // that covers the header form, suggestion clicks and pasted
    // links alike, because all of them land on /search?q=...
    if (window.location.pathname === "/search") {
        var currentQ = new URLSearchParams(window.location.search).get("q");
        if (currentQ && currentQ.trim() !== "") {
            var term = currentQ.trim();
            var recents = loadRecents().filter(function (t) {
                return t !== term;
            });
            recents.unshift(term);
            saveRecents(recents.slice(0, RECENT_MAX));
        }
    }

    // Show the recents under the header search box while the box is
    // focused and empty. Typing hands the same dropdown over to the
    // htmx suggestions; clearing the text brings the recents back.
    var searchInput = document.querySelector(".search-input");
    var suggestBox = document.querySelector(".search-suggestions");
    // The results page lands with the header box holding the
    // term that produced them, the way its own heading does.
    if (window.location.pathname === "/search" && searchInput && !searchInput.value) {
        var landedQ = new URLSearchParams(window.location.search).get("q");
        if (landedQ) {
            searchInput.value = landedQ;
        }
    }
    if (searchInput && suggestBox) {
        function showRecents() {
            if (searchInput.value.trim() !== "") {
                return;
            }
            var recents = loadRecents();
            if (recents.length === 0) {
                return;
            }
            suggestBox.innerHTML = "";
            var ul = document.createElement("ul");
            ul.className = "suggest-list";

            var head = document.createElement("li");
            head.className = "suggest-head";
            var label = document.createElement("span");
            label.textContent = "最近の検索";
            var clear = document.createElement("button");
            clear.type = "button";
            clear.className = "suggest-clear";
            clear.textContent = "履歴を削除";
            clear.addEventListener("click", function () {
                saveRecents([]);
                suggestBox.innerHTML = "";
                searchInput.focus();
            });
            head.appendChild(label);
            head.appendChild(clear);
            ul.appendChild(head);

            // Terms are user input: build nodes with textContent,
            // never innerHTML, so a stored term cannot inject markup.
            recents.forEach(function (recent) {
                var li = document.createElement("li");
                var a = document.createElement("a");
                a.href = "/search?q=" + encodeURIComponent(recent);
                a.textContent = recent;
                li.appendChild(a);
                ul.appendChild(li);
            });
            suggestBox.appendChild(ul);
        }

        searchInput.addEventListener("focus", showRecents);
        searchInput.addEventListener("input", function () {
            if (searchInput.value.trim() === "") {
                showRecents();
            }
        });
        // Hide on blur, unless focus moved into the dropdown itself
        // (a click on a recent search or on the clear button).
        searchInput.addEventListener("blur", function (event) {
            if (event.relatedTarget && suggestBox.contains(event.relatedTarget)) {
                return;
            }
            setTimeout(function () {
                suggestBox.innerHTML = "";
            }, 150);
        });
    }

    // The quantity stepper next to the add button: the detail
    // page's buybox and the quick-add modal share the markup.
    // Buttons only adjust the number input; the form itself is
    // submitted (via htmx) by the add button, and the server
    // enforces the same limits again. Steppers also arrive after
    // load (the modal fragment is swapped in), so they initialise
    // on arrival like the variant pickers, once each.
    function initStepper(box) {
        if (box.getAttribute("data-stepper-init")) {
            return;
        }
        box.setAttribute("data-stepper-init", "1");
        var input = box.querySelector("input[name='quantity']");
        if (!input) {
            return;
        }
        box.querySelectorAll("button[data-step]").forEach(function (btn) {
            btn.addEventListener("click", function () {
                var delta = parseInt(btn.getAttribute("data-step"), 10) || 0;
                var max = parseInt(input.getAttribute("max"), 10) || 99;
                var value = parseInt(input.value, 10) || 1;
                input.value = Math.min(Math.max(1, value + delta), max);
            });
        });
    }
    document.querySelectorAll(".qty-stepper").forEach(initStepper);
    // The variation picker: one shared module for the detail
    // page and the quick-add modal. One option
    // per axis; the embedded data block carries the axes/options
    // and every combination's id, signature and stock. The
    // selection drives the pills, the selected combination's
    // stock label, the quantity cap, the add button and the
    // hidden variant_id the add form posts. The server
    // re-validates the choice on submit.
    function initVariantPicker(picker) {
        if (picker.getAttribute("data-variant-init")) {
            return;
        }
        picker.setAttribute("data-variant-init", "1");
        var dataEl = picker.querySelector("[data-variant-data]");
        if (!dataEl) {
            return;
        }
        var data;
        try {
            data = JSON.parse(dataEl.textContent);
        } catch (e) {
            return;
        }
        var axes = data.axes || [];
        var variants = data.variants || [];
        var bySignature = {};
        variants.forEach(function (v) {
            bySignature[v.signature] = v;
        });
        var optionIdsOf = function (v) {
            return v.signature === ""
                ? []
                : v.signature.split("-").map(function (s) {
                      return parseInt(s, 10);
                  });
        };
        var selected = {};
        axes.forEach(function (axis) {
            selected[axis.id] = null;
        });
        var trackStock = data.trackStock !== false;
        var allowBackorder = data.allowBackorder === true;
        var threshold = typeof data.threshold === "number" ? data.threshold : 3;
        function sellable(v) {
            return (
                v.enabled !== false &&
                (v.stock > 0 || !trackStock || allowBackorder)
            );
        }
        // Initial selection: the first sellable combination.
        for (var i = 0; i < variants.length; i++) {
            if (sellable(variants[i])) {
                var ids = optionIdsOf(variants[i]);
                axes.forEach(function (axis, ai) {
                    selected[axis.id] = ids[ai];
                });
                break;
            }
        }
        var card = picker.closest("[data-buy-area]") || picker.parentElement;
        var form = card ? card.querySelector("form.detail-add-form") : null;
        var variantInput = form ? form.querySelector("[data-variant-input]") : null;
        var qtyInput = form ? form.querySelector("input[name='quantity']") : null;
        var addButton = form ? form.querySelector("[data-add-button]") : null;
        var hint = picker.querySelector("[data-variant-hint]");
        var isPreorder = picker.getAttribute("data-is-preorder") === "1";
        var labelEl = card ? card.querySelector("[data-variant-stock-label]") : null;
        var labelText = card ? card.querySelector("[data-variant-stock-text]") : null;

        function currentVariant() {
            var parts = [];
            for (var ai = 0; ai < axes.length; ai++) {
                var sel = selected[axes[ai].id];
                if (sel === null || sel === undefined) {
                    return null;
                }
                parts.push(sel);
            }
            return bySignature[parts.join("-")] || null;
        }

        function optionEnabled(axisIdx, optionId) {
            // An option stays enabled while some combination
            // carrying it — consistent with the other axes'
            // current selections — still has stock.
            return variants.some(function (v) {
                if (!sellable(v)) {
                    return false;
                }
                var ids = optionIdsOf(v);
                if (ids[axisIdx] !== optionId) {
                    return false;
                }
                return axes.every(function (axis, ai) {
                    return (
                        ai === axisIdx ||
                        selected[axis.id] === null ||
                        ids[ai] === selected[axis.id]
                    );
                });
            });
        }

        function stockWording(variant) {
            if (variant && variant.enabled === false) {
                return "選べません";
            }
            var stock = variant ? variant.stock : 0;
            if (!trackStock) {
                return "在庫あり";
            }
            if (isPreorder && stock > 0) {
                return "予約受付中";
            }
            if (stock === 0) {
                return allowBackorder ? "入荷次第発送" : "売り切れ";
            }
            if (stock <= threshold) {
                return "残りわずか（あと" + stock + "点）";
            }
            return "在庫あり";
        }
        function yen(n) {
            return "¥" + Number(n).toLocaleString("ja-JP");
        }

        function render() {
            picker.querySelectorAll(".variant-group").forEach(function (group, axisIdx) {
                group.querySelectorAll("[data-option-id]").forEach(function (pill) {
                    var optionId = parseInt(pill.getAttribute("data-option-id"), 10);
                    pill.classList.toggle(
                        "selected",
                        selected[axes[axisIdx].id] === optionId
                    );
                    pill.disabled = !optionEnabled(axisIdx, optionId);
                });
            });
            var missing = axes
                .filter(function (axis) {
                    return selected[axis.id] === null;
                })
                .map(function (axis) {
                    return axis.name;
                });
            var variant = currentVariant();
            if (hint) {
                hint.textContent = missing.length
                    ? missing.join("と") + "を選択してください"
                    : "";
            }
            if (variantInput && variant) {
                variantInput.value = variant.id;
            }
            if (qtyInput && variant) {
                var cap =
                    !trackStock || allowBackorder
                        ? 999
                        : Math.max(variant.stock, 1);
                qtyInput.setAttribute("max", String(cap));
                var q = parseInt(qtyInput.value, 10) || 1;
                qtyInput.value = Math.min(Math.max(1, q), cap);
            }
            if (addButton) {
                addButton.disabled = !variant || !sellable(variant);
            }
            var priceEl = card ? card.querySelector("[data-price-display]") : null;
            if (priceEl) {
                var price =
                    variant && variant.price != null
                        ? variant.price
                        : data.productPrice;
                if (typeof price === "number") {
                    priceEl.textContent = yen(price);
                }
            }
            // The price decomposition under the detail price
            // follows the selected combination's price, using
            // the rate and rounding the server rendered with:
            // the same div-round rule as pricing.rs, so the
            // figures never disagree with the server's.
            var taxDetail = card
                ? card.querySelector(".price-tax-detail")
                : null;
            var exclEl = card ? card.querySelector("[data-price-excl]") : null;
            var taxEl = card ? card.querySelector("[data-price-tax]") : null;
            if (taxDetail && (exclEl || taxEl) && typeof price === "number") {
                var taxRate = parseInt(
                    taxDetail.getAttribute("data-tax-rate") || "0",
                    10
                );
                var taxRounding = taxDetail.getAttribute("data-tax-rounding");
                var divRound = function (n, d) {
                    if (d <= 0 || n <= 0) return 0;
                    if (taxRounding === "ceil")
                        return Math.floor((n + d - 1) / d);
                    if (taxRounding === "round")
                        return Math.floor((n + Math.floor(d / 2)) / d);
                    return Math.floor(n / d);
                };
                var exclPart =
                    taxRate > 0 ? divRound(price * 100, 100 + taxRate) : price;
                if (exclEl) exclEl.textContent = yen(exclPart);
                if (taxEl) taxEl.textContent = yen(price - exclPart);
            }
            var galleryImg = card
                ? card.querySelector("[data-gallery-main]")
                : null;
            if (galleryImg && variant && variant.image) {
                galleryImg.setAttribute("src", variant.image);
            }
            if (labelEl && labelText) {
                var stock = variant ? variant.stock : 0;
                labelText.textContent = stockWording(variant);
                labelEl.classList.toggle(
                    "out",
                    trackStock && stock === 0 && !allowBackorder
                );
                labelEl.classList.toggle(
                    "low",
                    !isPreorder && stock > 0 && stock <= threshold
                );
            }
        }

        picker.querySelectorAll(".variant-group").forEach(function (group, axisIdx) {
            group.querySelectorAll("[data-option-id]").forEach(function (pill) {
                pill.addEventListener("click", function () {
                    selected[axes[axisIdx].id] = parseInt(
                        pill.getAttribute("data-option-id"),
                        10
                    );
                    render();
                });
            });
        });
        render();
    }

    document.querySelectorAll("[data-variant-picker]").forEach(initVariantPicker);
    // Pickers and steppers also arrive after load: the quick-add
    // modal fragment is swapped in by htmx, so swapped-in ones are
    // initialised on arrival (the init flags keep it once-only).
    document.body.addEventListener("htmx:afterSwap", function (evt) {
        var root = evt.detail.target;
        if (root && root.querySelectorAll) {
            root.querySelectorAll("[data-variant-picker]").forEach(initVariantPicker);
            root.querySelectorAll(".qty-stepper").forEach(initStepper);
        }
    });

    // Quick-add modal: the listing cards'
    // 選択する button is an anchor to the detail page (the no-JS
    // fallback); htmx loads the picker fragment into the modal
    // shell in the layout, and this module opens and closes the
    // shell. A successful add from inside the modal shows the
    // added state on the button, then the modal closes; the
    // response's out-of-band badge and toast apply as usual.
    var quickAddModal = document.querySelector("[data-quick-add-modal]");
    function openQuickAdd() {
        if (quickAddModal) {
            quickAddModal.hidden = false;
        }
    }
    function closeQuickAdd() {
        if (quickAddModal) {
            quickAddModal.hidden = true;
            var body = quickAddModal.querySelector("[data-quick-add-body]");
            if (body) {
                body.innerHTML = "";
            }
        }
    }
    document.addEventListener("click", function (event) {
        if (event.target.closest("[data-quick-add-open]")) {
            openQuickAdd();
        }
        if (event.target.closest("[data-quick-add-close]")) {
            closeQuickAdd();
        }
    });
    document.addEventListener("keydown", function (event) {
        if (event.key === "Escape") {
            closeQuickAdd();
        }
    });
    document.body.addEventListener("htmx:afterRequest", function (evt) {
        var form = evt.detail.elt;
        if (
            form &&
            form.hasAttribute &&
            form.hasAttribute("data-quick-add-form") &&
            evt.detail.successful
        ) {
            var btn = form.querySelector("[data-add-button]");
            if (btn) {
                btn.textContent = "カートに追加しました";
            }
            setTimeout(closeQuickAdd, 700);
        }
    });

    // Checkout coupon dropdown: choosing a held coupon
    // applies it at once. requestSubmit names the apply
    // button as the submitter, so its intent value reaches
    // the server and the multiple-click guard sees a real
    // submit; a bare form.submit() would send no intent and
    // skip the guard entirely. Without JavaScript the same
    // selection still applies through the apply button (the
    // server prefers a typed code, then the selection).
    var couponSelect = document.querySelector("select[name='coupon_select']");
    if (couponSelect) {
        couponSelect.addEventListener("change", function () {
            if (couponSelect.value === "") {
                return;
            }
            var form = couponSelect.closest("form");
            var applyButton = form
                ? form.querySelector("button[name='intent'][value='apply_coupon']")
                : null;
            if (form && applyButton) {
                form.requestSubmit(applyButton);
            }
        });
    }

    // Account menu (phone header): the account icon toggles
    // the dropdown panel holding the account links. Outside
    // phone widths the panel is transparent and the toggle
    // hidden, so this only ever has an effect on phones. The
    // panel closes on an outside tap, on Escape, and after a
    // tap inside it (its links navigate, and the logout form
    // submits, exactly as before).
    var accountToggle = document.querySelector(".account-toggle");
    var accountPanel = document.getElementById("account-panel");
    if (accountToggle && accountPanel) {
        var closeAccountPanel = function () {
            accountPanel.classList.remove("open");
            accountToggle.setAttribute("aria-expanded", "false");
        };
        accountToggle.addEventListener("click", function () {
            var open = accountPanel.classList.toggle("open");
            accountToggle.setAttribute(
                "aria-expanded",
                open ? "true" : "false",
            );
        });
        document.addEventListener("click", function (event) {
            if (
                accountPanel.classList.contains("open") &&
                !event.target.closest(".account-menu")
            ) {
                closeAccountPanel();
            }
        });
        accountPanel.addEventListener("click", function (event) {
            if (event.target.closest("a, button")) {
                closeAccountPanel();
            }
        });
        document.addEventListener("keydown", function (event) {
            if (event.key === "Escape") {
                closeAccountPanel();
            }
        });
    }

    // The search filter form sits in a <details> that ships
    // open, so with no JavaScript it is simply always visible.
    // With JavaScript the disclosure follows the layout: open
    // as the inline filter row on wide screens, closed behind
    // its summary on phone-width screens, where the
    // active-filter chips carry the current state. The sync
    // runs on load and when the width crosses the breakpoint;
    // the summary itself still toggles the form by hand.
    var filterDetails = document.querySelector("details.search-filters");
    if (filterDetails) {
        var filterMq = window.matchMedia("(max-width: 640px)");
        var syncFilterDetails = function () {
            filterDetails.open = !filterMq.matches;
        };
        syncFilterDetails();
        if (filterMq.addEventListener) {
            filterMq.addEventListener("change", syncFilterDetails);
        }
    }

    // Product detail buy bar (phone widths): the bar mirrors
    // the buy card — the picker's price is copied across as it
    // changes, and the bar's button forwards its tap to the buy
    // card's own add button. The bar holds no form and no state
    // of its own, so there is no second purchase path; its
    // disabled state follows the real button and the form's
    // in-flight marker, keeping the multiple-click guard exact.
    var buyBar = document.querySelector(".buy-bar");
    if (buyBar && window.MutationObserver) {
        var buyPrice = document.querySelector("[data-price-display]");
        var barPrice = buyBar.querySelector("[data-buy-bar-price]");
        if (buyPrice && barPrice) {
            new MutationObserver(function () {
                barPrice.textContent = buyPrice.textContent;
            }).observe(buyPrice, { childList: true, subtree: true });
        }
        var barButton = buyBar.querySelector("[data-buy-bar-button]");
        var realButton = document.querySelector("[data-add-button]");
        if (barButton && realButton) {
            var buyForm = realButton.closest("form");
            var syncBarButton = function () {
                barButton.disabled =
                    realButton.disabled ||
                    (buyForm !== null &&
                        buyForm.classList.contains("htmx-request"));
            };
            syncBarButton();
            new MutationObserver(syncBarButton).observe(realButton, {
                attributes: true,
                attributeFilter: ["disabled"],
            });
            if (buyForm) {
                new MutationObserver(syncBarButton).observe(buyForm, {
                    attributes: true,
                    attributeFilter: ["class"],
                });
            }
            barButton.addEventListener("click", function () {
                realButton.click();
            });
        }
        // PC widths: the bar is fixed (see design.css) and
        // slides in only once the buy card has scrolled out
        // of view. On phones the bar is the in-flow sticky
        // one and the class has no styles there.
        var buyArea = document.querySelector("[data-buy-area]");
        if (buyArea && window.IntersectionObserver) {
            new IntersectionObserver(function (entries) {
                buyBar.classList.toggle(
                    "is-visible",
                    !entries[0].isIntersecting
                );
            }).observe(buyArea);
        }
    }
});

    // Multiple-click proofing, one rule for every submission
    // in the app (design review D11, since made uniform): while
    // a request is in flight its submit buttons are disabled,
    // and they are re-enabled when the request completes --
    // success or failure, whether the answer is swapped in,
    // targets another element or is not swapped at all
    // (hx-swap="none"). A swapped-in answer brings fresh,
    // enabled controls of its own; re-enabling the old ones only
    // matters for the controls still on the page, and the
    // detached rest are unaffected. Only buttons this guard
    // disabled are marked and later re-enabled, so a button the
    // server rendered disabled (a sold-out stepper, say) keeps
    // its state.
    function setSubmitters(form, disabled) {
        form.querySelectorAll("button[type=submit], input[type=submit]").forEach(function (b) {
            if (disabled) {
                if (!b.disabled) {
                    b.dataset.guardDisabled = "1";
                    b.disabled = true;
                }
            } else if (b.dataset.guardDisabled === "1") {
                delete b.dataset.guardDisabled;
                b.disabled = false;
            }
        });
    }

    function setOne(elt, disabled) {
        if (disabled) {
            if (!elt.disabled) {
                elt.dataset.guardDisabled = "1";
                elt.disabled = true;
            }
        } else if (elt.dataset.guardDisabled === "1") {
            delete elt.dataset.guardDisabled;
            elt.disabled = false;
        }
    }

    // The element an htmx request belongs to. htmx reports it
    // as detail.elt on every request event (its triggerEvent
    // stamps the element it dispatches on into the detail
    // before dispatching), so the guard keys on that rather
    // than on event.target.
    function requestElement(event) {
        return (event.detail && event.detail.elt) || event.target;
    }

    function isSubmitter(elt) {
        return (elt.tagName === "BUTTON" && elt.type === "submit") ||
            (elt.tagName === "INPUT" && (elt.type === "submit" || elt.type === "image"));
    }

    // Applies the guard to one request element: a form's own
    // submitters; for a standalone submit control carrying its
    // own request (none in the current templates -- the rule is
    // uniform anyway), the control itself plus its form's
    // submitters. Any other element (the search box's
    // suggestion requests, the cart fragment's self-restore)
    // has no submitter to protect and is left alone.
    function guardRequest(event, disabled) {
        var elt = requestElement(event);
        if (!elt || !elt.tagName) {
            return;
        }
        if (elt.tagName === "FORM") {
            setSubmitters(elt, disabled);
        } else if (isSubmitter(elt)) {
            setOne(elt, disabled);
            var form = elt.closest ? elt.closest("form") : null;
            if (form) {
                setSubmitters(form, disabled);
            }
        }
    }

    document.addEventListener("htmx:beforeRequest", function (event) {
        guardRequest(event, true);
    });
    // htmx fires afterRequest on every way a request can end --
    // answer received, network error, abort, timeout -- so this
    // one listener is the whole re-enable rule, with no
    // condition left to go stale.
    document.addEventListener("htmx:afterRequest", function (event) {
        guardRequest(event, false);
    });

    // Plain (non-htmx) forms post the whole page; they get the
    // same rule in native form. The first submit marks the
    // form, and a repeat submit while the browser is still
    // working on the first is stopped outright. The buttons
    // disable a tick later, after the form's data is built, so
    // the clicked button's own name/value (the checkout form's
    // intent, say) still reaches the server. A submit that was
    // already cancelled -- a declined confirmation dialog,
    // above all -- never marks or disables anything, and htmx
    // forms are left to the pair above (htmx cancels their
    // native submit itself).
    document.addEventListener("submit", function (event) {
        var form = event.target;
        if (!form || form.tagName !== "FORM") {
            return;
        }
        if (event.defaultPrevented) {
            return;
        }
        if (form.hasAttribute("hx-post") || form.hasAttribute("hx-get") ||
            form.hasAttribute("hx-put") || form.hasAttribute("hx-patch") ||
            form.hasAttribute("hx-delete")) {
            return;
        }
        if (form.dataset.submitting === "1") {
            event.preventDefault();
            return;
        }
        form.dataset.submitting = "1";
        setTimeout(function () {
            setSubmitters(form, true);
        }, 0);
    });

    // A page restored from the back/forward cache keeps its
    // DOM, marks included: clear the guard's state so nothing
    // restored from history sits disabled.
    window.addEventListener("pageshow", function (event) {
        if (!event.persisted) {
            return;
        }
        document.querySelectorAll("form[data-submitting]").forEach(function (form) {
            delete form.dataset.submitting;
            setSubmitters(form, false);
        });
        document.querySelectorAll("[data-guard-disabled]").forEach(function (b) {
            delete b.dataset.guardDisabled;
            b.disabled = false;
        });
    });

    // Add-to-cart toast: the confirmation toast
    // (parts/cart_toast.html) shows for 3 seconds, then fades
    // out; its close button dismisses it at once. A toast
    // exists only in the add endpoint's answer, swapped in
    // out-of-band as a .toast inside #toast-region; a later
    // page load never re-renders it. The htmx swap replaces
    // the whole region element, so the observer watches
    // <body>'s child list (the region is a direct child)
    // rather than the region itself; a fresh toast node
    // restarts the timer, so adding again while a toast is up
    // replaces the message AND the 3 seconds.
    var TOAST_VISIBLE_MS = 3000;
    var toastTimer = null;
    var toastFadeTimer = null;
    var armedToast = null;

    function currentToast() {
        var region = document.getElementById("toast-region");
        return region ? region.querySelector(".toast") : null;
    }

    function dismissToast() {
        var toast = currentToast();
        if (toastTimer) {
            clearTimeout(toastTimer);
            toastTimer = null;
        }
        if (!toast) {
            armedToast = null;
            return;
        }
        // Reduced motion: no fade, the toast just disappears.
        if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
            toast.remove();
            armedToast = null;
            return;
        }
        toast.classList.add("toast-hide");
        if (toastFadeTimer) {
            clearTimeout(toastFadeTimer);
        }
        toastFadeTimer = setTimeout(function () {
            toast.remove();
            if (armedToast === toast) {
                armedToast = null;
            }
        }, 300);
    }

    function armToast() {
        var toast = currentToast();
        if (!toast) {
            armedToast = null;
            return;
        }
        if (toast === armedToast) {
            return;
        }
        armedToast = toast;
        if (toastTimer) {
            clearTimeout(toastTimer);
        }
        if (toastFadeTimer) {
            clearTimeout(toastFadeTimer);
        }
        toast.classList.remove("toast-hide");
        toastTimer = setTimeout(dismissToast, TOAST_VISIBLE_MS);
    }

    if ("MutationObserver" in window && document.body) {
        new MutationObserver(armToast).observe(document.body, { childList: true });
    }
    document.addEventListener("click", function (event) {
        if (event.target.closest && event.target.closest(".toast-close")) {
            dismissToast();
        }
    });

    // htmx failures were silent: an error status or a dropped
    // connection left the page looking untouched, so a shopper
    // could believe an add-to-cart went through. Show the same
    // toast the add endpoint returns, built here with a generic
    // retry message; the region's aria-live announces it too.
    function showErrorToast() {
        var region = document.getElementById("toast-region");
        if (!region) {
            return;
        }
        var toast = document.createElement("div");
        toast.className = "toast";
        toast.setAttribute("role", "status");
        var text = document.createElement("span");
        text.className = "toast-text";
        text.textContent = "通信に失敗しました。時間をおいて再度お試しください。";
        var close = document.createElement("button");
        close.type = "button";
        close.className = "toast-close";
        close.setAttribute("aria-label", "閉じる");
        close.textContent = "×";
        toast.appendChild(text);
        toast.appendChild(close);
        region.replaceChildren(toast);
        armToast();
    }

    document.addEventListener("htmx:responseError", showErrorToast);
    document.addEventListener("htmx:sendError", showErrorToast);

    // Search suggestions: keyboard navigation (design review
    // D5). Arrow keys move an active row, Enter follows it,
    // Escape closes the list. The list itself is still the
    // server-rendered fragment; this only adds the interaction.
    document.querySelectorAll(".search-input").forEach(function (input) {
        var wrap = input.closest(".search-form");
        var box = wrap ? wrap.querySelector(".search-suggestions") : null;
        if (!box) {
            return;
        }
        var active = -1;

        function rows() {
            return Array.prototype.slice.call(box.querySelectorAll(".suggest-list a"));
        }
        function setActive(index) {
            var items = rows();
            active = index;
            items.forEach(function (a, i) {
                a.classList.toggle("suggest-active", i === active);
                a.setAttribute("aria-selected", i === active ? "true" : "false");
            });
            input.setAttribute("aria-expanded", items.length > 0 ? "true" : "false");
            if (active >= 0 && items[active]) {
                input.setAttribute("aria-activedescendant", items[active].id || "");
            } else {
                input.removeAttribute("aria-activedescendant");
            }
        }
        box.addEventListener("htmx:afterSwap", function () {
            setActive(-1);
        });
        input.addEventListener("keydown", function (event) {
            var items = rows();
            if (event.key === "ArrowDown" && items.length > 0) {
                event.preventDefault();
                setActive(Math.min(active + 1, items.length - 1));
            } else if (event.key === "ArrowUp" && items.length > 0) {
                event.preventDefault();
                setActive(Math.max(active - 1, -1));
            } else if (event.key === "Enter" && active >= 0 && items[active]) {
                event.preventDefault();
                window.location.href = items[active].href;
            } else if (event.key === "Escape") {
                box.innerHTML = "";
                setActive(-1);
            }
        });
        document.addEventListener("click", function (event) {
            if (wrap && !wrap.contains(event.target)) {
                box.innerHTML = "";
                setActive(-1);
            }
        });
    });

// Image derivative fallback: card, quick-add and
// detail images name a resized derivative as their src and
// carry the original in data-fallback-src. A derivative can
// be missing — an upload from before derivatives existed,
// or one whose generation failed — and the error event does
// not bubble, so this capture-phase listener swaps the
// original in, once per image.
document.addEventListener(
    "error",
    function (event) {
        var img = event.target;
        if (
            img &&
            img.tagName === "IMG" &&
            img.getAttribute("data-fallback-src") &&
            !img.hasAttribute("data-fallback-used")
        ) {
            img.setAttribute("data-fallback-used", "");
            img.src = img.getAttribute("data-fallback-src");
        }
    },
    true
);

// Product detail gallery: clicking a thumbnail swaps the main
// image and moves the active outline. Single-image pages
// render no strip, so this only ever runs on galleries.
document.addEventListener("DOMContentLoaded", function () {
    var main = document.getElementById("detail-main-image");
    if (!main) {
        return;
    }
    var thumbs = document.querySelectorAll(".detail-thumb");
    thumbs.forEach(function (thumb) {
        thumb.addEventListener("click", function () {
            main.src = thumb.getAttribute("data-full");
            main.setAttribute(
                "data-fallback-src",
                thumb.getAttribute("data-full-fallback")
            );
            main.setAttribute(
                "data-lightbox-src",
                thumb.getAttribute("data-full-fallback")
            );
            thumbs.forEach(function (other) {
                other.classList.toggle("is-active", other === thumb);
            });
        });
    });
});

// Product image lightbox: clicking the main gallery image
// opens a full-screen overlay with the current image. Pure
// presentation over what the page already shows — the
// thumbnails keep swapping the main image, and the overlay
// reads whichever source is current when it opens. Esc,
// the backdrop and the close button all dismiss it.
document.addEventListener("DOMContentLoaded", function () {
    var main = document.querySelector("[data-gallery-main]");
    if (!main) {
        return;
    }
    var overlay = document.createElement("div");
    overlay.className = "lightbox";
    overlay.hidden = true;
    overlay.innerHTML =
        '<button type="button" class="lightbox-close" aria-label="閉じる">\u00d7</button><img alt="">';
    document.body.appendChild(overlay);
    var big = overlay.querySelector("img");
    function open() {
        big.src =
            main.getAttribute("data-lightbox-src") || main.getAttribute("src");
        big.alt = main.getAttribute("alt") || "";
        overlay.hidden = false;
        document.body.classList.add("lightbox-open");
    }
    function close() {
        overlay.hidden = true;
        document.body.classList.remove("lightbox-open");
    }
    main.addEventListener("click", open);
    overlay.addEventListener("click", function (event) {
        if (event.target !== big) {
            close();
        }
    });
    document.addEventListener("keydown", function (event) {
        if (event.key === "Escape" && !overlay.hidden) {
            close();
        }
    });
});

// Shopping-agent chat: a sent message
// appears at once, as in any chat app. On submit — in the
// capture phase, before htmx reads the form — this appends a
// pending turn (the visitor's own bubble, then the agent's
// typing bubble) and points the form's swap at it, so the
// server's fragment replaces the pair wholesale when the
// answer lands. The composer itself is never swapped, so its
// per-session token stays valid across turns; if the request
// fails, the typing bubble becomes a short error note in its
// place. Enter sends; Shift+Enter keeps the newline, and an
// IME composition never sends (Japanese input confirms with
// Enter).
var chatTurnSeq = 0;

// One agent typing row — three animated dots in an agent
// bubble — shared by the pending turn and the staged picks
// reveal, so both waits look and announce the same way.
function buildChatTypingRow(label) {
    var row = document.createElement("div");
    row.className = "chat-row chat-row-agent";
    var typing = document.createElement("div");
    typing.className = "chat-bubble chat-bubble-agent chat-bubble-typing";
    typing.setAttribute("role", "status");
    typing.setAttribute("aria-label", label);
    for (var i = 0; i < 3; i += 1) {
        var dot = document.createElement("span");
        dot.className = "chat-dot";
        typing.appendChild(dot);
    }
    row.appendChild(typing);
    return row;
}

document.addEventListener("submit", function (event) {
    var form = event.target;
    if (!form || form.tagName !== "FORM") {
        return;
    }
    if (form.getAttribute("hx-post") !== "/agent/chat") {
        return;
    }
    var thread = document.getElementById("chat-thread");
    if (!thread) {
        return;
    }
    var area = form.querySelector("textarea[name='message']");
    var hidden = form.querySelector("input[name='message']");
    var message = area ? area.value : hidden ? hidden.value : "";
    if (!message.trim()) {
        return;
    }
    if (form.chatPending && form.chatPending.parentNode) {
        form.chatPending.parentNode.removeChild(form.chatPending);
    }
    chatTurnSeq += 1;
    var pending = document.createElement("div");
    pending.className = "chat-pending chat-arrive";
    pending.id = "chat-pending-" + chatTurnSeq;

    var userRow = document.createElement("div");
    userRow.className = "chat-row chat-row-user";
    var userBubble = document.createElement("div");
    userBubble.className = "chat-bubble chat-bubble-user";
    userBubble.textContent = message;
    userRow.appendChild(userBubble);
    pending.appendChild(userRow);

    pending.appendChild(buildChatTypingRow("エージェントが回答を作成しています"));

    thread.appendChild(pending);
    form.chatPending = pending;
    form.chatTurn = { pending: pending, message: message };
    form.chatSwapTarget = form.getAttribute("hx-target");
    form.chatSwapMode = form.getAttribute("hx-swap");
    form.setAttribute("hx-target", "#" + pending.id);
    form.setAttribute("hx-swap", "outerHTML");
    pending.scrollIntoView({ behavior: "smooth", block: "end" });
}, true);

// htmx announces the request only after it has gathered the
// form's values, so the composer can clear at send time —
// the text lives on in the pending bubble — instead of
// sitting filled while the agent thinks. A failed turn puts
// the text back (chatTurnFailed), ready to retry.
document.addEventListener("htmx:beforeRequest", function (event) {
    var form = event.detail && event.detail.elt;
    if (!form || form.tagName !== "FORM" || !form.chatPending) {
        return;
    }
    var area = form.querySelector("textarea[name='message']");
    if (area) {
        area.value = "";
    }
});

// What the thread held just before a swap landed, so the
// arrival animation touches only the entries that are new.
var chatThreadBefore = null;

function chatSwapEvent(event) {
    var target = event.detail && event.detail.target;
    if (!target) {
        return false;
    }
    return target.id === "chat-thread" ||
        (target.classList && target.classList.contains("chat-pending"));
}

// A live answer that carries picks is staged: the reply text lands first, then
// the pick cards follow as their own message after a short
// typing beat — the way a chat app delivers two messages —
// instead of everything swapping in as one lump. Only the
// optimistic path stages, because it alone swaps into a
// pending turn; history, thread resets and the no-JS path
// render whole, as before. The picks are the server's own
// already-processed nodes, held back behind a typing row
// and put back where they were; apply buttons and OOB
// updates are untouched. A thread reset in the meantime
// drops the reveal — the persisted history still holds the
// answer whole.
function stageChatPicks(thread, before) {
    var held = [];
    Array.prototype.forEach.call(thread.children, function (child) {
        if (before.indexOf(child) !== -1) {
            return;
        }
        if (child.classList && child.classList.contains("chat-result") && child.querySelector(".agent-pick")) {
            var holder = buildChatTypingRow("おすすめ商品を準備しています");
            child.parentNode.insertBefore(holder, child);
            child.parentNode.removeChild(child);
            held.push({ holder: holder, node: child });
        }
    });
    if (!held.length) {
        return;
    }
    var reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    var delay = reduced ? 0 : 900;
    held.forEach(function (pair, idx) {
        setTimeout(function () {
            if (!pair.holder.parentNode) {
                return;
            }
            pair.node.classList.add("chat-arrive");
            pair.holder.parentNode.replaceChild(pair.node, pair.holder);
            pair.node.scrollIntoView({ behavior: "smooth", block: "end" });
        }, delay + idx * 450);
    });
}

document.addEventListener("htmx:beforeSwap", function (event) {
    if (!chatSwapEvent(event)) {
        return;
    }
    var thread = document.getElementById("chat-thread");
    chatThreadBefore = thread ? Array.prototype.slice.call(thread.children) : null;
});

document.addEventListener("htmx:afterSwap", function (event) {
    if (!chatSwapEvent(event)) {
        return;
    }
    var thread = document.getElementById("chat-thread");
    if (!thread) {
        return;
    }
    var textarea = document.querySelector(".chat-input-bar textarea");
    if (textarea) {
        textarea.value = "";
    }
    var examples = document.getElementById("chat-examples");
    if (examples) {
        examples.hidden = true;
    }
    var reset = document.getElementById("chat-reset-form");
    if (reset) {
        reset.hidden = false;
    }
    var wasPending = !!(event.detail.target && event.detail.target.classList &&
        event.detail.target.classList.contains("chat-pending"));
    if (wasPending && chatThreadBefore) {
        stageChatPicks(thread, chatThreadBefore);
    }
    if (chatThreadBefore) {
        Array.prototype.forEach.call(thread.children, function (child) {
            if (chatThreadBefore.indexOf(child) === -1) {
                child.classList.add("chat-arrive");
            }
        });
        chatThreadBefore = null;
    }
    if (thread.lastElementChild) {
        thread.lastElementChild.scrollIntoView({ behavior: "smooth", block: "end" });
    }
});

// Once a form's pending turn has been answered (or given up
// on), the form's swap goes back to the whole thread.
document.addEventListener("htmx:afterRequest", function (event) {
    var form = event.detail && event.detail.elt;
    if (!form || form.tagName !== "FORM" || !form.chatPending) {
        return;
    }
    if (form.chatSwapTarget) {
        form.setAttribute("hx-target", form.chatSwapTarget);
    }
    if (form.chatSwapMode) {
        form.setAttribute("hx-swap", form.chatSwapMode);
    }
    if (!event.detail || !event.detail.failed) {
        form.chatTurn = null;
    }
    form.chatPending = null;
    form.chatSwapTarget = null;
    form.chatSwapMode = null;
});

// A request that never produced an answer: the visitor's
// bubble stays, the typing bubble becomes the error, and
// the message goes back into the composer so it can be
// retried as-is. This keys off chatTurn, not chatPending:
// on a network error htmx fires afterRequest (which
// restores the swap attributes and clears chatPending)
// before sendError arrives.
function chatTurnFailed(event) {
    var form = event.detail && event.detail.elt;
    if (!form || form.tagName !== "FORM" || !form.chatTurn) {
        return;
    }
    var turn = form.chatTurn;
    form.chatTurn = null;
    var typing = turn.pending.querySelector(".chat-bubble-typing");
    if (typing) {
        typing.classList.remove("chat-bubble-typing");
        typing.classList.add("chat-bubble-error");
        typing.removeAttribute("aria-label");
        typing.textContent = "送信できませんでした。通信状態をご確認のうえ、もう一度お試しください。";
    }
    var area = form.querySelector("textarea[name='message']");
    if (area && turn.message) {
        area.value = turn.message;
    }
    turn.pending.scrollIntoView({ behavior: "smooth", block: "end" });
}

document.addEventListener("htmx:sendError", chatTurnFailed);
document.addEventListener("htmx:responseError", chatTurnFailed);

document.addEventListener("keydown", function (event) {
    var area = event.target;
    if (
        area.matches &&
        area.matches(".chat-input-bar textarea") &&
        event.key === "Enter" &&
        !event.shiftKey &&
        !event.isComposing
    ) {
        event.preventDefault();
        area.form.requestSubmit();
    }
});

document.addEventListener("DOMContentLoaded", function () {
    // Resend countdown: the server enforces a 60-second
    // cooldown between verification-code sends; the button
    // now shows that wait instead of only claiming it.
    // Reaching a verify page means a code was just sent, so
    // the wait starts on render; the stored timestamp keeps
    // a refresh inside the same window honest instead of
    // restarting the full minute.
    document.querySelectorAll("form[data-resend]").forEach(function (form) {
        var button = form.querySelector("[data-resend-button]");
        var wait = form.querySelector("[data-resend-wait]");
        if (!button || !wait) {
            return;
        }
        var key = "resend-wait:" + form.getAttribute("action");
        var started = 0;
        try {
            started = parseInt(localStorage.getItem(key) || "0", 10) || 0;
            if (!started || Date.now() - started > 60000) {
                started = Date.now();
                localStorage.setItem(key, String(started));
            }
        } catch (e) {
            started = Date.now();
        }
        var tick = function () {
            var left = Math.ceil((60000 - (Date.now() - started)) / 1000);
            if (left <= 0) {
                button.disabled = false;
                wait.textContent = "(今すぐ再送できます)";
                return;
            }
            button.disabled = true;
            wait.textContent = "(あと" + left + "秒で再送できます)";
            setTimeout(tick, 1000);
        };
        tick();
    });

    // Recovery codes: copy every code in one click, one per
    // line, ready to paste into a password manager.
    var copyRecovery = document.querySelector("[data-copy-recovery]");
    if (copyRecovery) {
        copyRecovery.addEventListener("click", function () {
            var codes = Array.prototype.map
                .call(
                    document.querySelectorAll(".recovery-codes code"),
                    function (el) {
                        return el.textContent.trim();
                    }
                )
                .join("\n");
            var done = function () {
                var label = copyRecovery.textContent;
                copyRecovery.textContent = "コピーしました";
                setTimeout(function () {
                    copyRecovery.textContent = label;
                }, 2000);
            };
            if (navigator.clipboard && navigator.clipboard.writeText) {
                navigator.clipboard.writeText(codes).then(done, done);
            } else {
                var area = document.createElement("textarea");
                area.value = codes;
                document.body.appendChild(area);
                area.select();
                try {
                    document.execCommand("copy");
                } catch (e) {
                    /* the codes stay selectable by hand */
                }
                document.body.removeChild(area);
                done();
            }
        });
    }
});

// Hero slideshow: crossfade the stacked slides on a timer,
// dots to jump, pause while the reader hovers or focuses,
// and no motion at all for readers who prefer it still.
(() => {
    const slider = document.querySelector("[data-hero-slider]");
    if (!slider) {
        return;
    }
    const slides = Array.from(slider.querySelectorAll(".hero-slide"));
    if (slides.length < 2) {
        return;
    }
    const dots = Array.from(slider.querySelectorAll("[data-hero-dot]"));
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let current = 0;
    let timer = null;
    const show = (index) => {
        current = (index + slides.length) % slides.length;
        slides.forEach((slide, i) => slide.classList.toggle("is-active", i === current));
        dots.forEach((dot, i) => dot.classList.toggle("is-active", i === current));
    };
    const start = () => {
        if (reduceMotion || timer !== null) {
            return;
        }
        timer = window.setInterval(() => show(current + 1), 5000);
    };
    const stop = () => {
        if (timer !== null) {
            window.clearInterval(timer);
            timer = null;
        }
    };
    dots.forEach((dot, i) => {
        dot.addEventListener("click", () => {
            show(i);
            stop();
            start();
        });
    });
    slider.addEventListener("mouseenter", stop);
    slider.addEventListener("mouseleave", start);
    slider.addEventListener("focusin", stop);
    slider.addEventListener("focusout", start);
    // A back/forward-cache restore revives the page without
    // re-running this script, and the interval does not
    // survive it: restart the rotation on the way back in.
    window.addEventListener("pageshow", (event) => {
        if (event.persisted) {
            start();
        }
    });
    start();
})();
