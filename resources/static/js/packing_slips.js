// The packing-slip page's only behaviour: the print button.
// The page is standalone (no admin chrome) so each printed
// sheet is one order's slip alone; print CSS hides the actions.
(function () {
    'use strict';
    var button = document.querySelector('[data-print]');
    if (button) {
        button.addEventListener('click', function () {
            window.print();
        });
    }
})();
