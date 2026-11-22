// Recently viewed products: the
// whole feature lives in the browser. A product page emits
// a record element carrying the facts it already rendered
// (id, name, displayed price, image) plus an empty mount;
// this module stores the record in localStorage (newest
// first, de-duplicated, capped) and renders the mounts on
// the home and product pages as a strip of small cards.
// No server round-trip and no Rust state: the strip is the
// visitor's own browsing history on this browser, so it
// survives nothing beyond localStorage and shows nothing
// the visitor has not already seen. The DOM is built with
// createElement and textContent only — never innerHTML.
(function () {
    'use strict';

    var KEY = 'recentlyViewedProducts';
    var MAX_ITEMS = 12;

    function load() {
        try {
            var parsed = JSON.parse(localStorage.getItem(KEY) || '[]');
            return Array.isArray(parsed) ? parsed : [];
        } catch (e) {
            return [];
        }
    }

    function save(items) {
        try {
            localStorage.setItem(KEY, JSON.stringify(items));
        } catch (e) {
            // Private mode / full storage: the feature just
            // quietly does not persist.
        }
    }

    // Record this page's product, if it carries a record.
    var record = document.querySelector('[data-recently-viewed-record]');
    var currentId = null;
    if (record) {
        currentId = record.getAttribute('data-id');
        var entry = {
            id: currentId,
            name: record.getAttribute('data-name') || '',
            price: record.getAttribute('data-price') || '',
            image: record.getAttribute('data-image') || ''
        };
        var rest = load().filter(function (item) {
            return item && item.id !== currentId;
        });
        rest.unshift(entry);
        save(rest.slice(0, MAX_ITEMS));
    }

    // Render every mount on the page from storage. The
    // product being viewed is excluded from its own strip.
    var mounts = document.querySelectorAll('[data-recently-viewed]');
    if (!mounts.length) {
        return;
    }
    var items = load().filter(function (item) {
        return item && item.id !== currentId && item.name;
    });
    if (!items.length) {
        return;
    }
    mounts.forEach(function (mount) {
        var row = mount.querySelector('[data-recently-viewed-row]');
        if (!row) {
            return;
        }
        items.forEach(function (item) {
            var card = document.createElement('a');
            card.className = 'recently-viewed-card';
            card.href = '/products/' + item.id;
            var img = document.createElement('img');
            img.src = item.image;
            img.alt = '';
            img.loading = 'lazy';
            var name = document.createElement('span');
            name.className = 'recently-viewed-name';
            name.textContent = item.name;
            var price = document.createElement('span');
            price.className = 'recently-viewed-price';
            price.textContent = item.price;
            card.appendChild(img);
            card.appendChild(name);
            card.appendChild(price);
            row.appendChild(card);
        });
        mount.hidden = false;
    });
})();
