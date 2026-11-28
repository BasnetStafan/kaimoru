// Breadcrumbs for the storefront (breadcrumbs are handled in the front end, links
// and paths included). Pages emit only an empty mount
// element — <nav class="breadcrumb" data-breadcrumb> — and
// this module owns the route table: it maps the current
// pathname to a trail, and renders it into the mount.
//
// Matching is segment-exact: ids must be numeric where an id
// belongs, a trailing slash is normalised away, and a path
// with no table entry renders NOTHING rather than a guessed
// trail. The DOM is built with createElement and textContent
// only — never innerHTML — and reuses the shared
// .breadcrumb / .crumb-sep styles; the last crumb is plain
// text marked aria-current="page".
//
// The one trail no route table can build is the product
// detail's: トップページ › {category} › {product}. Its two
// names live in the page itself, so for /products/{id} the
// category label (and link, while the category is active)
// is read from the spec table's カテゴリー row — the same
// link the page already exposes — and the product name from
// the page's <h1>.
//
// The filtered search listing is the same kind: /search is
// one path for every filter state, so its trail — トップ
// ページ › {category} (› {tag}) — is read from the page too:
// the category from the filter form's select (its selected
// option is the applied category) and the tag from its
// filter chip, the one place the page prints the tag's
// label. Unfiltered and keyword-only listings build no
// trail, exactly as before this entry existed.

(function () {
    "use strict";

    var TOP = ["トップページ", "/"];
    var MYPAGE = ["マイページ", "/mypage"];

    function link(label, href) {
        return [label, href];
    }

    function current(label) {
        return [label, null];
    }

    function isId(segment) {
        return /^[0-9]+$/.test(segment);
    }

    // The mypage trail shared by every subpage:
    // トップページ › マイページ › ...rest.
    function mypageTrail(rest) {
        return [TOP, MYPAGE].concat(rest);
    }

    // Resolves a segment list to a trail, or null when the
    // path is not in the table.
    function trailFor(segments) {
        if (segments[0] === "mypage") {
            var rest = segments.slice(1);
            if (rest.length === 0) {
                return [TOP, current("マイページ")];
            }
            if (rest.length === 1) {
                switch (rest[0]) {
                    case "edit":
                        return mypageTrail([current("会員情報の編集")]);
                    case "email":
                        return mypageTrail([current("メールアドレス変更")]);
                    case "password":
                        return mypageTrail([current("パスワード変更")]);
                    case "totp":
                        return mypageTrail([current("2段階認証設定")]);
                    case "sns":
                        return mypageTrail([current("SNS連携")]);
                    case "orders":
                        return mypageTrail([current("注文履歴")]);
                    case "addresses":
                        return mypageTrail([current("アドレス帳")]);
                    case "coupons":
                        return mypageTrail([current("クーポン一覧")]);
                    case "favorites":
                        return mypageTrail([current("お気に入り商品")]);
                    case "withdraw":
                        return mypageTrail([current("退会手続き")]);
                    default:
                        return null;
                }
            }
            if (rest.length === 2 && rest[0] === "email" && rest[1] === "verify") {
                return mypageTrail([
                    link("メールアドレス変更", "/mypage/email"),
                    current("認証コードの入力"),
                ]);
            }
            if (rest.length === 2 && rest[0] === "orders" && isId(rest[1])) {
                return mypageTrail([
                    link("注文履歴", "/mypage/orders"),
                    current("注文詳細"),
                ]);
            }
            if (rest.length === 2 && rest[0] === "addresses" && rest[1] === "new") {
                return mypageTrail([
                    link("アドレス帳", "/mypage/addresses"),
                    current("新しいお届け先"),
                ]);
            }
            if (rest.length === 2 && rest[0] === "totp" && rest[1] === "enable") {
                // The recovery-codes view answers the enable
                // POST, so its URL is the POST endpoint.
                return mypageTrail([
                    link("2段階認証設定", "/mypage/totp"),
                    current("リカバリーコード"),
                ]);
            }
            if (rest.length === 3 && rest[0] === "orders" && isId(rest[1]) && rest[2] === "cancel") {
                return mypageTrail([
                    link("注文履歴", "/mypage/orders"),
                    link("注文詳細", "/mypage/orders/" + rest[1]),
                    current("注文のキャンセル"),
                ]);
            }
            if (rest.length === 3 && rest[0] === "addresses" && isId(rest[1])) {
                if (rest[2] === "edit") {
                    return mypageTrail([
                        link("アドレス帳", "/mypage/addresses"),
                        current("お届け先の編集"),
                    ]);
                }
                if (rest[2] === "delete") {
                    return mypageTrail([
                        link("アドレス帳", "/mypage/addresses"),
                        current("お届け先の削除"),
                    ]);
                }
            }
            if (
                rest.length === 3 &&
                rest[0] === "totp" &&
                rest[1] === "recovery" &&
                rest[2] === "regenerate"
            ) {
                // The regenerated codes view answers this POST.
                return mypageTrail([
                    link("2段階認証設定", "/mypage/totp"),
                    current("リカバリーコード"),
                ]);
            }
            return null;
        }
        if (segments[0] === "products" && segments.length === 2 && isId(segments[1])) {
            return productTrail();
        }
        if (segments[0] === "search" && segments.length === 1) {
            return searchTrail();
        }
        return null;
    }

    // The product detail trail, read from the page: the
    // category name/link from the spec table's カテゴリー row,
    // the product name from the <h1>. A value the page does
    // not carry is simply left out of the trail (never
    // guessed): no category row renders トップページ › the
    // product, and no product heading renders トップページ
    // alone.
    function productTrail() {
        var heading = document.querySelector(".product-detail h1");
        var trail = [TOP];
        if (!heading) {
            return trail;
        }
        var headers = document.querySelectorAll("th");
        for (var i = 0; i < headers.length; i++) {
            if (headers[i].textContent.trim() === "カテゴリー") {
                var cell = headers[i].nextElementSibling;
                if (!cell) {
                    break;
                }
                var anchor = cell.querySelector("a");
                if (anchor) {
                    trail.push(
                        link(anchor.textContent.trim(), anchor.getAttribute("href"))
                    );
                } else {
                    trail.push(current(cell.textContent.trim()));
                }
                break;
            }
        }
        trail.push(current(heading.textContent.trim()));
        return trail;
    }

    // The filtered listing's trail, read from the page (see
    // the header comment): トップページ plus the applied
    // category and tag as plain current crumbs — the listing
    // itself is the page being read, so neither name is a
    // link. Returns null when neither filter is applied.
    function searchTrail() {
        var trail = [TOP];
        var select = document.querySelector(
            ".search-tools select[name='category']"
        );
        if (select && select.value) {
            var option = select.options[select.selectedIndex];
            var name = option ? option.textContent.trim() : select.value;
            if (name) {
                trail.push(current(name));
            }
        }
        // The tag name rides in the page heading
        // (タグ「…」の商品); the filter chips that used to
        // carry it on every filtered view were removed, so
        // where the heading does not name the tag, the trail
        // simply leaves it out.
        var heading = document.querySelector(".search-page h1");
        if (heading) {
            var tagMatch = heading.textContent.match(/タグ「([^」]+)」/);
            if (tagMatch) {
                trail.push(current(tagMatch[1].trim()));
            }
        }
        return trail.length > 1 ? trail : null;
    }

    function render(nav, trail) {
        trail.forEach(function (crumb, index) {
            if (index > 0) {
                var sep = document.createElement("span");
                sep.className = "crumb-sep";
                sep.textContent = "›";
                nav.appendChild(document.createTextNode(" "));
                nav.appendChild(sep);
                nav.appendChild(document.createTextNode(" "));
            }
            if (crumb[1]) {
                var anchor = document.createElement("a");
                anchor.href = crumb[1];
                anchor.textContent = crumb[0];
                nav.appendChild(anchor);
            } else {
                var span = document.createElement("span");
                if (index === trail.length - 1) {
                    span.setAttribute("aria-current", "page");
                }
                span.textContent = crumb[0];
                nav.appendChild(span);
            }
        });
    }

    document.addEventListener("DOMContentLoaded", function () {
        var nav = document.querySelector("nav.breadcrumb[data-breadcrumb]");
        if (!nav) {
            return;
        }
        var path = window.location.pathname;
        if (path.length > 1 && path.endsWith("/")) {
            path = path.slice(0, -1);
        }
        var segments = path.split("/").filter(function (part) {
            return part !== "";
        });
        var trail = trailFor(segments);
        if (trail) {
            render(nav, trail);
        }
    });
})();
