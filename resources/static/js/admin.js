// Back office interactions (vanilla JS, like every script in
// this app). Jobs: native confirmations for destructive
// admin actions, multiple-click proofing for form posts,
// the product image gallery's ordering and upload, the
// linked tax-exclusive / tax-inclusive price inputs, the
// variation editor, the tag picker, the product list's
// bulk-selection bar, the content-page preview, format toggle and meta
// counter, the menu editor's add-link row, the top page
// hero's image picker and live preview, the sidebar
// collapse, the sidebar menu search, the account form's
// role toggle, the coupon form's conditional fields and
// searchable target pickers, the multipart import
// forms' header-token submit, and the shared
// unsaved-changes guard for editor forms.

// Native confirmation dialogs, wired by form action/class the
// same way app.js wires the storefront's confirmations: the
// browser's own dialog keeps the pattern script-light and
// impossible to half-dismiss.
document.addEventListener("submit", (event) => {
    const form = event.target;
    const confirmations = [
        [".admin-logout-form", "ログアウトしますか？"],
        [".review-delete-form", "レビューを削除しますか？"],
        [".segment-delete-form", "このセグメントを削除しますか？定義のみが削除され、会員データには影響しません"],
        [".api-key-revoke-form", "このAPIキーを無効化しますか？このキーを使っているクライアントは接続できなくなります"],
        [".notice-delete-form", "このお知らせを削除しますか？"],
        [".category-delete-form", "このカテゴリーを削除しますか？"],
        [".tag-delete-form", "このタグを削除しますか？商品からも外れます"],
        [".content-page-delete-form", "このページを削除しますか？"],
        [".mail-template-delete-form", "このテンプレートを削除しますか？"],
        [".content-revision-restore-form", "この版の内容に戻しますか？現在の内容は版として保存されます"],
        [".admin-unarchive-form", "この商品のアーカイブを解除しますか？下書きとして戻ります"],
        [".admin-cancel-form", "この注文をキャンセルしますか？商品の在庫が自動で戻ります。"],
        [".campaign-test-form", "テスト送信しますか？保存されている内容が、管理者のメールアドレス宛てに送信されます。"],
        [".notification-test-form", "テスト送信しますか？保存されている内容が、管理者のメールアドレス宛てに送信されます。"],
        [".notification-reset-form", "初期値に戻しますか？保存した文面は破棄されます。"],
        [".coupon-activate-form", "このクーポンを有効化しますか？"],
        [".csv-import-apply-form", "この内容で取り込みますか？"],
        [".csv-import-discard-form", "この取込を破棄しますか？アップロードしたCSVは取り込まれません"],
        [".tax-rate-form", "この税率を登録しますか？発効日以降の注文に適用されます。"],
        [".admin-shipping-resend-form", "発送メールをお客様へ再送信しますか？"],
        [".holiday-delete-form", "この休業日を削除しますか？"],
        [".scenario-disable-form", "このシナリオを無効にしますか？配信中のステップメールは送信されなくなります。"],
        [".review-hide-form", "このレビューを非表示にしますか？"],
    ];
    for (const [selector, message] of confirmations) {
        if (form.matches(selector) && !window.confirm(message)) {
            event.preventDefault();
            return;
        }
    }
    if (form.matches(".admin-archive-form")
        && !window.confirm("この商品をアーカイブしますか？店舗には表示されなくなります")) {
        event.preventDefault();
        return;
    }
    // A refund moves real money through KOMOJU and cannot be
    // undone from this screen, so it confirms like the other
    // destructive actions — naming the amount when one was
    // typed (an empty field refunds whatever balance remains).
    if (form.matches(".admin-refund-form")) {
        const amount = form.querySelector("[name=amount_yen]");
        const raw = amount ? amount.value.trim() : "";
        const message = raw
            ? `この注文を ${raw} 円で返金しますか？返金はKOMOJUを通じて行われます。`
            : "この注文を返金しますか？金額が空欄のため、残り全額が返金されます。返金はKOMOJUを通じて行われます。";
        if (!window.confirm(message)) {
            event.preventDefault();
            return;
        }
    }
    // Order status: only the shipped transition reaches
    // outside the shop — it sends the customer the shipping
    // mail and stamps the carrier/tracking — so only it
    // confirms; the internal moves stay one-click.
    if (form.matches(".admin-status-form")) {
        const status = form.querySelector("[name=status]");
        if (status && status.value === "shipped"
            && !window.confirm("発送済みに変更しますか？お客様へ発送メールが送信されます。")) {
            event.preventDefault();
            return;
        }
    }
    // Change-detecting confirmations: the product form and
    // the staff form confirm only when the fields carrying
    // money or access actually changed against the snapshot
    // taken when the page rendered (further below), so
    // routine edits — a description fix, a name typo — stay
    // friction-free.
    if (form.matches("#admin-product-form")) {
        const before = moneySnapshots.get(form);
        if (before !== undefined && moneySignature(form) !== before
            && !window.confirm("価格・セール価格・在庫のいずれかが変更されています。保存すると店舗に反映されます。よろしいですか？")) {
            event.preventDefault();
            return;
        }
    }
    if (form.matches("[data-role-form]")) {
        const before = roleSnapshots.get(form);
        if (before !== undefined && roleSignature(form) !== before) {
            const deactivating = before.split("|")[1] === "active"
                && roleSignature(form).split("|")[1] !== "active";
            const message = deactivating
                ? "この管理者を無効化しますか？ログインできなくなります。"
                : "この管理者の役割を変更しますか？";
            if (!window.confirm(message)) {
                event.preventDefault();
                return;
            }
        }
    }
    // The bulk bar's archive button carries its own confirm;
    // it is a submit button inside the bulk form, so it is
    // checked on click rather than on submit.
});

// Multiple-click proofing for form posts, the same rule
// app.js applies on the storefront: the first submit marks
// the form and a repeat submit while the browser is still
// working on the first is stopped outright; the submit
// buttons disable a tick later, after the form's data is
// built, so the clicked button's own name/value (the bulk
// bar's action, for one) still reaches the server. This
// listener is registered after the confirmation listener
// above and skips any submit that was already cancelled,
// so declining a confirmation dialog never marks or
// disables a form. Only buttons this guard disabled are
// marked, so a control the server rendered disabled keeps
// its state.
document.addEventListener("submit", (event) => {
    const form = event.target;
    if (!form || form.tagName !== "FORM") {
        return;
    }
    if (event.defaultPrevented) {
        return;
    }
    if (form.dataset.submitting === "1") {
        event.preventDefault();
        return;
    }
    form.dataset.submitting = "1";
    setTimeout(() => {
        form.querySelectorAll("button[type=submit], input[type=submit]").forEach((b) => {
            if (!b.disabled) {
                b.dataset.guardDisabled = "1";
                b.disabled = true;
            }
        });
    }, 0);
});

// A page restored from the back/forward cache keeps its
// DOM, marks included: clear the guard's state so nothing
// restored from history sits disabled.
window.addEventListener("pageshow", (event) => {
    if (!event.persisted) {
        return;
    }
    document.querySelectorAll("form[data-submitting]").forEach((form) => {
        delete form.dataset.submitting;
    });
    document.querySelectorAll("[data-guard-disabled]").forEach((b) => {
        delete b.dataset.guardDisabled;
        b.disabled = false;
    });
});

// Multipart form posts — the CSV imports and the orders
// tracking import — cannot ride a native submit: the CSRF
// middleware never reads a token out of a multipart body
// (its form-body buffer is far smaller than an upload), so
// the injected csrf_token field goes unseen and the post is
// refused as不正なリクエスト. Uploads carry the token in
// the x-csrf-token header instead — the contract the
// product image upload already uses — so these forms are
// sent with fetch and the browser then follows the answer
// exactly as it would have followed the native post: a
// redirect is navigated to, and a rendered page (the
// tracking import's result) replaces this document. The
// multiple-click guard above has already marked the form
// and disabled its buttons by the time this runs; a network
// failure hands both back so the admin can retry.
document.addEventListener("submit", (event) => {
    const form = event.target;
    if (!form || form.tagName !== "FORM") {
        return;
    }
    if ((form.getAttribute("enctype") || "").toLowerCase() !== "multipart/form-data") {
        return;
    }
    if (event.defaultPrevented) {
        return;
    }
    event.preventDefault();
    const tokenInput = form.querySelector('input[name="csrf_token"]');
    fetch(form.action, {
        method: "POST",
        headers: { "x-csrf-token": tokenInput ? tokenInput.value : "" },
        body: new FormData(form),
    })
        .then(async (response) => {
            if (response.redirected) {
                window.location.href = response.url;
                return;
            }
            // A rendered answer (the tracking import's
            // result page): swap the document's contents
            // in place. document.write is not an option —
            // re-parsing would load this script again and
            // its top-level declarations would collide
            // with the running copy — while a parsed swap
            // keeps this document (and with it the
            // delegated listeners above) alive; the answer
            // pages are static reports that lose nothing
            // by not re-running page scripts.
            const html = await response.text();
            const doc = new DOMParser().parseFromString(html, "text/html");
            document.title = doc.title;
            document.documentElement.innerHTML = doc.documentElement.innerHTML;
            window.scrollTo(0, 0);
        })
        .catch(() => {
            delete form.dataset.submitting;
            form.querySelectorAll("[data-guard-disabled]").forEach((b) => {
                delete b.dataset.guardDisabled;
                b.disabled = false;
            });
        });
});

// Bulk bar confirmations (the submit event cannot tell
// which button was pressed, so these listen for clicks):
// publishing puts the products on the storefront and
// drafting takes them off it, so all three state changes
// confirm first, archive included.
document.addEventListener("click", (event) => {
    const messages = {
        "admin-bulk-publish": "選択した商品を公開しますか？店舗に表示されます",
        "admin-bulk-draft": "選択した商品を下書きにしますか？店舗には表示されなくなります",
        "admin-bulk-archive": "選択した商品をアーカイブしますか？店舗には表示されなくなります",
    };
    const message = event.target ? messages[event.target.id] : undefined;
    if (message && !window.confirm(message)) {
        event.preventDefault();
    }
});

// Campaign send confirmation: one form carries both send
// buttons (now / scheduled), a send cannot be taken back,
// and the bare submit event cannot tell the buttons apart —
// so the click confirms first, naming the recipient count
// the confirm page rendered into the form.
document.addEventListener("click", (event) => {
    const button = event.target;
    if (!button || (button.id !== "campaign-send-now" && button.id !== "campaign-send-scheduled")) {
        return;
    }
    const form = button.closest("form");
    const count = form ? form.dataset.recipientCount : "";
    const audience = count ? `メールマガジン購読者 ${count} 名` : "メールマガジン購読者";
    const message = button.id === "campaign-send-now"
        ? `${audience}へ今すぐ配信します。よろしいですか？`
        : `${audience}へ予約配信します。予約を取り消すことはできません。よろしいですか？`;
    if (!window.confirm(message)) {
        event.preventDefault();
    }
});

// Product image gallery: the form's ordered image list. The
// first thumbnail is the main image (badged by CSS position);
// the submitted order of the hidden "images" fields is the
// gallery order. Thumbnails reorder by drag or by the move
// buttons, and the remove button detaches an image —
// detaching never deletes the file from disk (uploads live
// under unique generated names; built-in assets are tracked
// files). Uploads post each chosen file straight to the
// upload endpoint, then append the stored image; the file
// input is unnamed on purpose, so it never posts with the
// product form. The CSRF token rides in the x-csrf-token
// header, read from the hidden field the server injects into
// the product form.
(() => {
    const list = document.getElementById("product-image-list");
    if (!list) return;
    const readonly = list.dataset.readonly === "1";
    const fileInput = document.getElementById("product-image-file");
    const dropzone = document.getElementById("product-image-dropzone");
    const status = document.getElementById("product-image-upload-status");
    const errorBox = document.getElementById("product-image-upload-error");
    const preview = document.getElementById("admin-image-preview-img");
    const emptyNote = document.getElementById("product-image-empty");
    const form = document.getElementById("admin-product-form");
    let uploading = false;

    const items = () => Array.from(list.querySelectorAll(".admin-image-item"));

    // Keeps the side states honest after any list change: the
    // preview shows the main (first) image, and the empty note
    // shows only with no images.
    const sync = () => {
        const current = items();
        if (preview) {
            const first = current[0];
            if (first) {
                preview.src = first.querySelector("img").src;
                preview.hidden = false;
            } else {
                preview.hidden = true;
            }
        }
        if (emptyNote) {
            emptyNote.hidden = current.length > 0;
        }
    };

    const showError = (message) => {
        if (!errorBox) return;
        errorBox.textContent = message;
        errorBox.hidden = false;
    };

    // Builds one gallery item in exactly the server-rendered
    // shape, so delegated handlers treat it the same.
    const buildItem = (path) => {
        const item = document.createElement("div");
        item.className = "admin-image-item";
        if (!readonly) item.draggable = true;
        const img = document.createElement("img");
        img.src = path;
        img.alt = "";
        img.draggable = false;
        const input = document.createElement("input");
        input.type = "hidden";
        input.name = "images";
        input.value = path;
        const alt = document.createElement("input");
        alt.type = "text";
        alt.name = "image_alt";
        alt.value = "";
        alt.placeholder = "代替テキスト（任意）";
        alt.className = "admin-image-alt";
        alt.setAttribute("aria-label", "画像の代替テキスト");
        if (readonly) alt.disabled = true;
        const badge = document.createElement("span");
        badge.className = "admin-image-main-badge";
        badge.textContent = "メイン";
        item.append(img, input, alt, badge);
        if (!readonly) {
            const actions = document.createElement("div");
            actions.className = "admin-image-item-actions";
            const left = document.createElement("button");
            left.type = "button";
            left.className = "text-button";
            left.dataset.moveImage = "-1";
            left.setAttribute("aria-label", "左へ移動");
            left.textContent = "◀";
            const right = document.createElement("button");
            right.type = "button";
            right.className = "text-button";
            right.dataset.moveImage = "1";
            right.setAttribute("aria-label", "右へ移動");
            right.textContent = "▶";
            const remove = document.createElement("button");
            remove.type = "button";
            remove.className = "text-button";
            remove.dataset.removeImage = "";
            remove.setAttribute("aria-label", "画像を外す");
            remove.textContent = "×";
            actions.append(left, right, remove);
            item.append(actions);
        }
        return item;
    };

    if (!readonly) {
        // Delegated at the list: items the upload adds later
        // wire the same way.
        list.addEventListener("click", (event) => {
            const remove = event.target.closest("[data-remove-image]");
            if (remove) {
                remove.closest(".admin-image-item").remove();
                sync();
                document.dispatchEvent(new CustomEvent("product-images-changed"));
                return;
            }
            const move = event.target.closest("[data-move-image]");
            if (move) {
                const item = move.closest(".admin-image-item");
                const dir = parseInt(move.dataset.moveImage, 10);
                const sibling =
                    dir < 0 ? item.previousElementSibling : item.nextElementSibling;
                if (sibling && sibling.classList.contains("admin-image-item")) {
                    if (dir < 0) {
                        list.insertBefore(item, sibling);
                    } else {
                        list.insertBefore(sibling, item);
                    }
                    sync();
                }
            }
        });

        // Drag reorder: dropping on a thumbnail's left or
        // right half places the dragged item before or after it.
        let dragged = null;
        list.addEventListener("dragstart", (event) => {
            const item = event.target.closest(".admin-image-item");
            if (item) {
                dragged = item;
                item.classList.add("is-dragging");
                event.dataTransfer.effectAllowed = "move";
            }
        });
        list.addEventListener("dragover", (event) => {
            if (!dragged) return;
            event.preventDefault();
            event.dataTransfer.dropEffect = "move";
        });
        list.addEventListener("drop", (event) => {
            if (!dragged) return;
            event.preventDefault();
            const target = event.target.closest(".admin-image-item");
            if (target && target !== dragged) {
                const rect = target.getBoundingClientRect();
                if (event.clientX < rect.left + rect.width / 2) {
                    list.insertBefore(dragged, target);
                } else {
                    list.insertBefore(dragged, target.nextElementSibling);
                }
            }
            dragged.classList.remove("is-dragging");
            dragged = null;
            sync();
        });
        list.addEventListener("dragend", () => {
            if (dragged) {
                dragged.classList.remove("is-dragging");
                dragged = null;
            }
        });
    }

    const uploadOne = async (file) => {
        const tokenInput = form.querySelector('input[name="csrf_token"]');
        const body = new FormData();
        body.append("image", file);
        const response = await fetch("/admin/products/image-upload", {
            method: "POST",
            headers: { "x-csrf-token": tokenInput ? tokenInput.value : "" },
            body,
        });
        const data = await response.json().catch(() => null);
        if (!response.ok || !data || !data.url) {
            throw new Error(
                (data && (data.message || data.error)) || "アップロードに失敗しました。"
            );
        }
        return data.url;
    };

    const uploadFiles = async (files) => {
        if (!files || files.length === 0 || uploading) return;
        uploading = true;
        if (errorBox) errorBox.hidden = true;
        if (status) {
            status.textContent = "アップロード中…";
            status.hidden = false;
        }
        try {
            for (const file of Array.from(files)) {
                const url = await uploadOne(file);
                list.append(buildItem(url));
                // Announced only after the append: the
                // variant grid rebuilds its image choices
                // from the gallery when this fires, so an
                // event sent any earlier reaches it one
                // image behind.
                document.dispatchEvent(new CustomEvent("product-images-changed"));
                sync();
            }
            if (status) status.textContent = "アップロードしました";
        } catch (error) {
            showError(error.message || "アップロードに失敗しました。");
            if (status) status.hidden = true;
        } finally {
            uploading = false;
            if (fileInput) {
                // Reset so picking the same files again re-fires change.
                fileInput.value = "";
            }
        }
    };

    if (fileInput) {
        fileInput.addEventListener("change", () => {
            if (fileInput.files && fileInput.files.length > 0) {
                uploadFiles(fileInput.files);
            }
        });
    }

    if (dropzone) {
        ["dragenter", "dragover"].forEach((type) => {
            dropzone.addEventListener(type, (event) => {
                event.preventDefault();
                dropzone.classList.add("is-dragover");
            });
        });
        ["dragleave", "drop"].forEach((type) => {
            dropzone.addEventListener(type, (event) => {
                event.preventDefault();
                dropzone.classList.remove("is-dragover");
            });
        });
        dropzone.addEventListener("drop", (event) => {
            const files = event.dataTransfer && event.dataTransfer.files;
            if (files && files.length > 0) {
                uploadFiles(files);
            }
        });
    }

    sync();
})();

// Bulk-selection bar: visible only while at least one row is
// checked, with the live count; the header checkbox toggles
// the page's rows.
(() => {
    const bar = document.getElementById("admin-bulk-bar");
    if (!bar) return;
    const count = document.getElementById("admin-bulk-count");
    const boxes = () => Array.from(document.querySelectorAll(".admin-row-check"));
    const sync = () => {
        const n = boxes().filter((b) => b.checked).length;
        bar.hidden = n === 0;
        if (count) count.textContent = n + "件選択中";
    };
    boxes().forEach((b) => b.addEventListener("change", sync));
    const all = document.getElementById("admin-check-all");
    if (all) {
        all.addEventListener("change", () => {
            boxes().forEach((b) => { b.checked = all.checked; });
            sync();
        });
    }
    sync();
})();

// Linked 税抜 / 税込 price inputs (product form): typing in
// either field updates the other at the selected tax class's
// rate, using the server's own div-round rule (pricing.rs),
// so the preview never disagrees with what a save stores.
// The sale price's 税抜 companion and the variant grid's
// 税抜 cells carry no name, so they never submit; the base
// price pair is the exception — in 税抜 mode its named
// 税抜 field submits, and the server converts it on save.
let museTaxRounding = "floor";
function museDivRound(n, d) {
    if (d <= 0 || n <= 0) return 0;
    if (museTaxRounding === "ceil") return Math.floor((n + d - 1) / d);
    if (museTaxRounding === "round")
        return Math.floor((n + Math.floor(d / 2)) / d);
    return Math.floor(n / d);
}
function museTaxRate() {
    const sel = document.querySelector('select[name="tax_class_id"]');
    if (!sel) return 0;
    const opt = sel.options[sel.selectedIndex];
    const rate = opt ? parseInt(opt.getAttribute("data-rate") || "0", 10) : 0;
    return Number.isNaN(rate) ? 0 : rate;
}
function museExclFromIncl(value) {
    if (value === null || value === undefined || value === "") return "";
    const incl = parseInt(value, 10);
    if (Number.isNaN(incl)) return "";
    const rate = museTaxRate();
    return String(rate > 0 ? museDivRound(incl * 100, 100 + rate) : incl);
}
function museInclFromExcl(value) {
    if (value === null || value === undefined || value === "") return "";
    const excl = parseInt(value, 10);
    if (Number.isNaN(excl)) return "";
    const rate = museTaxRate();
    return String(rate > 0 ? museDivRound(excl * (100 + rate), 100) : excl);
}

(() => {
    const classSel = document.querySelector('select[name="tax_class_id"]');
    if (!classSel) return;
    museTaxRounding = classSel.getAttribute("data-rounding") || "floor";
    const priceIncl = document.querySelector('input[name="price_yen"]');
    const priceExcl = document.querySelector('input[name="price_excl_yen"]');
    const modeSel = document.querySelector('select[name="price_mode"]');
    const saleIncl = document.getElementById("sale_price_yen");
    const saleExcl = document.getElementById("sale_price_excl_yen");
    const exclMode = () => modeSel && modeSel.value === "excl";
    // First render: fill the 税抜 companions from the stored
    // 税込 values (the edit form leaves them blank).
    if (priceIncl && priceExcl && !exclMode() && priceExcl.value === "") {
        priceExcl.value = museExclFromIncl(priceIncl.value);
    }
    if (saleIncl && saleExcl) {
        saleExcl.value = museExclFromIncl(saleIncl.value);
        saleIncl.addEventListener("input", () => {
            saleExcl.value = museExclFromIncl(saleIncl.value);
        });
        saleExcl.addEventListener("input", () => {
            saleIncl.value = museInclFromExcl(saleExcl.value);
        });
    }
    if (priceIncl && priceExcl) {
        // The base pair follows the input-mode select: in 税抜
        // mode the 税抜 field drives (the server converts it on
        // save); in 税込 mode the 税抜 field is the derived
        // preview, and typing there stays inert, as before.
        priceExcl.addEventListener("input", () => {
            if (exclMode()) priceIncl.value = museInclFromExcl(priceExcl.value);
        });
        priceIncl.addEventListener("input", () => {
            if (!exclMode()) priceExcl.value = museExclFromIncl(priceIncl.value);
        });
        if (modeSel) {
            modeSel.addEventListener("change", () => {
                if (exclMode()) {
                    priceIncl.value = museInclFromExcl(priceExcl.value);
                } else {
                    priceExcl.value = museExclFromIncl(priceIncl.value);
                }
            });
        }
    }
    classSel.addEventListener("change", () => {
        if (priceIncl && priceExcl) {
            if (exclMode()) {
                priceIncl.value = museInclFromExcl(priceExcl.value);
            } else {
                priceExcl.value = museExclFromIncl(priceIncl.value);
            }
        }
        if (saleIncl && saleExcl) {
            saleExcl.value = museExclFromIncl(saleIncl.value);
        }
        document.dispatchEvent(new CustomEvent("muse:tax-rate-changed"));
    });
    // Variant grid cells: a 税抜 cell edit rewrites its 税込
    // cell (whose own input event then updates the grid
    // memory through the editor's listener); a 税込 edit
    // refreshes its 税抜 cell.
    document.addEventListener("input", (event) => {
        const target = event.target;
        if (!target.dataset || !target.dataset.comboKey) return;
        const field = target.dataset.comboField;
        if (field !== "price" && field !== "price_excl") return;
        const grid = document.getElementById("variant-grid");
        if (!grid || !grid.contains(target)) return;
        let inclInput = null;
        let exclInput = null;
        grid.querySelectorAll("input").forEach((input) => {
            if (input.dataset.comboKey !== target.dataset.comboKey) return;
            if (input.dataset.comboField === "price") inclInput = input;
            if (input.dataset.comboField === "price_excl") exclInput = input;
        });
        if (field === "price_excl" && inclInput) {
            inclInput.value = museInclFromExcl(target.value);
            inclInput.dispatchEvent(new Event("input", { bubbles: true }));
        } else if (field === "price" && exclInput) {
            exclInput.value = museExclFromIncl(target.value);
        }
    });
})();

// Variation editor (product form): repeatable axis blocks with
// their options, the template presets, and the combination
// stock grid. The grid is the cartesian product of the current
// axes/options in form order (first axis slowest) — the same
// order the server regenerates — and each row posts its stock
// as combo_stock[<i>]. The server validates and rebuilds the
// matrix authoritatively; this grid is an editing convenience.
(() => {
    const root = document.getElementById("variant-editor");
    if (!root) return;
    const axesBox = document.getElementById("variant-axes");
    const grid = document.getElementById("variant-grid");
    const readonly = root.dataset.readonly === "1";
    const MAX_AXES = 5;
    const MAX_OPTIONS = 5;
    // Static presets: one click inserts the header and
    // its usual options, all editable before saving.
    const PRESETS = {
        size: { name: "サイズ", options: ["S", "M", "L", "XL"] },
        color: { name: "カラー", options: ["ブラック", "ホワイト", "グレー", "ネイビー", "レッド"] },
        shoe: { name: "靴サイズ", options: ["23.0", "24.0", "25.0", "26.0", "27.0"] },
        volume: { name: "容量", options: ["300ml", "500ml", "1L"] },
    };
    const parseJson = (id) => {
        const el = document.getElementById(id);
        try {
            return JSON.parse(el ? el.textContent : "[]") || [];
        } catch (e) {
            return [];
        }
    };
    // Stored combinations by option-value tuple: the prefill
    // source that survives renames and reorders in the editor.
    // Each entry is the combination's full field set.
    const existingByKey = new Map();
    parseJson("variant-existing-combos").forEach((combo) => {
        existingByKey.set(combo.values.join("\u0001"), combo);
    });
    const postedCombos = parseJson("variant-grid-combos");
    // Field sets typed into this grid, kept by combination key
    // so a re-render (an option renamed elsewhere) loses
    // nothing.
    const gridMemory = new Map();

    const blankCombo = () => ({
        stock: "0",
        price: "",
        sku: "",
        jan: "",
        image: "",
        enabled: true,
        incoming: "",
        incomingDate: "",
    });
    const comboOf = (raw) => {
        const base = blankCombo();
        if (!raw) return base;
        return {
            stock: raw.stock == null ? "0" : String(raw.stock),
            price: raw.price == null ? "" : String(raw.price),
            sku: raw.sku || "",
            jan: raw.jan || "",
            image: raw.image || "",
            enabled: raw.enabled !== false,
            incoming:
                raw.incoming == null || raw.incoming === 0
                    ? ""
                    : String(raw.incoming),
            incomingDate: raw.incomingDate || "",
        };
    };
    // The image choices a combination may point at: the form's
    // current gallery, read live so uploads join the list.
    const imagePaths = () =>
        Array.from(
            document.querySelectorAll(
                '#product-image-list input[name="images"]'
            )
        ).map((input) => input.value);

    const axisBlocks = () => Array.from(axesBox.querySelectorAll("[data-axis-block]"));
    const optionSpans = (block) => Array.from(block.querySelectorAll(".variant-option"));

    // Rewrites every field name from the blocks' positions, so
    // removals never leave index gaps for the server to collapse.
    function reindex() {
        axisBlocks().forEach((block, i) => {
            block.querySelector(".variant-axis-head input[type=hidden]").name =
                "axes[" + i + "][id]";
            block.querySelector(".variant-axis-head input[type=text]").name =
                "axes[" + i + "][name]";
            optionSpans(block).forEach((span, j) => {
                span.querySelector('input[type=hidden]').name =
                    "axes[" + i + "][options][" + j + "][id]";
                span.querySelector('input[type=text]').name =
                    "axes[" + i + "][options][" + j + "][value]";
            });
        });
    }

    function collectAxes() {
        return axisBlocks().map((block) => ({
            name: block.querySelector(".variant-axis-head input[type=text]").value,
            options: optionSpans(block).map(
                (span) => span.querySelector('input[type=text]').value
            ),
        }));
    }

    function cartesian(lists) {
        let combos = [[]];
        for (const list of lists) {
            const next = [];
            for (const combo of combos) {
                for (const item of list) {
                    next.push(combo.concat([item]));
                }
            }
            combos = next;
        }
        return combos;
    }

    // Builds one combination field as a labelled block: the
    // caption sits over its control, so every input states
    // what it is and — through the card it sits in — which
    // combination it belongs to.
    const FIELD_CAPTIONS = {
        stock: "在庫",
        price: "価格（任意）",
        price_excl: "税抜価格（換算）",
        sku: "SKU",
        jan: "JAN",
        image: "画像",
        enabled: "販売",
        incoming: "入荷予定数",
        incomingDate: "入荷予定日",
    };
    function gridField(k, key, field, value, attrs) {
        const wrap = document.createElement("label");
        wrap.className = "variant-field";
        const caption = document.createElement("span");
        caption.className = "variant-field-name";
        caption.textContent = FIELD_CAPTIONS[field] || field;
        wrap.append(caption);
        const input = document.createElement(attrs.select ? "select" : "input");
        if (!attrs.select) {
            input.type = attrs.type || "text";
            if (attrs.numeric) input.inputMode = "numeric";
            if (attrs.placeholder) input.placeholder = attrs.placeholder;
        }
        if (attrs.name) input.name = attrs.name + "[" + k + "]";
        input.dataset.comboKey = key;
        input.dataset.comboField = field;
        if (attrs.select) {
            const none = document.createElement("option");
            none.value = "";
            none.textContent = "なし";
            input.append(none);
            imagePaths().forEach((path) => {
                const option = document.createElement("option");
                option.value = path;
                option.textContent = path.split("/").pop();
                input.append(option);
            });
        }
        if (attrs.checkbox) {
            input.type = "checkbox";
            input.value = "1";
            input.checked = !!value;
        } else {
            input.value = value;
        }
        if (readonly) input.disabled = true;
        if (attrs.checkbox) {
            // An unchecked checkbox posts nothing, which the
            // server cannot tell from a form that predates the
            // toggle. A hidden 0 under the same name makes the
            // state explicit either way (the 1 follows when on).
            const off = document.createElement("input");
            off.type = "hidden";
            off.name = input.name;
            off.value = "0";
            if (readonly) off.disabled = true;
            wrap.append(off, input);
            return wrap;
        }
        wrap.append(input);
        return wrap;
    }

    // Renders the combinations as colour blocks: one tinted
    // block per first-axis value (サイズ S, then M), holding
    // one card per combination whose header names the
    // remaining axes (カラー: ブラック), so every input sits
    // inside the block and the card of the variation it
    // belongs to. Block tints cycle a small palette.
    function renderGrid() {
        const axes = collectAxes();
        const combos = cartesian(axes.map((a) => a.options));
        grid.innerHTML = "";
        const groups = new Map();
        combos.forEach((values, k) => {
            const key = values.join("\u0001");
            const data = gridMemory.has(key)
                ? gridMemory.get(key)
                : existingByKey.has(key)
                  ? comboOf(existingByKey.get(key))
                  : postedCombos.length === combos.length
                    ? comboOf(postedCombos[k])
                    : blankCombo();
            gridMemory.set(key, data);
            const groupValue = values.length ? values[0] : "";
            if (!groups.has(groupValue)) {
                groups.set(groupValue, []);
            }
            groups.get(groupValue).push({ values, k, key, data });
        });
        let groupIndex = 0;
        groups.forEach((items, groupValue) => {
            const group = document.createElement("section");
            group.className = "variant-group variant-group-" + (groupIndex % 5);
            groupIndex += 1;
            if (axes.length && groupValue !== "") {
                const head = document.createElement("h4");
                head.className = "variant-group-head";
                const axisName = document.createElement("span");
                axisName.className = "variant-group-axis";
                axisName.textContent = axes[0].name || "バリエーション";
                const value = document.createElement("span");
                value.className = "variant-group-value";
                value.textContent = groupValue;
                const count = document.createElement("span");
                count.className = "variant-group-count";
                count.textContent = items.length + "通り";
                head.append(axisName, value, count);
                group.append(head);
            }
            items.forEach(({ values, k, key, data }) => {
                const card = document.createElement("div");
                card.className = "variant-card";
                const rest = values.slice(1);
                if (rest.length) {
                    const cardHead = document.createElement("div");
                    cardHead.className = "variant-card-head";
                    cardHead.textContent = rest
                        .map((v, i) => {
                            const name = axes[i + 1] && axes[i + 1].name;
                            return name ? name + "：" + v : v;
                        })
                        .join(" ・ ");
                    card.append(cardHead);
                } else if (!values.length) {
                    const cardHead = document.createElement("div");
                    cardHead.className = "variant-card-head";
                    cardHead.textContent = "バリエーションなし";
                    card.append(cardHead);
                }
                const fields = document.createElement("div");
                fields.className = "variant-card-fields";
                fields.append(
                    gridField(k, key, "stock", data.stock, {
                        name: "combo_stock",
                        numeric: true,
                    })
                );
                fields.append(
                    gridField(k, key, "price", data.price, {
                        name: "combo_price",
                        numeric: true,
                    })
                );
                fields.append(
                    gridField(
                        k,
                        key,
                        "price_excl",
                        museExclFromIncl(
                            data.price === null || data.price === undefined
                                ? ""
                                : String(data.price)
                        ),
                        {
                            name: "",
                            numeric: true,
                        }
                    )
                );
                fields.append(
                    gridField(k, key, "sku", data.sku, {
                        name: "combo_sku",
                    })
                );
                fields.append(
                    gridField(k, key, "jan", data.jan, {
                        name: "combo_jan",
                        numeric: true,
                    })
                );
                fields.append(
                    gridField(k, key, "image", data.image, {
                        name: "combo_image",
                        select: true,
                    })
                );
                fields.append(
                    gridField(k, key, "enabled", data.enabled, {
                        name: "combo_enabled",
                        checkbox: true,
                    })
                );
                fields.append(
                    gridField(k, key, "incoming", data.incoming, {
                        name: "combo_incoming",
                        numeric: true,
                    })
                );
                fields.append(
                    gridField(k, key, "incomingDate", data.incomingDate, {
                        name: "combo_incoming_date",
                        type: "date",
                    })
                );
                card.append(fields);
                group.append(card);
            });
            grid.append(group);
        });
    }

    function makeOption(value) {
        const span = document.createElement("span");
        span.className = "variant-option";
        const id = document.createElement("input");
        id.type = "hidden";
        id.value = "";
        const text = document.createElement("input");
        text.type = "text";
        text.placeholder = "選択肢";
        text.value = value || "";
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "text-button";
        remove.dataset.removeOption = "";
        remove.textContent = "×";
        span.append(id, text, remove);
        return span;
    }

    function addAxis(name, optionValues) {
        if (axisBlocks().length >= MAX_AXES) return;
        const block = document.createElement("div");
        block.className = "variant-axis";
        block.dataset.axisBlock = "";
        const head = document.createElement("div");
        head.className = "variant-axis-head";
        const id = document.createElement("input");
        id.type = "hidden";
        id.value = "";
        const label = document.createElement("label");
        label.textContent = "見出し ";
        const nameInput = document.createElement("input");
        nameInput.type = "text";
        nameInput.placeholder = "例：サイズ";
        nameInput.value = name || "";
        label.append(nameInput);
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "text-button";
        remove.dataset.removeAxis = "";
        remove.textContent = "この見出しを削除";
        head.append(id, label, remove);
        const options = document.createElement("div");
        options.className = "variant-options";
        (optionValues && optionValues.length ? optionValues : [""]).forEach((value) => {
            options.append(makeOption(value));
        });
        const add = document.createElement("button");
        add.type = "button";
        add.className = "secondary-button";
        add.dataset.addOption = "";
        add.textContent = "選択肢を追加";
        block.append(head, options, add);
        axesBox.append(block);
        reindex();
        renderGrid();
    }

    root.addEventListener("click", (event) => {
        const target = event.target;
        if (!(target instanceof HTMLElement)) return;
        if (target.id === "variant-add-axis") {
            addAxis("", [""]);
        } else if (target.hasAttribute("data-remove-axis")) {
            target.closest("[data-axis-block]").remove();
            reindex();
            renderGrid();
        } else if (target.hasAttribute("data-add-option")) {
            const block = target.closest("[data-axis-block]");
            if (optionSpans(block).length >= MAX_OPTIONS) return;
            block.querySelector(".variant-options").append(makeOption(""));
            reindex();
            renderGrid();
        } else if (target.hasAttribute("data-remove-option")) {
            const block = target.closest("[data-axis-block]");
            if (optionSpans(block).length <= 1) return;
            target.closest(".variant-option").remove();
            reindex();
            renderGrid();
        }
    });

    const preset = document.getElementById("variant-preset");
    if (preset) {
        preset.addEventListener("change", () => {
            const chosen = PRESETS[preset.value];
            if (chosen) {
                addAxis(chosen.name, chosen.options.slice());
            }
            preset.value = "";
        });
    }

    root.addEventListener("input", (event) => {
        if (event.target.closest("#variant-grid")) {
            const target = event.target;
            const entry = gridMemory.get(target.dataset.comboKey) || blankCombo();
            entry[target.dataset.comboField] =
                target.type === "checkbox" ? target.checked : target.value;
            gridMemory.set(target.dataset.comboKey, entry);
        } else if (event.target.closest("#variant-axes")) {
            renderGrid();
        }
    });
    root.addEventListener("change", (event) => {
        if (event.target.closest("#variant-grid")) {
            const target = event.target;
            const entry = gridMemory.get(target.dataset.comboKey) || blankCombo();
            entry[target.dataset.comboField] =
                target.type === "checkbox" ? target.checked : target.value;
            gridMemory.set(target.dataset.comboKey, entry);
        }
    });
    document.addEventListener("product-images-changed", () => {
        renderGrid();
    });
    document.addEventListener("muse:tax-rate-changed", () => {
        renderGrid();
    });

    reindex();
    renderGrid();
})();

// Tag picker on the product form: the panel's checkboxes are
// the submitted data (the tag_ids fields, unchanged); the chip
// row above them is a mirror. Adding goes through the panel —
// chips are only ever created from the master tag list, never
// from typed text.
(() => {
    const panel = document.getElementById("tag-panel");
    const chipsRow = document.getElementById("tag-chips");
    const addButton = document.getElementById("tag-add-button");
    if (!panel || !chipsRow || !addButton) {
        return;
    }
    const boxFor = (id) => panel.querySelector(`input[name="tag_ids"][value="${id}"]`);
    const labelOf = (box) => box.closest("label").textContent.trim();
    const makeChip = (box) => {
        const chip = document.createElement("span");
        chip.className = "admin-badge tag-chip-selected";
        chip.append(document.createTextNode(labelOf(box)));
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "tag-chip-remove";
        remove.dataset.removeTag = box.value;
        remove.setAttribute("aria-label", `${labelOf(box)}を外す`);
        remove.textContent = "×";
        if (box.disabled) {
            remove.disabled = true;
        }
        chip.append(remove);
        return chip;
    };
    addButton.addEventListener("click", () => {
        panel.hidden = !panel.hidden;
    });
    const tagSearch = panel.querySelector("[data-tag-search]");
    const tagEmpty = panel.querySelector("[data-tag-empty]");
    if (tagSearch) {
        const applyTagFilter = () => {
            const q = tagSearch.value.trim().toLowerCase();
            let anyVisible = false;
            panel.querySelectorAll("label").forEach((row) => {
                const show = !q || row.textContent.trim().toLowerCase().includes(q);
                row.hidden = !show;
                if (show) {
                    anyVisible = true;
                }
            });
            if (tagEmpty) {
                tagEmpty.hidden = anyVisible;
            }
        };
        tagSearch.addEventListener("input", applyTagFilter);
        tagSearch.addEventListener("search", applyTagFilter);
    }
    panel.addEventListener("change", (event) => {
        const box = event.target;
        if (box.name !== "tag_ids") {
            return;
        }
        const existing = chipsRow.querySelector(`[data-remove-tag="${box.value}"]`);
        if (box.checked && !existing) {
            chipsRow.append(makeChip(box));
        } else if (!box.checked && existing) {
            existing.closest(".tag-chip-selected").remove();
        }
    });
    chipsRow.addEventListener("click", (event) => {
        const remove = event.target.closest("[data-remove-tag]");
        if (!remove) {
            return;
        }
        const box = boxFor(remove.dataset.removeTag);
        if (box) {
            box.checked = false;
        }
        remove.closest(".tag-chip-selected").remove();
    });
})();


// Content page preview (CMS spec C-2): the editor's preview
// button shows the UNSAVED title/body in the storefront layout
// in a new tab. It cannot be a second submit button in the
// edit form -- the multiple-click guard would mark the edit
// form as submitting and block the real save -- so the click
// builds a throwaway form carrying the current values (and the
// edit form's hidden fields, the CSRF token among them) and
// submits that instead. The edit form itself is never
// submitted, and no preview URL or token exists to leak.
document.addEventListener("click", (event) => {
    const button = event.target.closest("[data-preview]");
    if (!button) return;
    const form = button.closest("form");
    if (!form) return;
    const title = form.querySelector('input[name="title"]');
    const body = form.querySelector('textarea[name="body_md"]');
    const temp = document.createElement("form");
    temp.method = "post";
    temp.action = button.dataset.preview;
    temp.target = "_blank";
    const add = (name, value) => {
        const input = document.createElement("input");
        input.type = "hidden";
        input.name = name;
        input.value = value;
        temp.append(input);
    };
    form.querySelectorAll('input[type="hidden"]').forEach((input) => {
        add(input.name, input.value);
    });
    add("title", title ? title.value : "");
    add("body_md", body ? body.value : "");
    const format = form.querySelector('input[name="format"]:checked');
    add("format", format ? format.value : "markdown");
    document.body.append(temp);
    temp.submit();
    temp.remove();
});

// Content page format toggle (CMS v2): selecting HTML switches
// the body textarea to a monospace face — code reads as code.
// Presentation only; the server re-reads the format from the
// submitted radio on every save and preview.
document.querySelectorAll('input[name="format"]').forEach((radio) => {
    radio.addEventListener("change", () => {
        const form = radio.closest("form");
        const body = form ? form.querySelector("[data-body-source]") : null;
        if (body) {
            body.classList.toggle(
                "body-source-html",
                radio.checked && radio.value === "html"
            );
        }
    });
});

// Meta description counter (CMS spec C-2): the editor's meta
// field shows its live character count against the limit the
// server enforces. The count is a convenience only; the
// server's validation stays authoritative.
document.querySelectorAll("[data-meta-counter]").forEach((field) => {
    const label = field.closest("label");
    const counter = label ? label.querySelector("[data-meta-count]") : null;
    if (!counter) return;
    const update = () => {
        counter.textContent = "（現在 " + String(field.value).length + " / 160文字）";
    };
    field.addEventListener("input", update);
    update();
});


// Unsaved-changes warning, shared by every editor form that
// opts in with data-unsaved-guard (product, top page, menus,
// shop settings): leaving with unsaved edits asks first, and
// the note beside the save button lights up so the state is
// visible before any leave attempt. Submitting clears the
// flag (the global multiple-click guard owns the in-flight
// state), and a read-only (archived) form never warns —
// nothing on it can change.
(() => {
    document.querySelectorAll("form[data-unsaved-guard]").forEach((form) => {
        if (form.querySelector("fieldset[data-readonly]")) return;
        const note = form.querySelector("[data-unsaved-note]");
        let dirty = false;
        let submitting = false;
        const mark = () => {
            dirty = true;
            if (note) note.hidden = false;
        };
        const markUnlessSearch = (event) => {
            if (event.target.closest("[data-combobox-input], [data-tag-search]")) {
                return;
            }
            mark();
        };
        form.addEventListener("input", markUnlessSearch);
        form.addEventListener("change", mark);
        form.addEventListener("submit", () => {
            submitting = true;
        });
        window.addEventListener("beforeunload", (event) => {
            if (dirty && !submitting) {
                event.preventDefault();
            }
        });
    });
})();

// Snapshots for the change-detecting confirmations above:
// what the money-bearing fields of the product edit form
// and the access-bearing fields of the staff edit form
// held when the page rendered. Taken here — after the
// variant editor has built its grid — so the combo fields
// exist. The create forms are skipped: the product edit
// form is the one carrying the staleness `expected`
// field, and the staff edit form is the one posting to a
// numbered admin URL; a create has nothing to change
// against.
const moneySnapshots = new WeakMap();
const roleSnapshots = new WeakMap();

// The product form's money signature: the price fields,
// the sale window, and every combo's price and stock
// values, in DOM order.
function moneySignature(form) {
    const parts = [];
    form.querySelectorAll(
        "[name=price_yen], [name=price_excl_yen], [name=sale_price_yen], "
        + "[name=sale_starts_at], [name=sale_ends_at], "
        + "[name^=combo_price], [name^=combo_stock], [name^=combo_incoming]",
    ).forEach((el) => parts.push(el.value));
    return parts.join("|");
}

// The staff form's access signature: the checked role
// and active-state radio values, joined as "role|active".
function roleSignature(form) {
    const checked = (name) => {
        const el = form.querySelector(`[name=${name}]:checked`);
        return el ? el.value : "";
    };
    return `${checked("role")}|${checked("is_active")}`;
}

document.querySelectorAll("#admin-product-form").forEach((form) => {
    if (form.querySelector("[name=expected]")) {
        moneySnapshots.set(form, moneySignature(form));
    }
});
document.querySelectorAll("form[data-role-form]").forEach((form) => {
    if (/\/admin\/admins\/\d+$/.test(form.action)) {
        roleSnapshots.set(form, roleSignature(form));
    }
});

// Top page hero (CMS C-3): the image list uploads through
// the product image endpoint — the one upload pipeline, so
// the hero obeys the same type/size rules as product
// images — and the preview band mirrors the hero fields as
// they are typed, at desktop or phone width. The list is
// capped at ten (the server refuses more); the hidden
// field carries the paths comma-joined, the pins' contract.
(() => {
    const field = document.querySelector("[data-hero-images-field]");
    if (!field) {
        return;
    }
    const form = field.closest("form");
    const box = document.querySelector("[data-hero-images]");
    const rowsBox = box.querySelector("[data-hero-image-rows]");
    const fileInput = box.querySelector("[data-hero-image-file]");
    const addLabel = fileInput ? fileInput.closest("label") : null;
    const status = box.querySelector("[data-hero-image-status]");
    const errorBox = box.querySelector("[data-hero-image-error]");
    const previewImg = document.querySelector("[data-hero-preview-img]");
    const previewHeadline = document.querySelector("[data-hero-preview-headline]");
    const previewText = document.querySelector("[data-hero-preview-text]");
    const previewStage = document.querySelector("[data-hero-preview-stage]");
    const previewPlain = document.querySelector("[data-hero-preview-plain]");
    const previewHeadlinePlain = document.querySelector(
        "[data-hero-preview-headline-plain]"
    );
    const previewTextPlain = document.querySelector(
        "[data-hero-preview-text-plain]"
    );
    const linksField = document.querySelector("[data-hero-image-links-field]");
    const previewFrame = document.querySelector("[data-hero-preview-frame]");
    const MAX_IMAGES = 10;

    const rows = () =>
        Array.from(rowsBox.querySelectorAll("[data-hero-image-row]"));

    const sync = (announce) => {
        const paths = rows().map((row) => row.dataset.heroImagePath);
        field.value = paths.join(",");
        if (linksField) {
            linksField.value = rows()
                .map((row) => {
                    const input = row.querySelector("[data-hero-image-link]");
                    return input ? input.value.trim() : "";
                })
                .join(",");
        }
        if (announce) {
            // A programmatic set fires no event; announce it
            // so change listeners (the unsaved-changes guard)
            // see it.
            field.dispatchEvent(new Event("input", { bubbles: true }));
            if (linksField) {
                linksField.dispatchEvent(new Event("input", { bubbles: true }));
            }
        }
        if (addLabel) {
            addLabel.hidden = paths.length >= MAX_IMAGES;
        }
        if (previewImg) {
            if (paths.length) {
                previewImg.src = paths[0];
                previewImg.hidden = false;
            } else {
                previewImg.hidden = true;
                previewImg.removeAttribute("src");
            }
        }
        if (previewStage) {
            previewStage.hidden = paths.length === 0;
        }
        if (previewPlain) {
            previewPlain.hidden = paths.length !== 0;
        }
    };

    const buildRow = (path, link) => {
        const row = document.createElement("li");
        row.className = "pin-row";
        row.dataset.heroImageRow = "";
        row.dataset.heroImagePath = path;
        const wrap = document.createElement("div");
        const main = document.createElement("div");
        main.className = "hero-image-main";
        const img = document.createElement("img");
        img.className = "hero-image-thumb";
        img.src = path;
        img.alt = "";
        const name = document.createElement("span");
        name.className = "pin-name";
        name.textContent = path;
        const actions = document.createElement("span");
        actions.className = "pin-actions";
        const up = document.createElement("button");
        up.type = "button";
        up.className = "secondary-button";
        up.dataset.heroImageUp = "";
        up.setAttribute("aria-label", "上へ");
        up.textContent = "↑";
        const down = document.createElement("button");
        down.type = "button";
        down.className = "secondary-button";
        down.dataset.heroImageDown = "";
        down.setAttribute("aria-label", "下へ");
        down.textContent = "↓";
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "secondary-button";
        remove.dataset.heroImageRemove = "";
        remove.textContent = "外す";
        actions.append(up, down, remove);
        main.append(img, name, actions);
        const linkRow = document.createElement("label");
        linkRow.className = "hero-image-link-row";
        linkRow.textContent = "リンク先 ";
        const linkInput = document.createElement("input");
        linkInput.type = "text";
        linkInput.dataset.heroImageLink = "";
        linkInput.value = link || "";
        linkInput.placeholder = "/special（空欄ならリンクなし）";
        linkRow.append(linkInput);
        wrap.append(main, linkRow);
        row.append(wrap);
        return row;
    };

    rowsBox.addEventListener("input", (event) => {
        if (event.target.closest("[data-hero-image-link]")) {
            sync(true);
        }
    });

    box.addEventListener("click", (event) => {
        const row = event.target.closest("[data-hero-image-row]");
        if (!row) {
            return;
        }
        if (event.target.closest("[data-hero-image-remove]")) {
            row.remove();
            sync(true);
        } else if (event.target.closest("[data-hero-image-up]")) {
            const prev = row.previousElementSibling;
            if (prev) {
                rowsBox.insertBefore(row, prev);
                sync(true);
            }
        } else if (event.target.closest("[data-hero-image-down]")) {
            const next = row.nextElementSibling;
            if (next) {
                rowsBox.insertBefore(next, row);
                sync(true);
            }
        }
    });

    if (fileInput) {
        fileInput.addEventListener("change", async () => {
            const file = fileInput.files && fileInput.files[0];
            if (!file) {
                return;
            }
            if (errorBox) {
                errorBox.hidden = true;
            }
            if (status) {
                status.hidden = false;
            }
            try {
                const tokenInput = form.querySelector('input[name="csrf_token"]');
                const body = new FormData();
                body.append("image", file);
                const response = await fetch("/admin/products/image-upload", {
                    method: "POST",
                    headers: { "x-csrf-token": tokenInput ? tokenInput.value : "" },
                    body,
                });
                const data = await response.json().catch(() => null);
                if (!response.ok || !data || !data.url) {
                    throw new Error(
                        (data && (data.message || data.error)) ||
                            "アップロードに失敗しました。"
                    );
                }
                if (rows().some((row) => row.dataset.heroImagePath === data.url)) {
                    throw new Error("その画像は既に追加されています。");
                }
                rowsBox.append(buildRow(data.url));
                sync(true);
            } catch (error) {
                if (errorBox) {
                    errorBox.textContent =
                        error.message || "アップロードに失敗しました。";
                    errorBox.hidden = false;
                }
            } finally {
                if (status) {
                    status.hidden = true;
                }
                fileInput.value = "";
            }
        });
    }

    const bindPreview = (name, target) => {
        const input = form.querySelector('input[name="' + name + '"]');
        if (!input || !target) {
            return;
        }
        input.addEventListener("input", () => {
            target.textContent = input.value;
        });
    };
    bindPreview("hero_headline", previewHeadline);
    bindPreview("hero_text", previewText);
    bindPreview("hero_headline", previewHeadlinePlain);
    bindPreview("hero_text", previewTextPlain);

    document.querySelectorAll("[data-hero-preview-width]").forEach((button) => {
        button.addEventListener("click", () => {
            document
                .querySelectorAll("[data-hero-preview-width]")
                .forEach((other) => other.classList.remove("is-active"));
            button.classList.add("is-active");
            if (previewFrame) {
                previewFrame.classList.toggle(
                    "is-sp",
                    button.dataset.heroPreviewWidth === "sp"
                );
            }
        });
    });

    sync(false);
})();

// Sidebar collapse: the toggle hides the whole sidebar for a
// wider work area and the floating corner button brings it
// back; the choice persists per browser. Without a stored
// choice the sidebar starts open, as it always has.
(() => {
    const sidebar = document.querySelector("[data-admin-sidebar]");
    if (!sidebar) {
        return;
    }
    const buttons = document.querySelectorAll("[data-nav-toggle]");
    const KEY = "admin-sidebar-collapsed";
    const apply = (collapsed) => {
        sidebar.classList.toggle("is-collapsed", collapsed);
        buttons.forEach((button) => {
            if (button.classList.contains("admin-nav-open")) {
                button.hidden = !collapsed;
            }
            if (button.classList.contains("admin-nav-toggle")) {
                button.setAttribute("aria-expanded", collapsed ? "false" : "true");
                button.textContent = collapsed ? "メニューを開く" : "メニューを閉じる";
            }
        });
    };
    let collapsed = false;
    try {
        collapsed = window.localStorage.getItem(KEY) === "1";
    } catch (error) {
        collapsed = false;
    }
    apply(collapsed);
    buttons.forEach((button) => {
        button.addEventListener("click", () => {
            collapsed = !collapsed;
            try {
                window.localStorage.setItem(KEY, collapsed ? "1" : "0");
            } catch (error) {
                // Persistence is a nicety; the toggle still works.
            }
            apply(collapsed);
        });
    });
})();




// Account form role toggle: the permission checkboxes apply
// to staff only, so the block hides while the owner role is
// chosen. Presentation only - the server stores no
// permissions for an owner either way.
(() => {
    document.querySelectorAll("[data-role-form]").forEach((form) => {
        const block = form.querySelector("[data-role-permissions]");
        if (!block) {
            return;
        }
        const sync = () => {
            const chosen = form.querySelector("input[name='role']:checked");
            block.hidden = !chosen || chosen.value !== "staff";
        };
        form.querySelectorAll("input[name='role']").forEach((radio) => {
            radio.addEventListener("change", sync);
        });
        sync();
    });
})();

// Sidebar menu search: typing in the box above the nav narrows
// the menu to the entries that match, by the Japanese label or
// by the English value (the link's path under /admin/, such as
// "products" or "settings/shop"), so the menu is searchable in
// both languages. A group heading stays while any of its
// entries match, and a heading that itself matches keeps its
// whole group. The filter only hides rows in the rendered
// page; the served markup and every link stay exactly as they
// are. Escape clears the box.
(() => {
    const input = document.querySelector("[data-nav-search]");
    const nav = document.querySelector(".admin-nav");
    if (!input || !nav) {
        return;
    }
    const empty = nav.querySelector("[data-nav-empty]");
    const entries = Array.from(nav.children);
    // The English value of a link: its path under /admin/.
    const linkValue = (el) =>
        (el.getAttribute("href") || "").toLowerCase().replace(/^\/admin\/?/, "");
    const apply = () => {
        const query = input.value.trim().toLowerCase();
        if (!query) {
            entries.forEach((el) => el.classList.remove("is-search-hidden"));
            if (empty) {
                empty.hidden = true;
            }
            return;
        }
        let anyVisible = false;
        let groupMatches = false;
        entries.forEach((el) => {
            if (el.classList.contains("admin-nav-heading")) {
                groupMatches = el.textContent.toLowerCase().includes(query);
                return;
            }
            if (el.tagName !== "A") {
                return;
            }
            const show =
                groupMatches ||
                el.textContent.toLowerCase().includes(query) ||
                linkValue(el).includes(query);
            el.classList.toggle("is-search-hidden", !show);
            if (show) {
                anyVisible = true;
            }
        });
        // A heading stays only while a following entry, up to
        // the next heading, is visible.
        let heading = null;
        let headingHasVisible = false;
        const settleHeading = () => {
            if (heading) {
                heading.classList.toggle("is-search-hidden", !headingHasVisible);
            }
        };
        entries.forEach((el) => {
            if (el.classList.contains("admin-nav-heading")) {
                settleHeading();
                heading = el;
                headingHasVisible = false;
                return;
            }
            if (el.tagName === "A" && !el.classList.contains("is-search-hidden")) {
                headingHasVisible = true;
            }
        });
        settleHeading();
        if (empty) {
            empty.hidden = anyVisible;
        }
    };
    input.addEventListener("input", apply);
    input.addEventListener("keydown", (event) => {
        if (event.key === "Escape") {
            input.value = "";
            apply();
        }
    });
})();

// Menu editor add-link: the server renders one blank row per
// menu (the no-script fallback); this button clones the blank
// row template with the next free field index, under the
// exact field-name contract the save already parses. A cloned
// row can be removed again before saving; a row left empty is
// ignored by the save either way.
(() => {
    document.querySelectorAll("[data-menu-add]").forEach((button) => {
        const menu = button.getAttribute("data-menu-add");
        const form = button.closest("form");
        if (!form) return;
        const container = form.querySelector(`[data-menu-blank-rows="${menu}"]`);
        const template = form.querySelector(`[data-menu-blank-template="${menu}"]`);
        if (!container || !template) return;
        button.addEventListener("click", () => {
            let maxIndex = -1;
            let rowCount = 0;
            form.querySelectorAll(`input[name^="items[${menu}]["]`).forEach((input) => {
                const match = input.name.match(/\[(\d+)\]/);
                if (match) maxIndex = Math.max(maxIndex, Number(match[1]));
                if (input.name.endsWith("[label]")) rowCount += 1;
            });
            const holder = document.createElement("div");
            holder.innerHTML = template.innerHTML
                .replaceAll("__INDEX__", String(maxIndex + 1))
                .replaceAll("__SORT__", String((rowCount + 1) * 10));
            const row = holder.firstElementChild;
            if (!row) return;
            container.appendChild(row);
            const first = row.querySelector("input[type=\"text\"]");
            if (first) first.focus();
            // Wake the unsaved-changes guard: a structural
            // change carries no input event of its own.
            form.dispatchEvent(new Event("input", { bubbles: true }));
        });
    });
    document.querySelectorAll("[data-menu-blank-rows]").forEach((container) => {
        container.addEventListener("click", (event) => {
            const remove = event.target.closest("[data-menu-row-remove]");
            if (!remove) return;
            const row = remove.closest("[data-menu-blank-row]");
            if (row) row.remove();
        });
    });
})();


// Searchable combobox: a text input paired with the form's
// own select, for pickers whose option list can grow long
// (the coupon form's product and variant targets first).
// The operator can type to filter, or open the list and
// pick; the select keeps the posted name and value, so the
// server contract is unchanged, and it stays visible until
// this script enhances the wrapper. Typing clears the
// posted value until an option is picked, so a half-typed
// search can never submit a stale id. A select carrying
// data-parent-select filters its options to the parent
// select's current value (variants under their product).
(() => {
    const closeAll = (except) => {
        document.querySelectorAll("[data-combobox].is-open").forEach((wrap) => {
            if (wrap !== except && wrap._comboboxClose) {
                wrap._comboboxClose();
            }
        });
    };
    document.addEventListener("click", (event) => {
        document.querySelectorAll("[data-combobox].is-open").forEach((wrap) => {
            if (!wrap.contains(event.target) && wrap._comboboxClose) {
                wrap._comboboxClose();
            }
        });
    });
    document.querySelectorAll("[data-combobox]").forEach((wrap) => {
        const input = wrap.querySelector("[data-combobox-input]");
        const select = wrap.querySelector("[data-combobox-select]");
        const toggle = wrap.querySelector("[data-combobox-toggle]");
        if (!input || !select) {
            return;
        }
        const form = wrap.closest("form");
        const parentSelect = select.dataset.parentSelect && form
            ? form.querySelector(`select[name="${select.dataset.parentSelect}"]`)
            : null;
        wrap.classList.add("is-enhanced");
        const list = document.createElement("ul");
        list.className = "admin-combobox-list";
        list.hidden = true;
        list.setAttribute("role", "listbox");
        wrap.appendChild(list);
        input.setAttribute("role", "combobox");
        input.setAttribute("aria-haspopup", "listbox");
        input.setAttribute("aria-expanded", "false");
        let query = "";
        let activeBtn = null;
        let committedValue = select.value;
        const options = () => Array.from(select.options).map((opt) => ({
            value: opt.value,
            label: opt.textContent.trim(),
            parent: opt.dataset.parentId || "",
            disabled: opt.disabled,
            searchText: (opt.textContent.trim() + " " + opt.value).toLowerCase(),
        }));
        const selectedOption = () => options().find((o) => o.value !== "" && o.value === select.value) || null;
        const syncInput = () => {
            const chosen = selectedOption();
            input.value = chosen ? chosen.label : "";
        };
        const syncEnabled = () => {
            input.disabled = select.disabled;
            if (toggle) {
                toggle.disabled = select.disabled;
            }
            if (select.disabled && !list.hidden) {
                close();
            }
        };
        new MutationObserver(syncEnabled).observe(select, {
            attributes: true,
            attributeFilter: ["disabled"],
        });
        const render = () => {
            const q = query.trim().toLowerCase();
            const parentValue = parentSelect ? parentSelect.value : "";
            const items = options().filter((o) => {
                if (parentValue && o.value !== "" && o.parent !== parentValue) {
                    return false;
                }
                return !q || o.searchText.includes(q);
            });
            list.innerHTML = "";
            activeBtn = null;
            if (!items.length) {
                const li = document.createElement("li");
                li.className = "admin-combobox-empty";
                li.textContent = "該当する項目がありません";
                list.appendChild(li);
                return;
            }
            items.forEach((o) => {
                const li = document.createElement("li");
                const btn = document.createElement("button");
                btn.type = "button";
                btn.className = "admin-combobox-option";
                btn.textContent = o.label;
                btn.dataset.value = o.value;
                btn.setAttribute("role", "option");
                btn.setAttribute("aria-selected", o.value !== "" && o.value === select.value ? "true" : "false");
                btn.disabled = o.disabled;
                btn.addEventListener("mousedown", (event) => {
                    event.preventDefault();
                    choose(o);
                });
                li.appendChild(btn);
                list.appendChild(li);
            });
        };
        const open = () => {
            if (select.disabled) {
                return;
            }
            closeAll(wrap);
            render();
            list.hidden = false;
            wrap.classList.add("is-open");
            input.setAttribute("aria-expanded", "true");
        };
        const close = (restore = true) => {
            list.hidden = true;
            wrap.classList.remove("is-open");
            input.setAttribute("aria-expanded", "false");
            query = "";
            // A search that was abandoned mid-typing restores
            // the committed selection; clearing a field is
            // done by picking its blank option, never by
            // half-typing and walking away.
            if (restore && select.value !== committedValue) {
                select.value = committedValue;
                notifyDependents();
            }
            syncInput();
        };
        wrap._comboboxClose = close;
        const choose = (o) => {
            select.value = o.value;
            committedValue = o.value;
            select.dispatchEvent(new Event("change", { bubbles: true }));
            query = "";
            syncInput();
            close(false);
            input.focus();
        };
        const refresh = () => {
            const parentValue = parentSelect ? parentSelect.value : "";
            const chosen = selectedOption();
            if (parentValue && chosen && chosen.parent !== parentValue) {
                select.value = "";
                select.dispatchEvent(new Event("change", { bubbles: true }));
            }
            committedValue = select.value;
            query = "";
            syncInput();
            if (!list.hidden) {
                render();
            }
        };
        if (parentSelect) {
            parentSelect.addEventListener("change", refresh);
            (parentSelect._comboboxDependents ||= []).push(refresh);
        }
        const notifyDependents = () => {
            (select._comboboxDependents || []).forEach((fn) => fn());
        };
        select.addEventListener("change", syncInput);
        input.addEventListener("focus", open);
        input.addEventListener("click", open);
        input.addEventListener("input", () => {
            if (select.value !== "") {
                select.value = "";
                notifyDependents();
            }
            query = input.value;
            open();
        });
        input.addEventListener("keydown", (event) => {
            const buttons = Array.from(list.querySelectorAll(".admin-combobox-option:not(:disabled)"));
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                if (list.hidden) {
                    open();
                }
                if (!buttons.length) {
                    return;
                }
                let index = buttons.indexOf(activeBtn);
                index = event.key === "ArrowDown"
                    ? Math.min(index + 1, buttons.length - 1)
                    : Math.max(index - 1, 0);
                if (activeBtn) {
                    activeBtn.classList.remove("is-active");
                }
                activeBtn = buttons[index];
                activeBtn.classList.add("is-active");
                activeBtn.scrollIntoView({ block: "nearest" });
            } else if (event.key === "Enter") {
                if (!list.hidden) {
                    const target = activeBtn || buttons[0];
                    if (target) {
                        event.preventDefault();
                        const found = options().find((o) => o.value === target.dataset.value);
                        if (found) {
                            choose(found);
                        }
                    }
                }
            } else if (event.key === "Escape") {
                if (!list.hidden) {
                    event.preventDefault();
                    close();
                }
            }
        });
        input.addEventListener("blur", () => {
            setTimeout(() => {
                if (!list.hidden) {
                    close();
                }
            }, 120);
        });
        if (toggle) {
            toggle.addEventListener("click", () => {
                if (list.hidden) {
                    input.focus();
                    open();
                } else {
                    close();
                }
            });
        }
        syncEnabled();
        syncInput();
    });
})();

// Coupon form conditional fields: the discount type owns
// which value field exists (fixed shows the amount, percent
// shows the rate, free shipping shows neither and pins the
// scope to the whole order), and the scope owns which
// target pickers exist. Hidden rows disable their controls
// so an irrelevant value can never ride along on submit;
// the server still validates the combination it receives.
(() => {
    document.querySelectorAll("[data-coupon-form]").forEach((form) => {
        const type = form.querySelector("select[name='discount_type']");
        const scope = form.querySelector("select[name='scope']");
        if (!type || !scope) {
            return;
        }
        const setRow = (el, show) => {
            el.hidden = !show;
            el.querySelectorAll("input, select, textarea").forEach((control) => {
                control.disabled = !show;
            });
        };
        const setBadge = (name, required) => {
            const badgeEl = form.querySelector(`[data-scope-badge="${name}"]`);
            if (!badgeEl) {
                return;
            }
            badgeEl.textContent = required ? "必須" : "任意";
            badgeEl.classList.toggle("badge-required", required);
            badgeEl.classList.toggle("badge-optional", !required);
        };
        const sync = () => {
            const typeValue = type.value;
            form.querySelectorAll("[data-coupon-value]").forEach((el) => {
                setRow(el, el.dataset.couponValue === typeValue);
            });
            if (typeValue === "free_shipping" && scope.value !== "order") {
                scope.value = "order";
            }
            scope.disabled = typeValue === "free_shipping";
            const scopeValue = scope.value;
            form.querySelectorAll("[data-coupon-scope]").forEach((el) => {
                setRow(el, el.dataset.couponScope.split(" ").includes(scopeValue));
            });
            setBadge("product", scopeValue === "product");
            setBadge("variant", scopeValue === "variant");
        };
        type.addEventListener("change", sync);
        scope.addEventListener("change", sync);
        sync();
    });
})();


// Order status form shipping fields: shown only while the
// chosen next status is the shipped one.
(() => {
    document.querySelectorAll("[data-order-status-form]").forEach((form) => {
        const status = form.querySelector("select[name='status']");
        if (!status) {
            return;
        }
        const sync = () => {
            const show = status.value === "shipped";
            form.querySelectorAll("[data-shipping-field]").forEach((el) => {
                el.hidden = !show;
                el.querySelectorAll("input, select, textarea").forEach((control) => {
                    control.disabled = !show;
                });
            });
        };
        status.addEventListener("change", sync);
        sync();
    });
})();

// Notification mail editor: clicking a tag chip inserts
// the tag at the body textarea's cursor, so operators
// never have to type tag names from memory. The input
// event feeds the form's unsaved-changes guard, like a
// keystroke would.
document.addEventListener("click", (event) => {
    const chip = event.target.closest("[data-tag-insert]");
    if (!chip) {
        return;
    }
    const area = document.querySelector("textarea[name=body]");
    if (!area) {
        return;
    }
    const tag = "{" + chip.getAttribute("data-tag-insert") + "}";
    const start = typeof area.selectionStart === "number" ? area.selectionStart : area.value.length;
    const end = typeof area.selectionEnd === "number" ? area.selectionEnd : area.value.length;
    area.setRangeText(tag, start, end, "end");
    area.focus();
    area.dispatchEvent(new Event("input", { bubbles: true }));
});

// Top page pin editors: the operator builds each section's
// pin list in the browser — add from the searchable picker,
// reorder, remove — and each mutation rewrites the section's
// one hidden field with the rows' ids, comma-joined in row
// order: that field is the form's pin contract. Every
// mutation also dispatches an
// input event from the list, feeding the form's
// unsaved-changes guard like a keystroke would.
(() => {
    const markDirty = (list) => {
        list.dispatchEvent(new Event("input", { bubbles: true }));
    };
    // Rewrites the section's hidden field from its rows, in
    // row order: the field is the form's only pin contract.
    const syncField = (editor) => {
        const field = editor.querySelector("[data-pin-field]");
        const ids = Array.from(editor.querySelectorAll("[data-pin-row]")).map(
            (row) => row.getAttribute("data-pin-id"),
        );
        field.value = ids.join(",");
    };
    const buildRow = (editor, id, label) => {
        const row = document.createElement("li");
        row.className = "pin-row";
        row.setAttribute("data-pin-row", "");
        row.setAttribute("data-pin-id", id);
        const name = document.createElement("span");
        name.className = "pin-name";
        name.textContent = label;
        const actions = document.createElement("span");
        actions.className = "pin-actions";
        const up = document.createElement("button");
        up.type = "button";
        up.className = "secondary-button";
        up.setAttribute("data-pin-up", "");
        up.setAttribute("aria-label", "上へ");
        up.textContent = "↑";
        const down = document.createElement("button");
        down.type = "button";
        down.className = "secondary-button";
        down.setAttribute("data-pin-down", "");
        down.setAttribute("aria-label", "下へ");
        down.textContent = "↓";
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "secondary-button";
        remove.setAttribute("data-pin-remove", "");
        remove.textContent = "外す";
        actions.append(up, down, remove);
        row.append(name, actions);
        return row;
    };
    document.addEventListener("click", (event) => {
        const add = event.target.closest("[data-pin-add]");
        if (add) {
            const editor = add.closest("[data-pin-editor]");
            const select = editor.querySelector("[data-combobox-select]");
            const list = editor.querySelector("[data-pin-list]");
            const id = select.value;
            if (!id) {
                return;
            }
            const cap = Number(editor.getAttribute("data-pin-cap") || "0");
            const rows = list.querySelectorAll("[data-pin-row]");
            const already = Array.from(rows).some(
                (row) => row.getAttribute("data-pin-id") === id,
            );
            if (already || (cap > 0 && rows.length >= cap)) {
                return;
            }
            const option = select.options[select.selectedIndex];
            list.appendChild(buildRow(editor, id, option ? option.textContent.trim() : id));
            select.value = "";
            select.dispatchEvent(new Event("change", { bubbles: true }));
            syncField(editor);
            markDirty(list);
            return;
        }
        const remove = event.target.closest("[data-pin-remove]");
        if (remove) {
            const row = remove.closest("[data-pin-row]");
            const list = row.closest("[data-pin-list]");
            const editor = row.closest("[data-pin-editor]");
            row.remove();
            syncField(editor);
            markDirty(list);
            return;
        }
        const up = event.target.closest("[data-pin-up]");
        if (up) {
            const row = up.closest("[data-pin-row]");
            const prev = row.previousElementSibling;
            if (prev) {
                row.parentElement.insertBefore(row, prev);
                syncField(row.closest("[data-pin-editor]"));
                markDirty(row.closest("[data-pin-list]"));
            }
            return;
        }
        const down = event.target.closest("[data-pin-down]");
        if (down) {
            const row = down.closest("[data-pin-row]");
            const next = row.nextElementSibling;
            if (next) {
                row.parentElement.insertBefore(next, row);
                syncField(row.closest("[data-pin-editor]"));
                markDirty(row.closest("[data-pin-list]"));
            }
        }
    });
})();

// Select-all for bulk checkbox groups (the order list's
// bulk-status selection): the master checkbox toggles every
// box of the named field inside its own form. Pure
// presentation — the POST contract is plain repeated fields
// and works with scripting off.
document.addEventListener("change", (event) => {
    const master = event.target.closest("[data-select-all]");
    if (!master || !master.form) {
        return;
    }
    const name = master.getAttribute("data-select-all");
    master.form
        .querySelectorAll('input[type="checkbox"][name="' + name + '"]')
        .forEach((box) => {
            box.checked = master.checked;
        });
});
